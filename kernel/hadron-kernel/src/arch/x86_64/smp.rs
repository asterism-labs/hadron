//! SMP bootstrap: boots Application Processors (APs) using bootloader MP data.
//!
//! AP startup uses a two-phase approach:
//!
//! **Phase 1 — Parking (boot stub):** The boot stub calls [`park_aps`] right
//! after switching CR3 to the kernel page tables. This starts each AP via
//! Limine's `goto_address` mechanism, but sends them to a parking function
//! ([`ap_early_park`]) that immediately switches the AP's CR3 to the kernel
//! page tables and spins. This prevents APs from crashing in Limine's spin
//! loop when the BSP's kernel init modifies memory layouts.
//!
//! **Phase 2 — Initialization (kernel_init):** The BSP calls [`boot_aps`]
//! after platform init. It allocates per-CPU state (PerCpu, GDT, TSS) for
//! each AP, stores PerCpu addresses in a shared table, and releases the
//! parked APs. Each AP then completes full initialization (GDT, IDT, LAPIC,
//! SYSCALL MSRs) and enters the executor loop.

extern crate alloc;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

use alloc::boxed::Box;

use crate::arch::x86_64::hw::local_apic::LocalApic;
use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};
use crate::id::CpuId;
use crate::percpu::{MAX_CPUS, PerCpu};
use crate::{kdebug, kinfo, kwarn};

use crate::boot::{BootInfo, SmpCpuEntry};

// ---------------------------------------------------------------------------
// Phase 1: AP parking (called from boot stub before kernel_init)
// ---------------------------------------------------------------------------

/// Kernel CR3 value for APs to switch to when parking.
static AP_KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

/// Number of APs that have parked (switched to kernel page tables).
static AP_PARKED_COUNT: AtomicU32 = AtomicU32::new(0);

/// Flag set by `boot_aps` to release parked APs for full initialization.
static AP_RELEASE: AtomicBool = AtomicBool::new(false);

/// Per-AP PerCpu addresses, indexed by LAPIC ID. Written by `boot_aps`,
/// read by parked APs after release.
static AP_PERCPU_TABLE: [AtomicU64; MAX_CPUS] = {
    const ZERO: AtomicU64 = AtomicU64::new(0);
    [ZERO; MAX_CPUS]
};

/// Parks all APs on kernel page tables immediately after the CR3 switch.
///
/// Called by the boot stub (hadron-boot-limine) right after switching the
/// BSP's CR3 to the kernel-owned page tables. For each AP, this writes the
/// parking trampoline address to Limine's `goto_address`, causing the AP to
/// leave Limine's spin loop and switch to the kernel page tables.
///
/// After this function returns, all APs are safely parked with kernel CR3
/// and will remain spinning until [`boot_aps`] releases them.
///
/// # Arguments
///
/// * `smp_cpus` — AP entries from the Limine MP response (BSP excluded)
/// * `kernel_cr3` — Physical address of the kernel PML4
pub fn park_aps(smp_cpus: &[SmpCpuEntry], kernel_cr3: u64) {
    let ap_count = smp_cpus.len();
    if ap_count == 0 {
        return;
    }

    // Store kernel CR3 so APs can read it in the parking trampoline.
    AP_KERNEL_CR3.store(kernel_cr3, Ordering::Release);

    // Start each AP with the parking trampoline.
    for cpu_entry in smp_cpus {
        // SAFETY: The boot stub has just switched CR3 and the Limine MP info
        // memory is still valid and mapped via HHDM. ap_early_park is a valid
        // entry point matching Limine's calling convention.
        unsafe {
            cpu_entry.start(ap_early_park as *const () as usize, 0);
        }
    }

    // Wait for all APs to park (with timeout).
    let expected = ap_count as u32;
    let mut spin_count = 0u64;
    const PARK_TIMEOUT: u64 = 100_000_000;

    while AP_PARKED_COUNT.load(Ordering::Acquire) < expected {
        core::hint::spin_loop();
        spin_count += 1;
        if spin_count >= PARK_TIMEOUT {
            kwarn!(
                "SMP: Timeout parking APs ({}/{} parked)",
                AP_PARKED_COUNT.load(Ordering::Acquire),
                expected
            );
            break;
        }
    }

    let parked = AP_PARKED_COUNT.load(Ordering::Acquire);
    kinfo!("SMP: {} APs parked on kernel page tables", parked);
}

