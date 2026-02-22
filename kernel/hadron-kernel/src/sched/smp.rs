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

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use crate::arch::x86_64::hw::local_apic::LocalApic;
use crate::id::CpuId;
use crate::percpu::MAX_CPUS;
use crate::task::Priority;

use crate::arch::x86_64::interrupts::dispatch::vectors;

/// IPI vector used to wake a CPU from HLT.
const IPI_WAKE_VECTOR: crate::id::HwIrqVector = vectors::IPI_START; // 240

/// Global flag set by the panic handler to signal all CPUs to halt.
///
/// Checked by the NMI handler and the executor idle loop.
pub static PANIC_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

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
pub fn register_cpu_apic_id(cpu_id: CpuId, apic_id: u8) {
    CPU_APIC_IDS[cpu_id.as_u32() as usize].store(apic_id, Ordering::Release);
}

/// Initializes the IPI wakeup vector handler and registers the wake IPI callback.
///
/// Must be called before APs enter their executor loops.
pub fn init() {
    crate::arch::x86_64::interrupts::dispatch::register_handler(IPI_WAKE_VECTOR, ipi_wake_handler)
        .expect("Failed to register IPI wake vector");

    // Register the wake IPI callback so hadron-sched's waker can send
    // cross-CPU IPIs without depending on arch code directly.
    hadron_sched::waker::set_wake_ipi_fn(send_wake_ipi);
}

/// IPI wakeup handler — intentionally empty.
///
/// The interrupt itself breaks the target CPU out of `enable_and_hlt()`.
/// No additional work is needed here because the task was already pushed
/// into the target executor's ready queue before the IPI was sent.
fn ipi_wake_handler(_vector: crate::id::IrqVector) {}

/// Sends a wakeup IPI to the specified CPU.
///
/// The target CPU will exit `enable_and_hlt()`, re-enter the executor loop,
/// and poll any newly enqueued tasks.
pub fn send_wake_ipi(target_cpu: CpuId) {
    let target_apic_id = CPU_APIC_IDS[target_cpu.as_u32() as usize].load(Ordering::Acquire);
    if let Some(lapic_virt) = crate::arch::x86_64::acpi::Acpi::lapic_virt() {
        // SAFETY: The LAPIC is mapped and permanent. The target APIC ID was
        // registered during bootstrap. IPI_WAKE_VECTOR has a registered
        // handler (no-op) so the interrupt will be handled normally.
        let lapic = unsafe { LocalApic::new(lapic_virt) };
        unsafe { lapic.send_ipi(target_apic_id, IPI_WAKE_VECTOR.as_irq_vector()) };
    }
}

/// Type alias matching the signature expected by `Executor::run`.
type StealResult = (
    crate::task::TaskId,
    Priority,
    hadron_sched::executor::TaskEntry,
);

/// Attempts to steal one task from another CPU's executor.
///
/// Iterates over other CPUs starting from a pseudo-random offset (to avoid
/// thundering herd). Uses `try_lock` to avoid blocking victims. Returns the
/// stolen task's ID, priority, and full entry (future + metadata).
///
/// The caller must insert the stolen entry into their local executor's task
/// map and ready queue.
pub(crate) fn try_steal() -> Option<StealResult> {
    #[cfg(hadron_no_work_steal)]
    return None;

    #[cfg(not(hadron_no_work_steal))]
    {
        let local_cpu = crate::percpu::PerCpuState::current().get_cpu_id();
        let cpu_count = crate::percpu::PerCpuState::cpu_count();
        if cpu_count <= 1 {
            return None;
        }

        // Pseudo-random start offset to distribute stealing pressure.
        let start = (crate::time::Time::timer_ticks() as u32) % cpu_count;

        for i in 1..cpu_count {
            let target = CpuId::new((start + i) % cpu_count);
            if target == local_cpu {
                continue;
            }

            if let Some(stolen) = hadron_sched::executor::for_cpu(target).steal_task() {
                return Some(stolen);
            }
        }

        None
    }
}

/// Halts all other CPUs by sending a broadcast NMI.
///
/// Called from the panic handler. Sets [`PANIC_IN_PROGRESS`] and sends an
/// NMI to all-excluding-self via the Local APIC. The NMI handler on each
/// remote CPU checks the flag and enters `cli; hlt`.
///
/// If the LAPIC is not yet initialized (early boot panic), this is a no-op
/// — there are no other CPUs running yet.
pub fn panic_halt_other_cpus() {
    // Ensure only the first panicking CPU sends the broadcast.
    if PANIC_IN_PROGRESS.swap(true, Ordering::SeqCst) {
        // Another CPU already initiated the halt — just stop ourselves.
        loop {
            unsafe {
                core::arch::asm!("cli; hlt", options(nomem, nostack, preserves_flags));
            }
        }
    }

    if let Some(lapic_virt) = crate::arch::x86_64::acpi::Acpi::lapic_virt() {
        // SAFETY: The LAPIC is mapped and permanent. NMI delivery is always
        // safe — it cannot be masked, so all CPUs will receive it.
        let lapic = unsafe { LocalApic::new(lapic_virt) };
        unsafe { lapic.send_broadcast_nmi() };
    }
}

/// Called from the NMI handler to check if the NMI was a panic halt signal.
///
/// Returns `true` if a panic is in progress and the caller should halt.
/// The caller should enter `cli; hlt` when this returns `true`.
pub fn is_panic_halt() -> bool {
    PANIC_IN_PROGRESS.load(Ordering::SeqCst)
}
