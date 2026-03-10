//! Kernel entry point called by the UEFI boot stub.
//!
//! Executes the full BSP boot sequence: GDT → IDT → TLB registration →
//! PMM → VMM → heap → per-CPU state → logging flush.

use hadron_core::addr::VirtAddr;

use crate::boot::BootInfo;

/// Kernel entry point.
///
/// The UEFI boot stub jumps here after setting up page tables, loading
/// the kernel ELF, and building the `BootInfo` struct.
///
/// # Safety
///
/// `boot_info` must point to a valid, fully initialized `BootInfo` struct
/// in memory accessible under the kernel's page tables.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_init(boot_info: *const BootInfo) -> ! {
    // SAFETY: boot_info was set up by the UEFI stub and is in HHDM-mapped memory.
    let bi = unsafe { &*boot_info };

    // ── 1. Initialize HHDM offset ──────────────────────────────────────
    let hhdm_offset = VirtAddr::new_truncate(bi.hhdm_offset);
    hadron_mm::hhdm::init(hhdm_offset);
    crate::kinfo!("boot", "HHDM initialized at {:#x}", bi.hhdm_offset);

    // ── 2. Boot mapper ─────────────────────────────────────────────────
    // The boot stub maps the first 4 GiB into the HHDM. The boot mapper
    // callback extends the HHDM beyond 4 GiB, but after GDT/IDT init the
    // callback into stub code is no longer safe (the stub was compiled for
    // the UEFI target with different segment assumptions). For systems with
    // <=4 GiB RAM the initial mapping suffices.
    // TODO: Support >4 GiB by extending HHDM from the kernel's own VMM.
    crate::kdebug!("boot", "boot mapper skipped (4 GiB HHDM sufficient)");

    // ── 3. GDT init ────────────────────────────────────────────────────
    // SAFETY: Called exactly once during BSP init. No interrupts yet.
    unsafe { crate::arch::x86_64::gdt::init() };

    // ── 4. IDT init ────────────────────────────────────────────────────
    // SAFETY: Called after GDT init, CS is valid.
    unsafe { crate::arch::x86_64::idt::init() };

    // ── 5. Register TLB flush callback ─────────────────────────────────
    hadron_mm::mapper::register_tlb_flush(crate::arch::x86_64::instructions::tlb::flush);
    crate::kdebug!("boot", "TLB flush registered");

    // ── 6. Convert UEFI memory map ─────────────────────────────────────
    // SAFETY: bi contains a valid UEFI memory map; HHDM is initialized.
    let mem = unsafe { crate::boot::convert_uefi_memory_map(bi, hhdm_offset) };
    let regions = &mem.regions[..mem.count];
    crate::kinfo!(
        "boot",
        "UEFI memory map: {} regions, max_phys {:#x}",
        mem.count,
        mem.max_phys
    );

    // ── 7. PMM init ────────────────────────────────────────────────────
    let boot_reserved = [
        (bi.kernel_phys, bi.kernel_size),
        (bi.boot_pt_pool_phys, bi.boot_pt_pool_pages * 4096),
    ];
    hadron_mm::pmm::init(regions, hhdm_offset, &boot_reserved);
    crate::kinfo!("boot", "PMM initialized");

    // ── 8. VMM init + heap ─────────────────────────────────────────────
    let root_phys = crate::arch::x86_64::registers::control::Cr3::read();
    let mapper = crate::arch::x86_64::paging::PageTableMapper::new(hhdm_offset);
    let mut vmm = hadron_mm::vmm::Vmm::new(root_phys, mapper, hhdm_offset, mem.max_phys);

    let (heap_base, heap_size) = hadron_mm::pmm::with(|pmm| {
        let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
        vmm.map_initial_heap(&mut alloc)
            .expect("failed to map initial heap")
    });

    // SAFETY: heap_base points to a freshly mapped and zeroed region.
    unsafe { hadron_mm::heap::init_raw(heap_base.as_u64() as usize, heap_size as usize) };
    crate::kinfo!(
        "boot",
        "heap initialized: base {:#x}, size {} KiB",
        heap_base.as_u64(),
        heap_size / 1024
    );

    // ── 9. Store VMM globally ──────────────────────────────────────────
    crate::vmm::init(vmm);

    // ── 10. Per-CPU init (GS base) ─────────────────────────────────────
    // After this, cpu_is_initialized() returns true and logging switches
    // from Phase 0 (direct serial) to Phase 1 (ring buffer + sinks).
    // SAFETY: Called once after GDT, before any code that reads GS-relative data.
    unsafe { crate::percpu::init_gs_base() };
    crate::kinfo!("boot", "BSP per-CPU state initialized");

    // ── 11. Flush logs ─────────────────────────────────────────────────
    // Drain any Phase 1 messages that were buffered during init.
    crate::flush();

    // ── 12. Boot complete ──────────────────────────────────────────────
    crate::kinfo!("boot", "kernel bootstrap complete");
    crate::flush();

    // ── 13. Spin (placeholder for scheduler) ───────────────────────────
    loop {
        core::hint::spin_loop();
    }
}
