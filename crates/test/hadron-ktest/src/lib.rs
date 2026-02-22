//! Kernel test framework for Hadron OS.
//!
//! Provides the `#[kernel_test]` attribute macro and supporting types for
//! staged kernel boot testing. Tests are collected via linker sections and
//! executed at appropriate points during kernel initialization.
//!
//! # Architecture
//!
//! - **Test descriptors** are placed into the `.hadron_kernel_tests` linker
//!   section by the `#[kernel_test]` proc macro via [`linkset_entry!`].
//! - At boot, the kernel reads the section via [`kernel_test_entries()`] and
//!   runs tests matching each stage.
//! - The test runner lives in `hadron-kernel`'s `ktest` module — this crate
//!   only provides types, the linker section accessor, and serial/QEMU helpers.
//!
//! # Stages
//!
//! | Stage | Available subsystems |
//! |-------|---------------------|
//! | `early_boot` | CPU, HHDM, PMM, VMM, heap |
//! | `before_executor` | + ACPI, PCI, drivers, VFS, logging |
//! | `with_executor` | + async executor |
//! | `userspace` | + userspace process support |

#![no_std]
#![warn(missing_docs)]

extern crate alloc;

mod context;
mod descriptor;
#[doc(hidden)]
pub mod serial;

pub use context::{AsyncBarrier, TestContext};
pub use descriptor::{KernelTestDescriptor, TestKind, TestStage};

// Re-export the proc macro.
pub use hadron_ktest_macros::kernel_test;

hadron_linkset::declare_linkset! {
    /// Returns all registered kernel test descriptors from the linker section.
    pub fn kernel_test_entries() -> [KernelTestDescriptor],
    section = "hadron_kernel_tests"
}

/// QEMU exit interface for the `isa-debug-exit` device.
pub mod qemu {
    /// QEMU exit code indicating all tests passed (process exit code 33).
    pub const SUCCESS: u32 = 0x10;
    /// QEMU exit code indicating a test failure (process exit code 35).
    pub const FAILURE: u32 = 0x11;

    /// Exits QEMU via the `isa-debug-exit` device.
    ///
    /// QEMU computes the process exit code as `(value << 1) | 1`:
    /// - `0x10` → exit code 33 (success)
    /// - `0x11` → exit code 35 (failure)
    #[cfg(target_arch = "x86_64")]
    pub fn exit_qemu(code: u32) -> ! {
        // SAFETY: Writing to the isa-debug-exit I/O port causes QEMU to exit.
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") 0xf4u16,
                in("eax") code,
                options(nomem, nostack, preserves_flags),
            );
        }
        loop {
            core::hint::spin_loop();
        }
    }

    /// Exits QEMU (aarch64 stub).
    #[cfg(target_arch = "aarch64")]
    pub fn exit_qemu(_code: u32) -> ! {
        todo!("aarch64 QEMU exit")
    }
}
