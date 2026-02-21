//! Allocator microbenchmarks.
//!
//! Measures heap allocation and deallocation cycles using the kernel's
//! global allocator in QEMU.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_bench::bench_runner)]
#![reexport_test_harness_main = "bench_main"]

extern crate alloc;

hadron_bench::bench_entry_point_with_init!();

use alloc::boxed::Box;
use alloc::vec::Vec;
use hadron_bench::{Bencher, black_box};

#[test_case]
fn bench_box_alloc_dealloc(b: &mut Bencher) {
    b.iter(|| {
        let boxed = Box::new(42u64);
        black_box(boxed);
    });
}

#[test_case]
fn bench_vec_push_100(b: &mut Bencher) {
    b.iter(|| {
        let mut v = Vec::with_capacity(100);
        for i in 0..100u64 {
            v.push(i);
        }
        black_box(v);
    });
}

#[test_case]
fn bench_vec_alloc_4k(b: &mut Bencher) {
    b.iter(|| {
        let v: Vec<u8> = Vec::with_capacity(4096);
        black_box(v);
    });
}