/// AP parking trampoline. Limine calls this with RDI = MpInfo*, RSI = extra_argument.
///
/// Immediately switches CR3 to the kernel page tables, parks the AP in a spin
/// loop, and waits for `boot_aps` to release it with per-CPU data.
extern "C" fn ap_early_park(mp_info: u64, _extra: u64) -> ! {
    // 1. Switch to kernel page tables immediately.
    // SAFETY: AP_KERNEL_CR3 was stored with Release before starting this AP.
    // The kernel page tables are valid and map the HHDM, kernel image, and
    // the Limine stack this AP is using.
    let cr3 = AP_KERNEL_CR3.load(Ordering::Acquire);
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) cr3, options(nostack, preserves_flags));
    }

    // 2. Read our LAPIC ID from the MpInfo struct (offset 4: lapic_id field).
    // SAFETY: mp_info points to a valid MpInfo in bootloader memory, which is
    // still accessible via HHDM in the kernel page tables.
    let lapic_id = unsafe { *((mp_info as *const u8).add(4) as *const u32) };

    // 3. Signal BSP that we are parked.
    AP_PARKED_COUNT.fetch_add(1, Ordering::Release);

    // 4. Spin until boot_aps releases us.
    while !AP_RELEASE.load(Ordering::Acquire) {
        core::hint::spin_loop();
    }

    // 5. Read our PerCpu address from the shared table.
    let percpu_addr = AP_PERCPU_TABLE[lapic_id as usize].load(Ordering::Acquire);
    assert!(percpu_addr != 0, "AP released without PerCpu data");

    // 6. Continue with full AP initialization.
    // SAFETY: percpu_addr was set by boot_aps and points to a valid, leaked
    // PerCpu struct. This function never returns.
    ap_entry(mp_info, percpu_addr);
}

// ---------------------------------------------------------------------------
// Phase 2: AP initialization (called from kernel_init)
// ---------------------------------------------------------------------------

/// Counter of APs that have completed full initialization.
static AP_READY_COUNT: AtomicU32 = AtomicU32::new(0);

/// Initializes all parked Application Processors.
///
/// For each AP:
/// 1. Heap-allocates a [`PerCpu`] struct and populates it
/// 2. Stores the PerCpu address in [`AP_PERCPU_TABLE`] (indexed by LAPIC ID)
/// 3. Releases all parked APs by setting [`AP_RELEASE`]
/// 4. Waits for all APs to signal readiness
///
/// After this function returns, all CPUs are online and running their
/// executor loops.
pub fn boot_aps(boot_info: &impl BootInfo) {
    let smp_cpus = boot_info.smp_cpus();
    let ap_count = smp_cpus.len();

    if ap_count == 0 {
        kinfo!("SMP: No APs to boot (single-processor system)");
        return;
    }

    kinfo!("SMP: Initializing {} parked APs...", ap_count);

    // Allocate per-CPU state for each AP and store in the shared table.
    for (i, cpu_entry) in smp_cpus.iter().enumerate() {
        let cpu_id = CpuId::new((i + 1) as u32); // BSP is CPU 0

        // Heap-allocate a PerCpu for this AP. Leaked because it must live forever.
        let percpu = Box::leak(Box::new(PerCpu::new()));
        let percpu_addr = percpu as *const PerCpu as u64;
        percpu.self_ptr = percpu_addr;
        percpu.init(cpu_id, cpu_entry.lapic_id as u8);
        crate::sched::smp::register_cpu_apic_id(cpu_id, cpu_entry.lapic_id as u8);

        kdebug!(
            "SMP: Prepared AP {} (LAPIC ID={}, PerCpu={:#x})",
            cpu_id,
            cpu_entry.lapic_id,
            percpu_addr
        );

        // Store PerCpu address indexed by LAPIC ID for the parked AP to find.
        AP_PERCPU_TABLE[cpu_entry.lapic_id as usize].store(percpu_addr, Ordering::Release);
    }

    // Release all parked APs. The Release ordering ensures all PerCpu
    // table writes are visible before APs read them.
    AP_RELEASE.store(true, Ordering::Release);
    kinfo!("SMP: Released {} APs for initialization", ap_count);

    // Wait for all APs to complete their initialization (with timeout).
    let expected = ap_count as u32;
    let mut spin_count = 0u64;
    const SPIN_TIMEOUT: u64 = 100_000_000; // ~a few seconds on modern CPUs

    while AP_READY_COUNT.load(Ordering::Acquire) < expected {
        core::hint::spin_loop();
        spin_count += 1;
        if spin_count >= SPIN_TIMEOUT {
            kwarn!(
                "SMP: Timeout waiting for APs ({}/{} ready)",
                AP_READY_COUNT.load(Ordering::Acquire),
                expected
            );
            break;
        }
    }

    let ready = AP_READY_COUNT.load(Ordering::Acquire);
    crate::percpu::PerCpuState::set_cpu_count(1 + ready);
    kinfo!("SMP: {} APs online ({} total CPUs)", ready, 1 + ready);
}

