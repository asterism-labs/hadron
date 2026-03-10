//! Boot information re-exports and boot-related types.

pub use hadron_boot_info::BootInfo;

/// Boot information trait for platform-agnostic access.
///
/// The concrete `BootInfo` struct implements this trait so that
/// `arch::platform_init` can accept it generically.
pub trait BootInfoAccess {
    /// Returns the physical address of the ACPI RSDP.
    fn rsdp_address(&self) -> u64;
}

impl BootInfoAccess for BootInfo {
    fn rsdp_address(&self) -> u64 {
        self.rsdp_phys
    }
}

/// SMP CPU entry descriptor (stub for smp.rs references).
#[allow(dead_code)] // Phase 2: used by SMP bringup
#[derive(Debug, Clone, Copy)]
pub struct SmpCpuEntry {
    /// APIC ID of the processor.
    pub apic_id: u32,
    /// Entry point address for the AP.
    pub goto_address: u64,
}
