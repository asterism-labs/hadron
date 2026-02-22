//! Process lifecycle tests — signals, PID allocation, FD table.

extern crate alloc;

use alloc::sync::Arc;
use hadron_ktest::kernel_test;

use crate::proc::signal::{Signal, SignalState};
use crate::syscall::{SIGKILL, SIGTERM};

// ── Before executor stage — signal tests ────────────────────────────────

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_signal_post_and_dequeue() {
    let state = SignalState::new();
    state.post(SIGTERM);
    let sig = state.dequeue();
    assert_eq!(sig, Some(Signal(SIGTERM)), "should dequeue SIGTERM");
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_signal_dequeue_empty() {
    let state = SignalState::new();
    assert_eq!(state.dequeue(), None, "fresh SignalState should dequeue None");
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_signal_sigkill_priority() {
    let state = SignalState::new();
    state.post(SIGTERM);
    state.post(SIGKILL);
    let first = state.dequeue();
    assert_eq!(
        first,
        Some(Signal(SIGKILL)),
        "SIGKILL should be dequeued before SIGTERM"
    );
    let second = state.dequeue();
    assert_eq!(second, Some(Signal(SIGTERM)), "SIGTERM should follow SIGKILL");
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_signal_has_pending() {
    let state = SignalState::new();
    assert!(!state.has_pending(), "fresh state should have no pending");

    state.post(SIGTERM);
    assert!(state.has_pending(), "should have pending after post");

    state.dequeue();
    assert!(!state.has_pending(), "should have no pending after dequeue");
}

// ── Before executor stage — PID allocation ──────────────────────────────

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_pid_allocation_sequential() {
    use crate::mm::address_space::AddressSpace;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();
    let hhdm = crate::mm::hhdm::offset();

    #[cfg(target_arch = "x86_64")]
    type KernelMapper = crate::arch::x86_64::paging::PageTableMapper;

    fn dealloc_frame(frame: crate::paging::PhysFrame<crate::paging::Size4KiB>) {
        crate::mm::pmm::with(|pmm| unsafe {
            let _ = pmm.deallocate_frame(frame);
        });
    }

    // Create two processes inside the PMM lock, but return them so they
    // drop *outside* the lock (Drop -> dealloc_frame -> pmm::with).
    let (p1, p2) = crate::mm::pmm::with(|pmm| {
        let mut alloc = crate::mm::pmm::BitmapFrameAllocRef(pmm);

        let as1 = unsafe {
            AddressSpace::new_user(kernel_cr3, KernelMapper::new(hhdm), hhdm, &mut alloc, dealloc_frame)
                .expect("create address space 1")
        };
        let p1 = crate::proc::Process::new(as1, None);

        let as2 = unsafe {
            AddressSpace::new_user(kernel_cr3, KernelMapper::new(hhdm), hhdm, &mut alloc, dealloc_frame)
                .expect("create address space 2")
        };
        let p2 = crate::proc::Process::new(as2, None);

        (p1, p2)
    });

    assert_eq!(
        p2.pid.as_u32(),
        p1.pid.as_u32() + 1,
        "sequential Process::new() should yield sequential PIDs: {} -> {}",
        p1.pid.as_u32(),
        p2.pid.as_u32()
    );
    // p1, p2 drop here — outside the PMM lock.
}

// ── With executor stage — process table ─────────────────────────────────

#[kernel_test(stage = "with_executor", timeout = 10)]
async fn test_process_table_register_lookup() {
    use crate::mm::address_space::AddressSpace;

    let kernel_cr3 = crate::proc::TrapContext::kernel_cr3();
    let hhdm = crate::mm::hhdm::offset();

    #[cfg(target_arch = "x86_64")]
    type KernelMapper = crate::arch::x86_64::paging::PageTableMapper;

    fn dealloc_frame(frame: crate::paging::PhysFrame<crate::paging::Size4KiB>) {
        crate::mm::pmm::with(|pmm| unsafe {
            let _ = pmm.deallocate_frame(frame);
        });
    }

    // Create process inside PMM lock, return it so it drops outside.
    let process = crate::mm::pmm::with(|pmm| {
        let mut alloc = crate::mm::pmm::BitmapFrameAllocRef(pmm);
        let addr_space = unsafe {
            AddressSpace::new_user(kernel_cr3, KernelMapper::new(hhdm), hhdm, &mut alloc, dealloc_frame)
                .expect("create address space")
        };
        Arc::new(crate::proc::Process::new(addr_space, None))
    });

    let pid = process.pid;
    crate::proc::ProcessTable::register(&process);

    let found = crate::proc::ProcessTable::lookup(pid);
    assert!(found.is_some(), "registered process should be found by PID");
    assert_eq!(found.unwrap().pid, pid);

    // Clean up: unregister first, then drop the Arc (which may trigger
    // Process::Drop -> dealloc_frame -> pmm::with).
    crate::proc::ProcessTable::unregister(pid);
    assert!(
        crate::proc::ProcessTable::lookup(pid).is_none(),
        "unregistered process should not be found"
    );
    drop(process);
}

// ── With executor stage — FD table ──────────────────────────────────────

#[kernel_test(stage = "with_executor", timeout = 10)]
async fn test_fd_table_open_close() {
    use crate::fs::file::{FileDescriptorTable, OpenFlags};

    let inode = crate::fs::vfs::with_vfs(|vfs| vfs.resolve("/dev/null"))
        .expect("resolve /dev/null");

    let mut fdt = FileDescriptorTable::new();
    let fd = fdt.open(inode, OpenFlags::READ);

    assert!(fdt.get(fd).is_some(), "opened fd should be accessible");

    fdt.close(fd).expect("close should succeed");
    assert!(fdt.get(fd).is_none(), "closed fd should not be accessible");
}
