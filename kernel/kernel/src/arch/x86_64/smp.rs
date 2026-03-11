//! SMP (Symmetric Multiprocessing) support.
//!
//! Implements AP (Application Processor) startup via INIT-SIPI-SIPI.
//! The BSP (Bootstrap Processor) sends INIT and SIPI IPIs to each AP
//! found in the MADT. APs start in real mode, transition to long mode
//! using shared page tables, then enter the kernel executor loop.
//!
//! Boot protocol:
//! 1. BSP collects AP LAPIC IDs from MADT
//! 2. BSP copies real-mode trampoline to physical page 0x8000
//! 3. BSP writes kernel CR3 and entry point into trampoline data area
//! 4. For each AP: INIT IPI → 10 ms delay → SIPI → 200 µs → SIPI
//! 5. APs wake in real mode at 0x8000, transition 16→32→64-bit
//! 6. APs read per-CPU data, init GDT/IDT/LAPIC/SYSCALL, enter executor

extern crate alloc;

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use hadron_core::addr::VirtAddr;

use crate::arch::x86_64::hw::local_apic::LocalApic;
use crate::percpu::MAX_CPUS;

/// Physical address of the AP trampoline code (must be page-aligned, < 1 MiB).
const AP_TRAMPOLINE_PHYS: u64 = 0x8000;

/// Maximum number of CPUs supported.
const MAX_AP_COUNT: usize = MAX_CPUS - 1;

// ── Shared AP startup state ─────────────────────────────────────────

/// Kernel CR3 for APs to load during trampoline.
static AP_KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// Number of APs that have reached the parking loop.
static AP_STARTED_COUNT: AtomicU32 = AtomicU32::new(0);

/// Release flag: when true, parked APs proceed to full initialization.
static AP_RELEASE: AtomicBool = AtomicBool::new(false);

/// Per-AP `PerCpuState` pointers, indexed by logical CPU ID (1..N).
static AP_PERCPU_TABLE: [AtomicU64; MAX_CPUS] = [const { AtomicU64::new(0) }; MAX_CPUS];

/// Total CPU count (BSP + APs), set during boot_aps.
static CPU_COUNT: AtomicU32 = AtomicU32::new(1);

/// LAPIC ID → logical CPU ID mapping.
static LAPIC_TO_CPU: [AtomicU32; 256] = [const { AtomicU32::new(u32::MAX) }; 256];

// ── AP entry point (called from trampoline in long mode) ────────────

