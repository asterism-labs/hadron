//! Physical memory manager tests — frame allocation and deallocation.

extern crate alloc;

use hadron_ktest::kernel_test;

// ── Migrated from pmm_test.rs ───────────────────────────────────────────

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_allocate_single_frame() {
    crate::mm::pmm::with_pmm(|pmm| {
        let free_before = pmm.free_frames();
        let frame = pmm.allocate_frame();
        assert!(frame.is_some(), "should allocate a frame");
        let frame = frame.unwrap();
        assert_eq!(
            frame.start_address().as_u64() % 4096,
            0,
            "frame should be 4 KiB aligned"
        );
        assert_eq!(pmm.free_frames(), free_before - 1);
    });
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_allocate_and_deallocate() {
    crate::mm::pmm::with_pmm(|pmm| {
        let free_before = pmm.free_frames();
        let frame = pmm.allocate_frame().expect("should allocate");
        assert_eq!(pmm.free_frames(), free_before - 1);
        unsafe {
            pmm.deallocate_frame(frame).expect("should deallocate");
        }
        assert_eq!(pmm.free_frames(), free_before);
    });
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_allocate_contiguous() {
    crate::mm::pmm::with_pmm(|pmm| {
        let free_before = pmm.free_frames();
        let mut frames = alloc::vec::Vec::new();
        for _ in 0..4 {
            let f = pmm.allocate_frame().expect("should allocate");
            frames.push(f);
        }
        assert_eq!(pmm.free_frames(), free_before - 4);
        for f in frames {
            unsafe {
                pmm.deallocate_frame(f).expect("should deallocate");
            }
        }
        assert_eq!(pmm.free_frames(), free_before);
    });
}

// ── Migrated from sanitizer_test.rs (PMM tests) ────────────────────────

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_pmm_alloc_dealloc_cycle() {
    // Allocate a frame, deallocate it (poisons), re-allocate (checks poison).
    crate::mm::pmm::with_pmm(|pmm| {
        let frame = pmm.allocate_frame().expect("failed to allocate frame");
        unsafe {
            pmm.deallocate_frame(frame)
                .expect("failed to deallocate frame");
        }
        // Re-allocate — if poison is active, the check runs here.
        let _frame2 = pmm.allocate_frame().expect("failed to re-allocate frame");
    });
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_pmm_multi_frame_cycle() {
    crate::mm::pmm::with_pmm(|pmm| {
        // Allocate 8 contiguous frames.
        let base = pmm.allocate_frames(8).expect("failed to allocate 8 frames");
        // Deallocate all 8 (each gets poisoned).
        unsafe {
            pmm.deallocate_frames(base, 8)
                .expect("failed to deallocate 8 frames");
        }
        // Re-allocate 8 — poison check runs on each frame.
        let _base2 = pmm
            .allocate_frames(8)
            .expect("failed to re-allocate 8 frames");
    });
}
