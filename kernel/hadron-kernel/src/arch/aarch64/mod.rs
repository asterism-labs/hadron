//! AArch64 architecture support for the kernel (stub).

pub mod interrupts;

/// AArch64 CPU initialization (exception vectors, etc.).
pub fn cpu_init() {
    todo!("aarch64 cpu_init")
}

/// AArch64 platform initialization (device tree, interrupt controller, timers).
pub fn platform_init(_boot_info: &impl crate::boot::BootInfo) {
    todo!("aarch64 platform_init")
}

/// Spawn arch-specific async tasks for aarch64.
pub fn spawn_platform_tasks() {
    todo!("aarch64 spawn_platform_tasks")
}
