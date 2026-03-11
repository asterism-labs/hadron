//! TLB shootdown IPI handling.
//!
//! When a page is unmapped from an address space that may be loaded on
//! multiple CPUs, the unmapping CPU must invalidate the TLB entry on
//! all other CPUs that have loaded that address space. This module
//! implements the IPI-based shootdown protocol.

use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

/// Pending TLB shootdown request.
///
/// The initiating CPU writes the virtual address here, sends the IPI,
/// and spins until all target CPUs have acknowledged.
static SHOOTDOWN_VADDR: AtomicU64 = AtomicU64::new(0);

/// Number of CPUs that have acknowledged the shootdown.
static SHOOTDOWN_ACK: AtomicU32 = AtomicU32::new(0);

/// Number of CPUs expected to acknowledge.
static SHOOTDOWN_EXPECTED: AtomicU32 = AtomicU32::new(0);

/// Perform a TLB shootdown for a virtual address on all other CPUs.
///
/// Flushes the local TLB entry, then sends a TLB_SHOOTDOWN IPI to all
/// other CPUs and waits for acknowledgment.
#[cfg(hadron_smp)]
pub fn shootdown_page(vaddr: u64) {
    use crate::arch::x86_64::hw::local_apic::LocalApic;
    use crate::arch::x86_64::interrupts::dispatch::vectors;

    // Flush local TLB first.
    // SAFETY: invlpg is always safe for any virtual address.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) vaddr, options(nostack, preserves_flags));
    }

    let total_cpus = crate::arch::x86_64::smp::cpu_count();
    if total_cpus <= 1 {
        return;
    }

    let target_count = total_cpus - 1;

    // Set up the request.
    SHOOTDOWN_ACK.store(0, Ordering::Release);
    SHOOTDOWN_EXPECTED.store(target_count, Ordering::Release);
    SHOOTDOWN_VADDR.store(vaddr, Ordering::Release);

    // Send TLB shootdown IPI to all other CPUs.
    if let Some(lapic_virt) = crate::arch::x86_64::acpi::Acpi::lapic_virt() {
        // SAFETY: LAPIC is mapped.
        let lapic = unsafe { LocalApic::new(lapic_virt) };
        let my_cpu = hadron_core::cpu_local::current_cpu_id();

        for cpu_id in 0..total_cpus {
            if cpu_id == my_cpu {
                continue;
            }
            if let Some(lapic_id) = crate::arch::x86_64::smp::cpu_to_lapic_pub(cpu_id) {
                // SAFETY: Valid LAPIC ID, valid IPI vector.
                unsafe {
                    lapic.send_ipi(lapic_id, vectors::TLB_SHOOTDOWN_IPI.as_irq_vector());
                }
            }
        }
    }

    // Wait for all targets to acknowledge.
    while SHOOTDOWN_ACK.load(Ordering::Acquire) < target_count {
        core::hint::spin_loop();
    }
}

/// Handle a TLB shootdown IPI (called from the IPI handler).
pub fn handle_shootdown_ipi() {
    let vaddr = SHOOTDOWN_VADDR.load(Ordering::Acquire);
    if vaddr != 0 {
        // SAFETY: invlpg is always safe for any virtual address.
        unsafe {
            core::arch::asm!("invlpg [{}]", in(reg) vaddr, options(nostack, preserves_flags));
        }
    }
    SHOOTDOWN_ACK.fetch_add(1, Ordering::Release);
}
