//! Waker implementation for kernel tasks.
//!
//! Packs a [`TaskId`], [`Priority`], and CPU ID into the `RawWaker` data
//! pointer. When woken, the task ID is pushed onto the correct priority
//! queue in the **originating CPU's** executor — not the current CPU's.
//!
//! Encoding (64-bit data pointer):
//! - Bits 63-62: Priority (2 bits, 3 levels used)
//! - Bits 61-56: CPU ID (6 bits, supports up to 64 CPUs)
//! - Bits 55-0:  TaskId (56 bits)

use core::task::{RawWaker, RawWakerVTable, Waker};

use crate::id::CpuId;
use crate::task::{Priority, TaskId};

/// Mask for the 56-bit task ID field (bits 55-0).
const ID_MASK: u64 = 0x00FF_FFFF_FFFF_FFFF;

/// Bit position of the CPU ID field.
const CPU_SHIFT: u32 = 56;

/// Mask for the 6-bit CPU ID field (bits 61-56), after shifting.
const CPU_MASK: u64 = 0x3F;

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake_by_ref, drop_waker);

/// Creates a [`Waker`] that will re-queue the given task at the given
/// priority on the current CPU's executor when woken.
pub fn task_waker(id: TaskId, priority: Priority) -> Waker {
    // SAFETY: Our vtable correctly handles the packed data pointer.
    unsafe { Waker::from_raw(raw_waker(id, priority)) }
}

fn pack(id: TaskId, priority: Priority) -> *const () {
    let cpu_id = crate::percpu::current_cpu().get_cpu_id().as_u32() as u64;
    let packed = ((priority as u64) << 62) | (cpu_id << CPU_SHIFT) | (id.0 & ID_MASK);
    packed as *const ()
}

fn unpack(data: *const ()) -> (TaskId, Priority, CpuId) {
    let raw = data as u64;
    let priority = Priority::from_u8((raw >> 62) as u8);
    let cpu_id = CpuId::new(((raw >> CPU_SHIFT) & CPU_MASK) as u32);
    let id = TaskId(raw & ID_MASK);
    (id, priority, cpu_id)
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
    let (id, priority, target_cpu) = unpack(data);
    // Push to the target CPU's executor ready queue.
    // IrqSpinLock is interrupt-safe and the target executor is always
    // initialized (LazyLock), so cross-CPU pushes are safe.
    super::executor::for_cpu(target_cpu)
        .ready_queues
        .lock()
        .push(priority, id);

    // If the target is a different CPU, send an IPI to wake it from HLT
    // so it processes the newly enqueued task promptly.
    let current = crate::percpu::current_cpu().get_cpu_id();
    if target_cpu != current {
        super::smp::send_wake_ipi(target_cpu);
    }
}

fn drop_waker(_data: *const ()) {
    // No-op — packed data is Copy, no allocation to free.
}