/// Entry point for APs after they reach long mode via the trampoline.
///
/// Called with:
/// - Interrupts disabled
/// - Correct CR3 loaded (kernel page tables)
/// - A temporary stack from the trampoline
/// - LAPIC ID in the first argument (read by trampoline from CPUID)
///
/// # Safety
///
/// Called only from the AP trampoline code. Must not be called from Rust.
#[unsafe(no_mangle)]
#[cfg(hadron_smp)]
unsafe extern "C" fn ap_entry(lapic_id: u64) {
    let lapic_id = lapic_id as u8;

    // Look up our logical CPU ID.
    let cpu_id = LAPIC_TO_CPU[lapic_id as usize].load(Ordering::Acquire);
    if cpu_id == u32::MAX {
        // Unknown AP — spin forever.
        loop {
            core::hint::spin_loop();
        }
    }

    // Signal that we've started.
    AP_STARTED_COUNT.fetch_add(1, Ordering::Release);

    // Spin until BSP releases us (per-CPU data is ready).
    while !AP_RELEASE.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }

    // Read our PerCpuState pointer.
    let percpu_ptr = AP_PERCPU_TABLE[cpu_id as usize].load(Ordering::Acquire);
    if percpu_ptr == 0 {
        loop {
            core::hint::spin_loop();
        }
    }
    let percpu = percpu_ptr as *mut crate::percpu::PerCpuState;

    // Initialize GDT + TSS (allocates kernel + double-fault stacks).
    let cpu_id_typed = crate::id::CpuId::new(cpu_id);
    // SAFETY: GDT init_ap is safe to call once per AP after heap is ready.
    let kernel_rsp = unsafe { crate::arch::x86_64::gdt::init_ap(cpu_id_typed) };

    // Update PerCpu with the real kernel RSP from GDT init.
    // SAFETY: We own this PerCpu, no one else accesses it yet.
    unsafe {
        (*percpu).kernel_rsp = kernel_rsp;
    }

    // Load IDT (shared with BSP).
    // SAFETY: IDT is initialized by BSP and is static.
    unsafe {
        crate::arch::x86_64::idt::init();
    }

    // Set GS base to our PerCpu.
    // SAFETY: Valid PerCpu pointer, correct MSR.
    unsafe {
        crate::arch::x86_64::registers::model_specific::IA32_GS_BASE.write(percpu_ptr);
        crate::arch::x86_64::registers::model_specific::IA32_KERNEL_GS_BASE.write(percpu_ptr);
        (*percpu).initialized = 1;
    }

    // Initialize SYSCALL/SYSRET MSRs.
    // SAFETY: Same setup as BSP, valid MSR writes.
    unsafe {
        crate::arch::x86_64::syscall::init();
    }

    // Initialize LAPIC and start periodic timer.
    if let Some(lapic_virt) = crate::arch::x86_64::acpi::Acpi::lapic_virt() {
        // SAFETY: LAPIC MMIO is mapped and valid.
        let lapic = unsafe { LocalApic::new(lapic_virt) };

        use crate::arch::x86_64::interrupts::dispatch::vectors;
        lapic.enable(vectors::SPURIOUS.as_irq_vector());
        lapic.set_tpr(0);

        let (initial_count, divide) = crate::arch::x86_64::acpi::Acpi::lapic_timer_config();
        if initial_count > 0 {
            lapic.start_timer_periodic(vectors::TIMER.as_irq_vector(), initial_count, divide);
        }
    }

    crate::kinfo!(
        "smp",
        "AP {} (LAPIC {}) initialized, entering executor",
        cpu_id,
        lapic_id
    );

    // Enable interrupts and enter the executor loop.
    // SAFETY: All per-CPU state is initialized.
    unsafe { core::arch::asm!("sti") };

    let halt = crate::entry::HltHalt;
    let steal = crate::entry::make_steal_fn();
    hadron_sched::executor().run(&halt, steal);
}

// ── BSP-side AP startup ─────────────────────────────────────────────

/// Collected AP information from the MADT.
struct ApInfo {
    lapic_id: u8,
    cpu_id: u32,
}

