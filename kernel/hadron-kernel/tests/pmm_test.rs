//! Integration tests for the physical memory manager.
//!
//! Tests frame allocation and deallocation in the real QEMU environment.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

hadron_test::test_entry_point_with_init!();

#[test_case]
fn allocate_single_frame() {
    hadron_kernel::mm::pmm::with_pmm(|pmm| {
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

#[test_case]
fn allocate_and_deallocate() {
    hadron_kernel::mm::pmm::with_pmm(|pmm| {
        let free_before = pmm.free_frames();
        let frame = pmm.allocate_frame().expect("should allocate");
        assert_eq!(pmm.free_frames(), free_before - 1);
        unsafe {
            pmm.deallocate_frame(frame).expect("should deallocate");
        }
        assert_eq!(pmm.free_frames(), free_before);
    });
}

#[test_case]
fn allocate_contiguous() {
    hadron_kernel::mm::pmm::with_pmm(|pmm| {
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
