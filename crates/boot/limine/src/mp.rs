//! Module defining the multiprocessor information structure for different architectures.
//!
//! This module provides types for working with multiprocessor systems. The [`MpInfo`]
//! structure contains information about each CPU/core in the system and provides a
//! mechanism to start application processors (APs).
//!
//! # Overview
//!
//! The MP (multiprocessor) feature allows kernels to:
//! - Enumerate all CPUs/cores in the system
//! - Identify the bootstrap processor (BSP)
//! - Start and configure application processors (APs)
//! - Pass data to each CPU during startup
//!
//! # Architecture-Specific Information
//!
//! ## `x86_64`
//! Each CPU is identified by its Local APIC ID. The `goto_address` field can be
//! atomically written to start an AP, with the `MpInfo` pointer passed in RDI.
//!
//! ## `AArch64`
//! Each CPU is identified by its MPIDR (Multiprocessor Affinity Register) value.
//!
//! ## RISC-V
//! Each CPU is identified by its Hart ID. Harts (hardware threads) are the RISC-V
//! equivalent of CPU cores.
//!
//! # Starting Application Processors
//!
//! To start an AP, atomically write the entry point address to the `goto_address`
//! field. The AP will jump to that address with a fresh stack and receive a pointer
//! to its `MpInfo` structure.
//!
//! # Example
//!
//! ```no_run
//! use limine::MpRequest;
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MP_REQUEST: MpRequest = MpRequest::new(0);
//!
//! fn start_application_processors() {
//!     if let Some(mp_response) = MP_REQUEST.response() {
//!         println!("Found {} CPUs", mp_response.cpu_count);
//!
//!         // Start each AP by writing to goto_address
//!         for cpu_info in mp_response.cpus() {
//!             // Write entry point to start the AP
//!             // (implementation depends on synchronization requirements)
//!         }
//!     }
//! }
//! ```

/// Multiprocessor Information Structure for `x86_64` architecture.
pub mod x86_64 {
    /// Function pointer type for the `goto_address` field.
    pub type GotoAddress = fn(*const MpInfo);

    /// Multiprocessor Information Structure (`x86_64`).
    #[repr(C)]
    pub struct MpInfo {
        /// A unique identifier for the processor.
        pub processor_id: u32,
        /// The Local APIC ID of the processor.
        pub lapic_id: u32,
        /// Reserved field for alignment.
        _reserved: u64,
        /// An atomic write to this field causes the parked CPU to jump to the written address,
        /// on a 64KiB (or Stack Size feature size) stack. A pointer to the `limine_mp_info`
        /// structure of the CPU is passed in RDI. Other than that, the CPU state will be the same
        /// as described for the bootstrap processor. This field is unused for the structure
        /// describing the bootstrap processor. For all CPUs, this field is guaranteed to be NULL
        /// when control is first passed to the bootstrap processor.
        pub goto_address: GotoAddress,
        /// An extra argument that will be passed in RSI to the `goto_address` function.
        pub extra_argument: u64,
    }
}

/// Multiprocessor Information Structure for `AArch64` architecture.
pub mod aarch64 {
    /// Function pointer type for the `goto_address` field.
    pub type GotoAddress = fn(*const MpInfo);

    /// Multiprocessor Information Structure (`AArch64`).
    #[repr(C)]
    pub struct MpInfo {
        /// A unique identifier for the processor.
        pub processor_id: u32,
        /// Reserved field for alignment.
        _reserved1: u32,
        /// The MPIDR value of the processor.
        pub mpidr: u64,
        /// Reserved field for alignment.
        _reserved2: u64,
        /// An atomic write to this field causes the parked CPU to jump to the written address,
        /// on a 64KiB (or Stack Size feature size) stack. A pointer to the `limine_mp_info`
        /// structure of the CPU is passed in RDI. Other than that, the CPU state will be the same
        /// as described for the bootstrap processor. This field is unused for the structure
        /// describing the bootstrap processor. For all CPUs, this field is guaranteed to be NULL
        /// when control is first passed to the bootstrap processor.
        pub goto_address: GotoAddress,
        /// An extra argument that will be passed in RSI to the `goto_address` function.
        pub extra_argument: u64,
    }
}

/// Multiprocessor Information Structure for RISC-V architecture.
pub mod riscv {
    /// Function pointer type for the `goto_address` field.
    pub type GotoAddress = fn(*const MpInfo);

    /// Multiprocessor Information Structure (RISC-V).
    #[repr(C)]
    pub struct MpInfo {
        /// A unique identifier for the processor.
        pub processor_id: u64,
        /// The Hart ID of the processor.
        pub hart_id: u64,
        /// Reserved field for alignment.
        _reserved: u64,
        /// An atomic write to this field causes the parked CPU to jump to the written address,
        /// on a 64KiB (or Stack Size feature size) stack. A pointer to the `limine_mp_info`
        /// structure of the CPU is passed in RDI. Other than that, the CPU state will be the same
        /// as described for the bootstrap processor. This field is unused for the structure
        /// describing the bootstrap processor. For all CPUs, this field is guaranteed to be NULL
        /// when control is first passed to the bootstrap processor.
        pub goto_address: GotoAddress,
        /// An extra argument that will be passed in RSI to the `goto_address` function.
        pub extra_argument: u64,
    }
}

#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(target_arch = "riscv64")]
pub use riscv::*;