/// Boot all APs found in the MADT.
///
/// Called from `kernel_init()` after ACPI, PMM, heap, and per-CPU BSP setup.
///
/// # Safety
///
/// Must be called exactly once from the BSP.
#[cfg(hadron_smp)]
pub unsafe fn boot_aps() {
    use hadron_acpi::madt;

    // Get LAPIC virtual address for sending IPIs.
    let lapic_virt = match crate::arch::x86_64::acpi::Acpi::lapic_virt() {
        Some(v) => v,
        None => {
            crate::kwarn!("smp", "SMP: No LAPIC, skipping AP startup");
            return;
        }
    };
    // SAFETY: LAPIC is mapped.
    let lapic = unsafe { LocalApic::new(lapic_virt) };
    let bsp_lapic_id = lapic.id();

    // Collect AP LAPIC IDs from MADT.
    let aps = collect_ap_info(bsp_lapic_id);
    if aps.is_empty() {
        crate::kinfo!("smp", "SMP: No APs found in MADT (single-CPU system)");
        return;
    }

    let ap_count = aps.len();
    crate::kinfo!(
        "smp",
        "SMP: Found {} APs, starting INIT-SIPI-SIPI",
        ap_count
    );

    // Store kernel CR3 for trampoline.
    let cr3: u64;
    // SAFETY: Reading CR3 is always safe in ring 0.
    unsafe { core::arch::asm!("mov {}, cr3", out(reg) cr3) };
    AP_KERNEL_CR3.store(cr3, Ordering::Release);

    // Set up LAPIC-to-CPU mapping.
    LAPIC_TO_CPU[bsp_lapic_id as usize].store(0, Ordering::Release);
    for ap in &aps {
        LAPIC_TO_CPU[ap.lapic_id as usize].store(ap.cpu_id, Ordering::Release);
    }

    // Register IPI handlers before APs start (they need the wakeup IPI).
    init_ipi_handlers();

    // Copy trampoline to low memory and set up data area.
    // SAFETY: Physical page 0x8000 is available (below 1 MiB, not used by kernel).
    unsafe {
        setup_trampoline(cr3);
    }

    // Send INIT-SIPI-SIPI to each AP.
    for ap in &aps {
        // SAFETY: Sending IPIs to valid LAPIC IDs.
        unsafe {
            send_init_sipi_sipi(&lapic, ap.lapic_id);
        }
    }

    // Wait for all APs to reach their parking loop.
    let deadline = crate::time::nanos_since_boot() + 500_000_000; // 500 ms timeout
    while AP_STARTED_COUNT.load(Ordering::Acquire) < ap_count as u32 {
        if crate::time::nanos_since_boot() > deadline {
            crate::kerror!(
                "smp",
                "SMP: Timeout waiting for APs ({}/{} started)",
                AP_STARTED_COUNT.load(Ordering::Acquire),
                ap_count
            );
            break;
        }
        core::hint::spin_loop();
    }

    let started = AP_STARTED_COUNT.load(Ordering::Acquire);
    crate::kinfo!(
        "smp",
        "SMP: {}/{} APs reached parking loop",
        started,
        ap_count
    );

    // Allocate per-CPU state for each AP.
    for ap in &aps {
        if ap.cpu_id as usize >= MAX_CPUS {
            continue;
        }
        let percpu = crate::percpu::init_ap_percpu(ap.cpu_id, 0);
        AP_PERCPU_TABLE[ap.cpu_id as usize].store(percpu as u64, Ordering::Release);
    }

    // Update total CPU count.
    CPU_COUNT.store(1 + started, Ordering::Release);

    // Release all APs to proceed with full initialization.
    AP_RELEASE.store(true, Ordering::Release);

    // Wait briefly for APs to enter executor loops.
    let deadline = crate::time::nanos_since_boot() + 1_000_000_000; // 1s timeout
    // Give APs time to initialize — they'll log when ready.
    while crate::time::nanos_since_boot() < deadline {
        // Check if all started APs have initialized by checking if they've
        // set their percpu.initialized flag.
        let mut all_init = true;
        for ap in &aps {
            let percpu_ptr = AP_PERCPU_TABLE[ap.cpu_id as usize].load(Ordering::Acquire);
            if percpu_ptr != 0 {
                // SAFETY: We allocated this PerCpu and the AP writes to it.
                let percpu = unsafe { &*(percpu_ptr as *const crate::percpu::PerCpuState) };
                if percpu.initialized == 0 {
                    all_init = false;
                    break;
                }
            }
        }
        if all_init {
            break;
        }
        core::hint::spin_loop();
    }

    crate::kinfo!("smp", "SMP: {} CPUs online", cpu_count());
}

/// Returns the total number of online CPUs.
pub fn cpu_count() -> u32 {
    CPU_COUNT.load(Ordering::Acquire)
}

/// Collect AP information from the MADT.
#[cfg(hadron_smp)]
fn collect_ap_info(bsp_lapic_id: u8) -> Vec<ApInfo> {
    use hadron_acpi::madt;

    let mut aps = Vec::new();
    let mut next_cpu_id = 1u32;

    crate::arch::x86_64::acpi::Acpi::with_madt(|madt_data| {
        for entry in madt_data.entries() {
            if let madt::MadtEntry::LocalApic { apic_id, flags, .. } = entry {
                // Skip disabled CPUs and the BSP.
                if flags & 1 == 0 || apic_id == bsp_lapic_id {
                    continue;
                }
                if aps.len() >= MAX_AP_COUNT {
                    break;
                }
                aps.push(ApInfo {
                    lapic_id: apic_id,
                    cpu_id: next_cpu_id,
                });
                next_cpu_id += 1;
            }
        }
    });

    aps
}

