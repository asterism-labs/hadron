//! Architecture-specific modules and uniform facade.

#[cfg(target_arch = "x86_64")]
pub mod x86_64;

// --- Arch facade: uniform API re-exported from the active arch ---

/// Architecture-specific CPU initialization (GDT+IDT on x86_64).
pub fn cpu_init() {
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: Called once during early BSP init. GDT and IDT are
        // statically allocated and valid.
        unsafe { x86_64::gdt::init() };
        unsafe { x86_64::idt::init() };
    }
}

/// Architecture-specific platform initialization.
///
/// Currently empty — ACPI, PCI, and driver matching are deferred to a
/// later phase when the subsystem crates are re-integrated.
pub fn platform_init() {}

/// Spawn arch-specific async tasks.
///
/// Currently empty — the executor and driver subsystem are not yet
/// integrated.
pub fn spawn_platform_tasks() {}

/// Arch-uniform interrupt facade.
pub mod interrupts {
    #[cfg(target_arch = "x86_64")]
    pub use super::x86_64::interrupts::*;
}
