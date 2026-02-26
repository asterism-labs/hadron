//! AArch64 instruction wrappers (stub).

/// Interrupt control (stub).
pub mod interrupts {
    /// Enable interrupts.
    pub fn enable() {
        todo!("aarch64 interrupts::enable")
    }

    /// Disable interrupts.
    pub fn disable() {
        todo!("aarch64 interrupts::disable")
    }
}

/// TLB management (stub).
pub mod tlb {
    use crate::addr::VirtAddr;

    /// Flush TLB entry for the given virtual address.
    pub fn flush(_addr: VirtAddr) {
        todo!("aarch64 tlb::flush")
    }
}