// ── INIT-SIPI-SIPI protocol ────────────────────────────────────────

/// ICR delivery modes for INIT-SIPI-SIPI.
const ICR_INIT: u32 = 0b101 << 8; // INIT
const ICR_SIPI: u32 = 0b110 << 8; // Startup IPI
const ICR_ASSERT: u32 = 1 << 14; // Assert level
const ICR_LEVEL: u32 = 1 << 15; // Level-triggered

/// Send INIT-SIPI-SIPI to a single AP.
///
/// The AP will start executing at physical address `AP_TRAMPOLINE_PHYS`.
///
/// # Safety
///
/// The trampoline must be set up at `AP_TRAMPOLINE_PHYS` before calling.
#[cfg(hadron_smp)]
unsafe fn send_init_sipi_sipi(lapic: &LocalApic, target_apic_id: u8) {
    let sipi_vector = (AP_TRAMPOLINE_PHYS >> 12) as u32;

    // INIT IPI (assert).
    // SAFETY: Writing to LAPIC ICR registers.
    unsafe {
        lapic.write_icr(target_apic_id, ICR_INIT | ICR_ASSERT | ICR_LEVEL);
    }

    // Wait 10 ms for INIT to take effect.
    busy_wait_us(10_000);

    // INIT IPI (de-assert) — not needed on modern CPUs but safe to send.
    // SAFETY: Writing to LAPIC ICR registers.
    unsafe {
        lapic.write_icr(target_apic_id, ICR_INIT | ICR_LEVEL);
    }

    busy_wait_us(200);

    // First SIPI.
    // SAFETY: Writing to LAPIC ICR registers.
    unsafe {
        lapic.write_icr(target_apic_id, ICR_SIPI | sipi_vector);
    }

    busy_wait_us(200);

    // Second SIPI (in case the first was missed).
    // SAFETY: Writing to LAPIC ICR registers.
    unsafe {
        lapic.write_icr(target_apic_id, ICR_SIPI | sipi_vector);
    }

    busy_wait_us(200);
}

/// Busy-wait for approximately `us` microseconds using TSC.
fn busy_wait_us(us: u64) {
    let nanos = us * 1_000;
    let start = crate::time::nanos_since_boot();
    while crate::time::nanos_since_boot() - start < nanos {
        core::hint::spin_loop();
    }
}

// ── Trampoline setup ───────────────────────────────────────────────

/// Data area offsets within the trampoline page (at fixed addresses).
/// These are read by the real-mode trampoline code.
const TRAMPOLINE_DATA_OFFSET: usize = 0xF00; // Last 256 bytes of page
const TRAMPOLINE_CR3_OFFSET: usize = TRAMPOLINE_DATA_OFFSET;
const TRAMPOLINE_ENTRY_OFFSET: usize = TRAMPOLINE_DATA_OFFSET + 8;
const TRAMPOLINE_STACK_OFFSET: usize = TRAMPOLINE_DATA_OFFSET + 16;

