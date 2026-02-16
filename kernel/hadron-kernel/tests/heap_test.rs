//! Integration tests for the kernel heap allocator.
//!
//! Tests Box, Vec, and allocation stress in the real QEMU environment.

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
fn box_allocation() {
    let b = Box::new(42u64);
    assert_eq!(*b, 42);
}

#[test_case]
fn vec_push_and_read() {
    let mut v = Vec::new();
    for i in 0..100u64 {
        v.push(i);
    }
    assert_eq!(v.len(), 100);
    for (i, &val) in v.iter().enumerate() {
        assert_eq!(val, i as u64);
    }
}

#[test_case]
fn large_allocation() {
    let v: Vec<u8> = vec![0xAA; 64 * 1024];
    assert_eq!(v.len(), 64 * 1024);
    assert!(v.iter().all(|&b| b == 0xAA));
}

#[test_case]
fn alloc_dealloc_cycles() {
    for i in 0..50u64 {
        let b = Box::new(i);
        assert_eq!(*b, i);
        drop(b);
    }
}