/// Full AP initialization. Called from [`ap_early_park`] after release, or
/// could be called directly if parking is not used.
///
/// Sets up per-CPU state (GS base, GDT, TSS, IDT, SYSCALL MSRs, LAPIC)
/// and enters the executor loop.
fn ap_entry(_mp_info: u64, percpu_addr: u64) -> ! {
    let percpu = unsafe { &*(percpu_addr as *const PerCpu) };
    let cpu_id = percpu.get_cpu_id();

    // 1. Initialize per-CPU GDT and TSS (allocates kernel stack).
    // Must be done BEFORE setting GS base because `load_gs(null)` in GDT
    // init clears the GS base MSR on Intel CPUs.
    // SAFETY: Heap and VMM are initialized by BSP. Called once per AP.
    let kernel_stack_top = unsafe { super::gdt::init_ap(cpu_id) };

    // 2. Set GS base to our PerCpu struct.
    // Done AFTER GDT init because `load_gs(null_selector)` clears GS base.
    // SAFETY: percpu_addr is a valid, leaked PerCpu pointer.
    unsafe {
        IA32_GS_BASE.write(percpu_addr);
        IA32_KERNEL_GS_BASE.write(percpu_addr);
    }

    // Update PerCpu with the kernel stack top from GDT/TSS init.
    // SAFETY: We have exclusive access to this AP's PerCpu; no other CPU
    // references it.
    unsafe {
        let percpu_mut = percpu_addr as *mut PerCpu;
        (*percpu_mut).kernel_rsp = kernel_stack_top;
    }

    // 3. Load IDT (shared static, same as BSP).
    // SAFETY: IDT is initialized by BSP and is a shared immutable static.
    unsafe { super::idt::init() };

    // 4. Initialize SYSCALL/SYSRET MSRs.
    // SAFETY: GDT is loaded, GS base is set.
    unsafe { crate::arch::x86_64::syscall::init() };

    // 4b. Populate per-CPU pointers for assembly stubs (timer, syscall).
    // These pointers let naked ASM access per-CPU CpuLocal elements via
    // GS:[offset] instead of RIP-relative addressing.
    // SAFETY: We have exclusive access to this AP's PerCpu. The CpuLocal
    // elements are static and live forever. get_for(cpu_id) returns the
    // correct element for this AP.
    unsafe {
        use crate::proc;
        let percpu_mut = percpu_addr as *mut PerCpu;
        (*percpu_mut).user_context_ptr = proc::USER_CONTEXT.get_for(cpu_id).get() as u64;
        (*percpu_mut).saved_kernel_rsp_ptr =
            proc::SAVED_KERNEL_RSP.get_for(cpu_id) as *const _ as u64;
        (*percpu_mut).trap_reason_ptr = proc::TRAP_REASON.get_for(cpu_id) as *const _ as u64;
        (*percpu_mut).saved_regs_ptr = crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
            .get_for(cpu_id)
            .get() as u64;
    }

    // 5. Enable this AP's Local APIC and start its timer.
    init_ap_lapic(cpu_id);

    // 6. Signal BSP that we are ready.
    AP_READY_COUNT.fetch_add(1, Ordering::Release);

    kinfo!(
        "SMP: AP {} online (LAPIC ID={})",
        cpu_id,
        percpu.get_apic_id()
    );

    // 7. Enable interrupts and enter this AP's executor loop.
    // SAFETY: All interrupt infrastructure is initialized.
    unsafe { crate::arch::x86_64::instructions::interrupts::enable() };

    // Run this AP's per-CPU executor. Initially empty — tasks arrive via
    // cross-CPU wakeup (Step 12.5) or work stealing (Step 12.6).
    crate::sched::executor().run(&crate::sched::X86ArchHalt, crate::sched::smp::try_steal);
}

/// Initializes the Local APIC on an AP and starts the periodic timer.
fn init_ap_lapic(cpu_id: CpuId) {
    use crate::arch::x86_64::interrupts::dispatch::vectors;

    let lapic_virt = super::acpi::Acpi::lapic_virt().expect("AP bootstrap: LAPIC not initialized by BSP");

    // SAFETY: lapic_virt was mapped by BSP and is valid for this CPU's LAPIC.
    let lapic = unsafe { LocalApic::new(lapic_virt) };
    lapic.enable(vectors::SPURIOUS.as_irq_vector());
    lapic.set_tpr(0);

    // Start periodic timer using BSP's calibrated values.
    let (initial_count, divide) = super::acpi::Acpi::lapic_timer_config();
    if initial_count > 0 {
        lapic.start_timer_periodic(vectors::TIMER.as_irq_vector(), initial_count, divide);
        kdebug!(
            "SMP: AP {} LAPIC timer started (initial_count={}, divide={})",
            cpu_id,
            initial_count,
            divide
        );
    } else {
        kwarn!(
            "SMP: AP {} LAPIC timer not started (BSP calibration not available)",
            cpu_id
        );
    }
}