/// Set up the AP trampoline at physical address 0x8000.
///
/// The trampoline code transitions APs from real mode → protected mode →
/// long mode, then jumps to `ap_entry`.
///
/// # Safety
///
/// Physical page at `AP_TRAMPOLINE_PHYS` must be mapped and available.
#[cfg(hadron_smp)]
unsafe fn setup_trampoline(kernel_cr3: u64) {
    // Map the trampoline physical page into kernel virtual space via HHDM.
    let hhdm_offset = hadron_mm::hhdm::offset();
    let trampoline_virt = (AP_TRAMPOLINE_PHYS + hhdm_offset.as_u64()) as *mut u8;

    // Write the trampoline machine code.
    let trampoline_code = build_trampoline();
    // SAFETY: We have exclusive access to this physical page.
    unsafe {
        core::ptr::copy_nonoverlapping(
            trampoline_code.as_ptr(),
            trampoline_virt,
            trampoline_code.len(),
        );
    }

    // Write data area: CR3, entry point, temporary stack.
    // SAFETY: Offset is within the trampoline page we own.
    let data_base = unsafe { trampoline_virt.add(TRAMPOLINE_DATA_OFFSET) };
    // SAFETY: Writing to our data area within the trampoline page.
    unsafe {
        // CR3 for long mode.
        (data_base as *mut u64).write(kernel_cr3);
        // Entry point (ap_entry function).
        (data_base.add(8) as *mut u64).write(ap_entry as u64);
        // Temporary stack (top of trampoline page — grows down).
        // APs will get their real stack from GDT init_ap later.
        (data_base.add(16) as *mut u64).write(AP_TRAMPOLINE_PHYS + 0xF00);
    }
}

