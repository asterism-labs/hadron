//! Cross-CPU task wakeup via IPI and work stealing.
//!
//! **IPI Wakeup**: When a waker fires for a task on a different CPU, the task
//! ID is pushed into the remote executor's ready queue and an IPI is sent to
//! wake that CPU from HLT.
//!
//! **Work Stealing**: When a CPU's executor is idle, it attempts to steal a
//! task from another CPU's executor. The stolen task's entire entry (future +
//! metadata) is migrated to the stealer's executor. On the next poll, the
//! waker will encode the stealer's CPU ID, completing the migration.

use core::sync::atomic::{AtomicU8, Ordering};

use crate::percpu::MAX_CPUS;
use crate::task::Priority;
use crate::arch::x86_64::hw::local_apic::LocalApic;

use super::executor::TaskEntry;
use crate::arch::x86_64::interrupts::dispatch::vectors;

/// IPI vector used to wake a CPU from HLT.
const IPI_WAKE_VECTOR: u8 = vectors::IPI_START; // 240

/// CPU ID → APIC ID mapping, populated during bootstrap.
///
/// Indexed by logical CPU ID (0 = BSP). Used by [`send_wake_ipi`] to
/// translate the target CPU ID to the physical APIC ID needed for IPI delivery.
static CPU_APIC_IDS: [AtomicU8; MAX_CPUS] = {
    const INIT: AtomicU8 = AtomicU8::new(0);
    [INIT; MAX_CPUS]
};

/// Registers a CPU's APIC ID for IPI routing.
///
/// Called during BSP ACPI init (cpu_id=0) and AP bootstrap (cpu_id=1+).
pub fn register_cpu_apic_id(cpu_id: u32, apic_id: u8) {
    CPU_APIC_IDS[cpu_id as usize].store(apic_id, Ordering::Release);
}

/// Initializes the IPI wakeup vector handler.
///
/// Must be called before APs enter their executor loops.
pub fn init() {
    crate::arch::x86_64::interrupts::dispatch::register_handler(IPI_WAKE_VECTOR, ipi_wake_handler)
        .expect("Failed to register IPI wake vector");
}

/// IPI wakeup handler — intentionally empty.
///
/// The interrupt itself breaks the target CPU out of `enable_and_hlt()`.
/// No additional work is needed here because the task was already pushed
/// into the target executor's ready queue before the IPI was sent.
fn ipi_wake_handler(_vector: u8) {}

/// Sends a wakeup IPI to the specified CPU.
///
/// The target CPU will exit `enable_and_hlt()`, re-enter the executor loop,
/// and poll any newly enqueued tasks.
pub fn send_wake_ipi(target_cpu: u32) {
    let target_apic_id = CPU_APIC_IDS[target_cpu as usize].load(Ordering::Acquire);
    if let Some(lapic_virt) = crate::arch::x86_64::acpi::lapic_virt() {
        // SAFETY: The LAPIC is mapped and permanent. The target APIC ID was
        // registered during bootstrap. IPI_WAKE_VECTOR has a registered
        // handler (no-op) so the interrupt will be handled normally.
        let lapic = unsafe { LocalApic::new(lapic_virt) };
        unsafe { lapic.send_ipi(target_apic_id, IPI_WAKE_VECTOR) };
    }
}

/// Attempts to steal one task from another CPU's executor.
///
/// Iterates over other CPUs starting from a pseudo-random offset (to avoid
/// thundering herd). Uses `try_lock` to avoid blocking victims. Returns the
/// stolen task's ID, priority, and full entry (future + metadata).
///
/// The caller must insert the stolen entry into their local executor's task
/// map and ready queue.
pub(crate) fn try_steal() -> Option<(crate::task::TaskId, Priority, TaskEntry)> {
    let local_cpu = crate::percpu::current_cpu().get_cpu_id();
    let cpu_count = crate::percpu::cpu_count();
    if cpu_count <= 1 {
        return None;
    }

    // Pseudo-random start offset to distribute stealing pressure.
    let start = (crate::arch::x86_64::acpi::timer_ticks() as u32) % cpu_count;

    for i in 1..cpu_count {
        let target = (start + i) % cpu_count;
        if target == local_cpu {
            continue;
        }

        if let Some(stolen) = super::executor::for_cpu(target).steal_task() {
            return Some(stolen);
        }
    }

    None
}
