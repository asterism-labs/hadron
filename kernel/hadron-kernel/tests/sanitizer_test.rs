//! Integration tests for heap and PMM sanitizers.
//!
//! Verifies that poison fill patterns and red zones don't produce false
//! positives during normal kernel operations. When sanitizers are disabled
//! (default config), these tests still pass — they exercise regular
//! alloc/dealloc without the poison checks.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

extern crate alloc;

hadron_test::test_entry_point_with_init!();

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;

#[test_case]
fn test_box_alloc_dealloc_no_corruption() {
    let mut b = Box::new([0u8; 128]);
    for byte in b.iter_mut() {
        *byte = 0xAB;
    }
    assert!(b.iter().all(|&b| b == 0xAB));
    drop(b);
}

#[test_case]
fn test_vec_growth_no_false_positives() {
    let mut v: Vec<u64> = Vec::new();
    for i in 0..1000u64 {
        v.push(i);
    }
    assert_eq!(v.len(), 1000);
    assert_eq!(v[999], 999);
}

#[test_case]
fn test_many_small_allocations() {
    for i in 0u8..250 {
        let b = Box::new([i; 32]);
        assert_eq!(b[0], i);
        drop(b);
    }
}

#[test_case]
fn test_large_allocation() {
    let size = 256 * 1024;
    let mut v: Vec<u8> = vec![0; size];
    for (i, byte) in v.iter_mut().enumerate() {
        *byte = (i & 0xFF) as u8;
    }
    assert_eq!(v.len(), size);
    drop(v);
}

#[test_case]
fn test_pmm_alloc_dealloc_cycle() {
    // Allocate a frame, deallocate it (poisons), re-allocate (checks poison).
    hadron_kernel::mm::pmm::with_pmm(|pmm| {
        let frame = pmm.allocate_frame().expect("failed to allocate frame");
        unsafe {
            pmm.deallocate_frame(frame)
                .expect("failed to deallocate frame");
        }
        // Re-allocate — if poison is active, the check runs here.
        let _frame2 = pmm.allocate_frame().expect("failed to re-allocate frame");
    });
}

#[test_case]
fn test_pmm_multi_frame_cycle() {
    hadron_kernel::mm::pmm::with_pmm(|pmm| {
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

#[test_case]
fn test_alloc_stats_smoke() {
    // Exercise alloc tracking if enabled; just verify no crash.
    let b1 = Box::new(42u64);
    let b2 = Box::new([0u8; 256]);
    #[cfg(hadron_debug_alloc_track)]
    hadron_kernel::mm::heap::dump_alloc_stats();
    drop(b1);
    drop(b2);
}