/// Build the AP trampoline machine code.
///
/// The trampoline executes at physical address 0x8000 and transitions from
/// 16-bit real mode to 64-bit long mode:
///
/// 1. Real mode (16-bit): Set up segments, enable A20 (fast method)
/// 2. Protected mode (32-bit): Load temporary GDT, enable PE in CR0
/// 3. Long mode (64-bit): Enable PAE+PML4, set EFER.LME, load CR3, enable PG
/// 4. Jump to `ap_entry` with LAPIC ID as argument
#[cfg(hadron_smp)]
fn build_trampoline() -> Vec<u8> {
    let mut code = Vec::with_capacity(512);

    // The trampoline is position-dependent at 0x8000.
    // All addresses are relative to this base.
    let base = AP_TRAMPOLINE_PHYS as u32;
    let data_base = base + TRAMPOLINE_DATA_OFFSET as u32;
    let gdt_offset = 0x100u32; // GDT at 0x8100
    let gdt64_offset = 0x140u32; // 64-bit GDT at 0x8140

    // ── 16-bit real mode entry ──────────────────────────────
    // APs start here after SIPI (CS:IP = 0x0800:0x0000 = phys 0x8000)

    // cli
    code.push(0xFA);
    // cld
    code.push(0xFC);

    // xor ax, ax
    code.extend_from_slice(&[0x31, 0xC0]);
    // mov ds, ax
    code.extend_from_slice(&[0x8E, 0xD8]);
    // mov es, ax
    code.extend_from_slice(&[0x8E, 0xC0]);
    // mov ss, ax
    code.extend_from_slice(&[0x8E, 0xD0]);

    // Enable A20 (fast method via port 0x92)
    // in al, 0x92
    code.extend_from_slice(&[0xE4, 0x92]);
    // or al, 2
    code.extend_from_slice(&[0x0C, 0x02]);
    // and al, 0xFE  (don't reset)
    code.extend_from_slice(&[0x24, 0xFE]);
    // out 0x92, al
    code.extend_from_slice(&[0xE6, 0x92]);

    // lgdt [gdt_desc] (at base + gdt_offset + 0x30)
    // Address prefix override for 32-bit address in 16-bit mode
    code.push(0x67); // address size override
    code.extend_from_slice(&[0x0F, 0x01, 0x15]); // lgdt [disp32]
    code.extend_from_slice(&(base + gdt_offset + 0x30).to_le_bytes()); // GDT descriptor address

    // Enable protected mode: mov eax, cr0; or eax, 1; mov cr0, eax
    code.extend_from_slice(&[0x0F, 0x20, 0xC0]); // mov eax, cr0
    code.extend_from_slice(&[0x66, 0x83, 0xC8, 0x01]); // or eax, 1
    code.extend_from_slice(&[0x0F, 0x22, 0xC0]); // mov cr0, eax

    // Far jump to 32-bit protected mode code
    // jmp 0x08:pm_entry (use 66 prefix for 32-bit offset in 16-bit mode)
    let pm_entry_offset = 0x60u32; // 32-bit code starts at 0x8060
    code.extend_from_slice(&[0x66, 0xEA]); // far jmp with 32-bit offset
    code.extend_from_slice(&(base + pm_entry_offset).to_le_bytes()); // offset
    code.extend_from_slice(&[0x08, 0x00]); // segment selector (code32)

    // Pad to 0x60 (pm_entry)
    while code.len() < pm_entry_offset as usize {
        code.push(0x90); // nop
    }

    // ── 32-bit protected mode ──────────────────────────────
    // .code32 equivalent — all instructions from here are 32-bit

    // mov ax, 0x10; mov ds, ax; mov es, ax; mov ss, ax
    code.extend_from_slice(&[0x66, 0xB8, 0x10, 0x00]); // mov ax, 0x10
    code.extend_from_slice(&[0x8E, 0xD8]); // mov ds, ax
    code.extend_from_slice(&[0x8E, 0xC0]); // mov es, ax
    code.extend_from_slice(&[0x8E, 0xD0]); // mov ss, ax
    code.extend_from_slice(&[0x8E, 0xE0]); // mov fs, ax
    code.extend_from_slice(&[0x8E, 0xE8]); // mov gs, ax

    // Enable PAE (CR4.PAE = bit 5)
    code.extend_from_slice(&[0x0F, 0x20, 0xE0]); // mov eax, cr4
    code.extend_from_slice(&[0x0F, 0xBA, 0xE8, 0x05]); // bts eax, 5
    code.extend_from_slice(&[0x0F, 0x22, 0xE0]); // mov cr4, eax

    // Load CR3 from data area
    code.extend_from_slice(&[0xB8]); // mov eax, imm32
    code.extend_from_slice(&data_base.to_le_bytes()); // address of CR3 in data area
    code.extend_from_slice(&[0x8B, 0x00]); // mov eax, [eax]
    code.extend_from_slice(&[0x0F, 0x22, 0xD8]); // mov cr3, eax

    // Enable long mode (EFER.LME = bit 8)
    code.extend_from_slice(&[0xB9, 0x80, 0x00, 0x00, 0xC0]); // mov ecx, 0xC0000080 (IA32_EFER)
    code.extend_from_slice(&[0x0F, 0x32]); // rdmsr
    code.extend_from_slice(&[0x0F, 0xBA, 0xE8, 0x08]); // bts eax, 8 (LME)
    code.extend_from_slice(&[0x0F, 0xBA, 0xE8, 0x0B]); // bts eax, 11 (NXE)
    code.extend_from_slice(&[0x0F, 0x30]); // wrmsr

    // Enable paging (CR0.PG = bit 31)
    code.extend_from_slice(&[0x0F, 0x20, 0xC0]); // mov eax, cr0
    code.extend_from_slice(&[0x0F, 0xBA, 0xE8, 0x1F]); // bts eax, 31
    code.extend_from_slice(&[0x0F, 0x22, 0xC0]); // mov cr0, eax

    // Load 64-bit GDT
    code.extend_from_slice(&[0x0F, 0x01, 0x15]); // lgdt [disp32]
    code.extend_from_slice(&(base + gdt64_offset + 0x30).to_le_bytes());

    // Far jump to 64-bit long mode
    let lm_entry_offset = 0xC0u32; // 64-bit code at 0x80C0
    code.extend_from_slice(&[0xEA]); // far jmp
    code.extend_from_slice(&(base + lm_entry_offset).to_le_bytes());
    code.extend_from_slice(&[0x08, 0x00]); // code64 selector

    // Pad to 0x100 (GDT area)
    while code.len() < gdt_offset as usize {
        // Pad with NOPs
        code.push(0x90);
    }

    // ── 32-bit GDT (at 0x8100) ─────────────────────────────
    // Null, Code32, Data32
    // Entry 0: Null
    code.extend_from_slice(&[0; 8]);
    // Entry 1 (0x08): Code32 — base=0, limit=0xFFFFF, 32-bit, executable
    code.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00, 0x00, 0x9A, 0xCF, 0x00]);
    // Entry 2 (0x10): Data32 — base=0, limit=0xFFFFF, 32-bit, writable
    code.extend_from_slice(&[0xFF, 0xFF, 0x00, 0x00, 0x00, 0x92, 0xCF, 0x00]);
    // Pad to 0x30 relative to GDT start for descriptor
    while code.len() < (gdt_offset + 0x30) as usize {
        code.push(0);
    }
    // GDT descriptor (limit + base)
    let gdt_limit: u16 = 3 * 8 - 1; // 3 entries
    code.extend_from_slice(&gdt_limit.to_le_bytes());
    code.extend_from_slice(&(base + gdt_offset).to_le_bytes());
    // Pad for alignment
    while code.len() < gdt64_offset as usize {
        code.push(0);
    }

    // ── 64-bit GDT (at 0x8140) ─────────────────────────────
    // Null, Code64, Data64
    // Entry 0: Null
    code.extend_from_slice(&[0; 8]);
    // Entry 1 (0x08): Code64 — long mode, executable
    code.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x9A, 0x20, 0x00]);
    // Entry 2 (0x10): Data64 — writable
    code.extend_from_slice(&[0x00, 0x00, 0x00, 0x00, 0x00, 0x92, 0x00, 0x00]);
    // Pad to 0x30 relative to GDT64 start for descriptor
    while code.len() < (gdt64_offset + 0x30) as usize {
        code.push(0);
    }
    // GDT64 descriptor
    let gdt64_limit: u16 = 3 * 8 - 1;
    code.extend_from_slice(&gdt64_limit.to_le_bytes());
    // 64-bit base address (8 bytes for 64-bit LGDT)
    code.extend_from_slice(&(base as u64 + gdt64_offset as u64).to_le_bytes());

    // Pad to 0xC0 (lm_entry)
    while code.len() < lm_entry_offset as usize {
        code.push(0x90);
    }

    // ── 64-bit long mode code (at 0x80C0) ──────────────────

    // mov ax, 0x10; mov ds, ax; mov es, ax; mov ss, ax; xor ax,ax; mov fs,ax; mov gs,ax
    // Use REX.W prefix for 64-bit segment loads
    code.extend_from_slice(&[0x66, 0xB8, 0x10, 0x00]); // mov ax, 0x10
    code.extend_from_slice(&[0x8E, 0xD8]); // mov ds, ax
    code.extend_from_slice(&[0x8E, 0xC0]); // mov es, ax
    code.extend_from_slice(&[0x8E, 0xD0]); // mov ss, ax
    code.extend_from_slice(&[0x66, 0x31, 0xC0]); // xor ax, ax
    code.extend_from_slice(&[0x8E, 0xE0]); // mov fs, ax
    code.extend_from_slice(&[0x8E, 0xE8]); // mov gs, ax

    // Load temporary stack from data area
    // mov rsp, [data_base + 16]
    code.extend_from_slice(&[0x48, 0xBC]); // mov rsp, imm64  -- actually movabs
    code.extend_from_slice(&(AP_TRAMPOLINE_PHYS + TRAMPOLINE_STACK_OFFSET as u64).to_le_bytes());
    // Now dereference: mov rsp, [rsp]
    code.extend_from_slice(&[0x48, 0x8B, 0x24, 0x24]); // mov rsp, [rsp]

    // Get LAPIC ID from CPUID
    // mov eax, 1
    code.extend_from_slice(&[0xB8, 0x01, 0x00, 0x00, 0x00]);
    // cpuid
    code.extend_from_slice(&[0x0F, 0xA2]);
    // shr ebx, 24 — LAPIC ID is in EBX[31:24]
    code.extend_from_slice(&[0xC1, 0xEB, 0x18]);
    // movzx rdi, bl — first argument to ap_entry
    code.extend_from_slice(&[0x48, 0x0F, 0xB6, 0xFB]);

    // Load ap_entry address from data area and call it
    // mov rax, [data_base + 8]
    code.extend_from_slice(&[0x48, 0xB8]); // movabs rax, imm64
    code.extend_from_slice(&(AP_TRAMPOLINE_PHYS + TRAMPOLINE_ENTRY_OFFSET as u64).to_le_bytes());
    code.extend_from_slice(&[0x48, 0x8B, 0x00]); // mov rax, [rax]
    // jmp rax
    code.extend_from_slice(&[0xFF, 0xE0]);

    code
}

