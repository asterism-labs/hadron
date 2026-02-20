//! x86_64 architecture support.

pub mod acpi;
pub mod gdt;
pub mod hw;
pub mod idt;
pub mod instructions;
pub mod interrupts;
pub mod paging;
pub mod registers;
pub mod smp;
pub mod structures;
pub mod syscall;
pub mod userspace;

// Re-export commonly used types for ergonomic imports.
pub use instructions::port::{Port, PortRead, PortWrite, ReadOnlyPort, WriteOnlyPort};
pub use structures::machine_state::MachineState;
pub use structures::paging::{PageTable, PageTableEntry, PageTableFlags};
