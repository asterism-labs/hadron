//! Advanced Programmable Interrupt Controller (APIC) support.

pub mod io_apic;
pub mod local_apic;

pub use io_apic::IoApic;
pub use local_apic::LocalApic;
