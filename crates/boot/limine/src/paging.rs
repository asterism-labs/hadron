//! Defines the paging modes for different architectures.
//!
//! This module provides the [`PagingMode`] enum which describes the different paging
//! modes available on various architectures. Paging modes determine how virtual addresses
//! are translated to physical addresses and how much virtual address space is available.
//!
//! # Architecture Support
//!
//! ## `x86_64`
//! - **4-Level Paging**: Provides 48-bit virtual address space (256 TiB)
//! - **5-Level Paging**: Provides 57-bit virtual address space (128 PiB)
//!
//! ## `AArch64`
//! - **4-Level Paging**: Standard paging mode
//! - **5-Level Paging**: Extended paging mode
//!
//! ## RISC-V
//! - **Sv39**: 39-bit virtual address space (512 GiB)
//! - **Sv48**: 48-bit virtual address space (256 TiB)
//! - **Sv57**: 57-bit virtual address space (128 PiB)
//!
//! ## `LoongArch64`
//! - **4-Level Paging**: Standard paging mode
//!
//! # Example
//!
//! ```no_run
//! use limine::{PagingModeRequest, paging::PagingMode};
//!
//! #[used]
//! #[link_section = ".requests"]
//! static PAGING_REQUEST: PagingModeRequest = PagingModeRequest::new(
//!     PagingMode::Paging4Level,  // Preferred mode
//!     PagingMode::Paging4Level,  // Minimum acceptable
//!     PagingMode::Paging5Level,  // Maximum acceptable
//! );
//! ```

/// Paging modes supported by various architectures.
///
/// This enum is non-exhaustive to allow for future extensions.
#[non_exhaustive]
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingMode {
    /// 4-level paging providing a 48-bit virtual address space (256 TiB).
    #[cfg(target_arch = "x86_64")]
    Paging4Level = 0,
    /// 5-level paging providing a 57-bit virtual address space (128 PiB).
    #[cfg(target_arch = "x86_64")]
    Paging5Level = 1,

    /// 4-level paging (standard `AArch64` paging mode).
    #[cfg(target_arch = "aarch64")]
    Paging4Level = 0,
    /// 5-level paging (extended `AArch64` paging mode).
    #[cfg(target_arch = "aarch64")]
    Paging5Level = 1,

    /// Sv39 paging providing a 39-bit virtual address space (512 GiB).
    #[cfg(target_arch = "riscv64")]
    RiscvSv39 = 0,
    /// Sv48 paging providing a 48-bit virtual address space (256 TiB).
    #[cfg(target_arch = "riscv64")]
    RiscvSv48 = 1,
    /// Sv57 paging providing a 57-bit virtual address space (128 PiB).
    #[cfg(target_arch = "riscv64")]
    RiscvSv57 = 2,

    /// 4-level paging (standard `LoongArch64` paging mode).
    #[cfg(target_arch = "loongarch64")]
    Paging4Level = 0,
}
