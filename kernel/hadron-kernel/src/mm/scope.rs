//! Scoped lock guard types for memory management subsystems.
//!
//! These types make lock ordering visible in function signatures.
//! A function that takes `PmmScope` guarantees it holds the PMM lock;
//! one that takes both `VmmScope` and `PmmScope` acquired them in the
//! correct ascending level order (VMM=2, PMM=3).
//!
//! # Lock Level Map
//!
//! Locks must be acquired in ascending level order. A lock at level N
//! may only be acquired while holding locks at levels ≤ N. Level 0 is
//! unassigned (leaf lock, no ordering check enforced by lockdep).
//!
//! ```text
//! Level  Lock(s)                                              Subsystem
//! ─────  ───────────────────────────────────────────────────── ─────────
//!   1    HEAP                                                 allocator
//!   2    VMM                                                  mm
//!   3    PMM                                                  mm
//!   4    PROCESS_TABLE, VFS, DEVICE_REGISTRY,                  kernel
//!        HPET_DRIVER, HKIF_STATE, fd_table, mmap_alloc,
//!        exit_status
//!   6    AHCI_DISK_INDEX, VIRTIO_DISK_INDEX                   drivers
//!  10    SCANCODE_BUF, CONSOLE_INPUT_STATE                    input
//!  11    PLATFORM (ACPI)                                      arch
//!  12    SLEEP_QUEUE                                          sched
//!  13    Executor.ready_queues                                sched
//!  14    Executor.tasks                                       sched
//!
//!   0    LOGGER, CURSOR                                       logging/display
//! ```
//!
//! `LOGGER` and `CURSOR` use level 0 (opt out of lockdep ordering) because
//! the logger is a cross-cutting concern called from within any lock's
//! critical section. Their ordering is enforced structurally: `CURSOR` is
//! only ever acquired inside `LOGGER` (via `LogSink::write_str`).
//!
//! Common nesting paths:
//! - Heap grow: `HEAP(1) → VMM(2) → PMM(3)`
//! - MMIO mapping: `VMM(2) → PMM(3)`
//! - Work stealing: `Executor.ready_queues(13) → Executor.tasks(14)`

use super::pmm::BitmapAllocator;
use super::vmm::KernelVmm;

/// Proof that the PMM lock is held. Only created by [`with_pmm_scope`].
pub struct PmmScope<'a> {
    pmm: &'a mut BitmapAllocator,
}

impl<'a> PmmScope<'a> {
    /// Returns a mutable reference to the physical frame allocator.
    pub fn allocator(&mut self) -> &mut BitmapAllocator {
        self.pmm
    }
}

/// Proof that the VMM lock is held. Only created by [`with_vmm_scope`].
pub struct VmmScope<'a> {
    vmm: &'a mut KernelVmm,
}

impl<'a> VmmScope<'a> {
    /// Returns a mutable reference to the virtual memory manager.
    pub fn vmm(&mut self) -> &mut KernelVmm {
        self.vmm
    }
}

/// Acquire the PMM lock and pass a typed scope token.
pub fn with_pmm_scope<R>(f: impl FnOnce(PmmScope<'_>) -> R) -> R {
    super::pmm::with_pmm(|pmm| f(PmmScope { pmm }))
}

/// Acquire the VMM lock and pass a typed scope token.
pub fn with_vmm_scope<R>(f: impl FnOnce(VmmScope<'_>) -> R) -> R {
    super::vmm::with_vmm(|vmm| f(VmmScope { vmm }))
}

/// Acquire both VMM and PMM in correct ascending level order (VMM=2 then PMM=3).
pub fn with_vmm_and_pmm<R>(f: impl FnOnce(VmmScope<'_>, PmmScope<'_>) -> R) -> R {
    super::vmm::with_vmm(|vmm| super::pmm::with_pmm(|pmm| f(VmmScope { vmm }, PmmScope { pmm })))
}
