//! Sample kernel tests demonstrating the `#[kernel_test]` framework.
//!
//! These tests exercise the ktest infrastructure at each boot stage.
//! They are compiled only when `--cfg ktest` is active.

extern crate alloc;

use alloc::boxed::Box;
use alloc::vec;
use hadron_ktest::kernel_test;

// ── Early boot stage ────────────────────────────────────────────────────

/// Verifies that the heap allocator works.
#[kernel_test(stage = "early_boot")]
fn test_heap_alloc() {
    let b = Box::new(42u64);
    assert_eq!(*b, 42);
}

/// Verifies that Vec allocation and push work.
#[kernel_test(stage = "early_boot")]
fn test_vec_alloc() {
    let mut v = vec![1u32, 2, 3];
    v.push(4);
    assert_eq!(v.len(), 4);
    assert_eq!(v[3], 4);
}

// ── Before executor stage ───────────────────────────────────────────────

/// Verifies that the VFS is mounted and the root inode is accessible.
#[kernel_test(stage = "before_executor")]
fn test_vfs_root_exists() {
    let result = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/"));
    assert!(result.is_ok(), "VFS root should be resolvable");
}

// ── With executor stage ─────────────────────────────────────────────────

/// Simple async test that yields and completes.
#[kernel_test(stage = "with_executor")]
async fn test_async_yield() {
    crate::sched::primitives::yield_now().await;
}

/// Instanced async test: two concurrent tasks synchronize via barrier.
#[kernel_test(stage = "with_executor", instances = 0..=1)]
async fn test_barrier_sync(ctx: &hadron_ktest::TestContext) {
    // Both instances arrive at the barrier before proceeding.
    ctx.barrier().await;
}
