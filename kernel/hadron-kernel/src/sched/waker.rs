//! Waker implementation for kernel tasks.
//!
//! Packs a [`TaskId`], [`Priority`], and CPU ID into the `RawWaker` data
//! pointer. When woken, the task ID is pushed onto the correct priority
//! queue in the executor.
//!
//! Encoding (64-bit data pointer, SMP-forward-compatible):
//! - Bits 63-62: Priority (2 bits, 3 levels used)
//! - Bits 61-56: CPU ID (6 bits, supports up to 64 CPUs; 0 for now)
//! - Bits 55-0:  TaskId (56 bits)

use core::task::{RawWaker, RawWakerVTable, Waker};

use hadron_core::task::{Priority, TaskId};

/// Mask for the 56-bit task ID field (bits 55-0).
const ID_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;

/// Bit position of the CPU ID field.
const CPU_SHIFT: u32 = 56;

/// Mask for the 6-bit CPU ID field (bits 61-56), after shifting.
#[allow(dead_code)] // Phase 12: used for cross-CPU wakeup routing
const CPU_MASK: u64 = 0x3F;

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

/// Creates a [`Waker`] that will re-queue the given task at the given
/// priority when woken.
pub fn task_waker(id: TaskId, priority: Priority) -> Waker {
    // SAFETY: Our vtable correctly handles the packed data pointer.
    unsafe { Waker::from_raw(raw_waker(id, priority)) }
}

fn pack(id: TaskId, priority: Priority) -> *const () {
    // CPU ID is 0 for now (BSP-only); Phase 12 will pass the actual CPU index.
    let cpu_id: u64 = 0;
    let packed = ((priority as u64) << 62) | (cpu_id << CPU_SHIFT) | (id.0 & ID_MASK);
    packed as *const ()
}

fn unpack(data: *const ()) -> (TaskId, Priority) {
    let raw = data as u64;
    let priority = Priority::from_u8((raw >> 62) as u8);
    // CPU ID (bits 61-56) is available via: (raw >> CPU_SHIFT) & CPU_MASK
    let id = TaskId(raw & ID_MASK);
    (id, priority)
}

fn raw_waker(id: TaskId, priority: Priority) -> RawWaker {
    RawWaker::new(pack(id, priority), &VTABLE)
}

fn clone(data: *const ()) -> RawWaker {
    RawWaker::new(data, &VTABLE)
}

fn wake(data: *const ()) {
    wake_by_ref(data);
}

fn wake_by_ref(data: *const ()) {
    let (id, priority) = unpack(data);
    super::executor().ready_queues.lock().push(priority, id);
}

fn drop_waker(_data: *const ()) {
    // No-op — packed data is Copy, no allocation to free.
}
