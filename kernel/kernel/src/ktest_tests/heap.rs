//! Heap allocator tests — Box, Vec, and allocation stress.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use alloc::vec::Vec;
use hadron_ktest::kernel_test;

// ── Migrated from heap_test.rs ──────────────────────────────────────────

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_box_allocation() {
    let b = Box::new(42u64);
    assert_eq!(*b, 42);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_vec_push_and_read() {
    let mut v = Vec::new();
    for i in 0..100u64 {
        v.push(i);
    }
    assert_eq!(v.len(), 100);
    for (i, &val) in v.iter().enumerate() {
        assert_eq!(val, i as u64);
    }
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_large_allocation() {
    let v: Vec<u8> = vec![0xAA; 64 * 1024];
    assert_eq!(v.len(), 64 * 1024);
    assert!(v.iter().all(|&b| b == 0xAA));
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_alloc_dealloc_cycles() {
    for i in 0..50u64 {
        let b = Box::new(i);
        assert_eq!(*b, i);
        drop(b);
    }
}

// ── Migrated from sanitizer_test.rs (heap tests) ───────────────────────

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_box_alloc_dealloc_no_corruption() {
    let mut b = Box::new([0u8; 128]);
    for byte in b.iter_mut() {
        *byte = 0xAB;
    }
    assert!(b.iter().all(|&b| b == 0xAB));
    drop(b);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_vec_growth_no_false_positives() {
    let mut v: Vec<u64> = Vec::new();
    for i in 0..1000u64 {
        v.push(i);
    }
    assert_eq!(v.len(), 1000);
    assert_eq!(v[999], 999);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_many_small_allocations() {
    for i in 0u8..250 {
        let b = Box::new([i; 32]);
        assert_eq!(b[0], i);
        drop(b);
    }
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_large_allocation_sanitizer() {
    let size = 256 * 1024;
    let mut v: Vec<u8> = vec![0; size];
    for (i, byte) in v.iter_mut().enumerate() {
        *byte = (i & 0xFF) as u8;
    }
    assert_eq!(v.len(), size);
    drop(v);
}

#[kernel_test(stage = "early_boot", timeout = 5)]
fn test_alloc_stats_smoke() {
    // Exercise alloc tracking if enabled; just verify no crash.
    let b1 = Box::new(42u64);
    let b2 = Box::new([0u8; 256]);
    #[cfg(hadron_debug_alloc_track)]
    crate::mm::heap::dump_alloc_stats();
    drop(b1);
    drop(b2);
}