// ── IPI framework ──────────────────────────────────────────────────

/// Initialize IPI handlers and register the cross-CPU wake function.
///
/// Called from `boot_aps()` after AP enumeration but before releasing APs.
#[cfg(hadron_smp)]
pub(crate) fn init_ipi_handlers() {
    use crate::arch::x86_64::interrupts::dispatch::{register_handler, vectors};
    use crate::id::IrqVector;

    // Wakeup IPI: just EOI — the return from ISR resumes the executor poll loop.
    let _ = register_handler(vectors::WAKEUP_IPI, wakeup_ipi_handler);

    // TLB shootdown IPI: process pending shootdown requests.
    let _ = register_handler(vectors::TLB_SHOOTDOWN_IPI, tlb_shootdown_ipi_handler);

    // Reschedule IPI: hint to check work stealing — EOI only.
    let _ = register_handler(vectors::RESCHEDULE_IPI, reschedule_ipi_handler);

    // Register the cross-CPU wake function with the scheduler.
    hadron_sched::waker::set_wake_ipi_fn(send_wakeup_ipi);

    crate::kdebug!("smp", "IPI handlers registered");
}

/// Wakeup IPI handler — just EOI. Returning from the ISR resumes the executor.
fn wakeup_ipi_handler(_vector: crate::id::IrqVector) {
    // EOI is handled by the dispatch layer.
}

