//! Virtual memory manager tests — heap watermark, PMM consistency, HHDM.

extern crate alloc;

use alloc::boxed::Box;
use hadron_ktest::kernel_test;

// ── Early boot stage ────────────────────────────────────────────────────

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_heap_watermark_increases() {
    let wm_before = crate::mm::vmm::with(|vmm| vmm.heap_watermark());

    // Force a heap allocation to grow the watermark.
    let _b = Box::new([0u8; 4096]);

    let wm_after = crate::mm::vmm::with(|vmm| vmm.heap_watermark());
    assert!(
        wm_after >= wm_before,
        "heap watermark should not decrease after allocation: before={:#x}, after={:#x}",
        wm_before.as_u64(),
        wm_after.as_u64()
    );
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_pmm_free_count_consistency() {
    crate::mm::pmm::with(|pmm| {
        let before = pmm.free_frames();
        let frame = pmm.allocate_frame().expect("should allocate");
        assert_eq!(pmm.free_frames(), before - 1);
        unsafe {
            pmm.deallocate_frame(frame).expect("should dealloc");
        }
        assert_eq!(
            pmm.free_frames(),
            before,
            "free count should be restored after alloc+dealloc"
        );
    });
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_hhdm_phys_to_virt_roundtrip() {
    use crate::addr::PhysAddr;

    // Use a known safe physical address (first frame after zero).
    let phys = PhysAddr::new(0x1000);
    let virt = crate::mm::hhdm::phys_to_virt(phys);
    let hhdm = crate::mm::hhdm::offset();

    assert_eq!(
        virt.as_u64(),
        hhdm + 0x1000,
        "phys_to_virt should add HHDM offset"
    );
}

// ── Before executor stage ───────────────────────────────────────────────

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_mmio_map_unmap() {
    use crate::addr::PhysAddr;

    // Map a small MMIO region (1 page). Use a physical address in the
    // QEMU MMIO hole (e.g. 0xFED0_0000 — HPET area) that we know exists.
    let phys = PhysAddr::new(0xFED0_0000);
    let mapping = crate::mm::vmm::map_mmio_region(phys, 4096);

    let virt = mapping.virt_base();
    assert_ne!(
        virt.as_u64(),
        0,
        "MMIO mapping should produce non-null virtual address"
    );
    assert_eq!(mapping.size(), 4096);
    assert_eq!(mapping.phys_base(), phys);

    // Drop unmaps — no crash means success.
    drop(mapping);
}