/// TLB shootdown IPI handler — invalidate the requested virtual address.
fn tlb_shootdown_ipi_handler(_vector: crate::id::IrqVector) {
    crate::arch::x86_64::tlb_shootdown::handle_shootdown_ipi();
}

/// Reschedule IPI handler — just EOI. The executor will check for work on next poll.
fn reschedule_ipi_handler(_vector: crate::id::IrqVector) {
    hadron_sched::set_preempt_pending();
}

/// Send a wakeup IPI to a target CPU.
///
/// Called by the scheduler's waker when a task is enqueued on a different CPU.
fn send_wakeup_ipi(target: hadron_core::id::CpuId) {
    if let Some(lapic_virt) = crate::arch::x86_64::acpi::Acpi::lapic_virt() {
        // Look up the target's LAPIC ID from CPU ID.
        let target_lapic_id = cpu_to_lapic(target.as_u32());
        if let Some(lapic_id) = target_lapic_id {
            // SAFETY: LAPIC is mapped and valid. Vector is a registered IPI.
            unsafe {
                let lapic = LocalApic::new(lapic_virt);
                use crate::arch::x86_64::interrupts::dispatch::vectors;
                lapic.send_ipi(lapic_id, vectors::WAKEUP_IPI.as_irq_vector());
            }
        }
    }
}

/// CPU ID → LAPIC ID reverse lookup (public for TLB shootdown).
pub fn cpu_to_lapic_pub(cpu_id: u32) -> Option<u8> {
    cpu_to_lapic(cpu_id)
}

/// CPU ID → LAPIC ID reverse lookup.
fn cpu_to_lapic(cpu_id: u32) -> Option<u8> {
    for (lapic_id, entry) in LAPIC_TO_CPU.iter().enumerate() {
        if entry.load(Ordering::Acquire) == cpu_id {
            #[expect(clippy::cast_possible_truncation, reason = "LAPIC IDs are 8-bit")]
            return Some(lapic_id as u8);
        }
    }
    None
}
