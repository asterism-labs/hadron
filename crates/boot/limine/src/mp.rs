//! Multiprocessor request flags, response and per-CPU information structures.
//!
//! The MP feature lets the kernel enumerate every CPU, identify the bootstrap processor
//! (BSP) and start the application processors (APs). The bootloader starts the APs only
//! when an [`MpRequest`](crate::MpRequest) is present, and parks each one until the kernel
//! releases it.
//!
//! The response and per-CPU layouts differ between architectures, so each architecture
//! has its own module. The module for the target architecture is re-exported here.
//!
//! # Starting application processors
//!
//! Call [`MpInfo::start`] with an entry point and an extra argument. It stores the
//! argument, then publishes the entry point with release ordering, as the spec requires.
//! The AP jumps to the entry point on its own stack, sized by the Stack Size feature,
//! with a pointer to its `MpInfo` as the first argument. Calling `start` on the BSP's
//! entry has no effect.
//!
//! # Example
//!
//! ```no_run
//! use limine::{MpRequest, mp::{MpInfo, MpRequestFlags}};
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MP_REQUEST: MpRequest = MpRequest::new(MpRequestFlags::empty());
//!
//! extern "C" fn ap_main(info: &'static MpInfo) -> ! {
//!     loop {}
//! }
//!
//! fn start_application_processors() {
//!     if let Some(mp_response) = MP_REQUEST.response() {
//!         for cpu in mp_response.cpus() {
//!             cpu.start(ap_main, 0);
//!         }
//!     }
//! }
//! ```

bitflags::bitflags! {
    /// Flags for [`MpRequest`](crate::MpRequest).
    #[repr(transparent)]
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct MpRequestFlags: u64 {
        /// Enable x2APIC mode, if possible (`x86_64` only).
        ///
        /// If firmware already enabled x2APIC and this bit is clear, the bootloader tries
        /// to disable x2APIC, and fails to boot if it cannot.
        const X86_64_X2APIC = 1 << 0;
    }
}

/// Generates the methods shared by every architecture's `MpInfo` and `MpResponse`.
///
/// Each architecture has its own struct layouts, but the goto-address handshake and the
/// CPU list are identical.
macro_rules! mp_common {
    () => {
        /// Entry point type for an application processor.
        ///
        /// The AP receives a reference to its own [`MpInfo`] and must not return.
        pub type GotoAddress = extern "C" fn(&'static MpInfo) -> !;

        impl MpInfo {
            /// Releases this application processor to run `entry`.
            ///
            /// Stores `extra_argument`, then stores `entry` into `goto_address` with
            /// release ordering. Every write made before this call is visible to the AP
            /// when it reaches `entry`. Has no effect on the BSP's entry.
            pub fn start(&self, entry: GotoAddress, extra_argument: u64) {
                self.extra_argument
                    .store(extra_argument, core::sync::atomic::Ordering::Relaxed);
                self.goto_address
                    .store(entry as *mut (), core::sync::atomic::Ordering::Release);
            }

            /// Returns the free-for-use extra argument, as set by [`Self::start`].
            #[must_use]
            pub fn extra_argument(&self) -> u64 {
                self.extra_argument
                    .load(core::sync::atomic::Ordering::Relaxed)
            }
        }

        impl MpResponse {
            /// Returns an iterator over the CPU information entries, including the BSP.
            pub fn cpus(&self) -> impl Iterator<Item = &'static MpInfo> {
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "Limine protocol fields fit in target width"
                )]
                // SAFETY: The bootloader provides a valid pointer to an array of
                // `cpu_count` non-null pointers to `MpInfo` structures.
                let cpus = unsafe {
                    core::slice::from_raw_parts(self.cpus.as_ptr(), self.cpu_count as usize)
                };
                // SAFETY: Each pointer in the array points to a valid `MpInfo` that lives
                // in bootloader-reclaimable memory for the lifetime of the kernel.
                cpus.iter().map(|ptr| unsafe { ptr.as_ref() })
            }
        }
    };
}

/// Multiprocessor structures for `x86_64`.
pub mod x86_64 {
    use core::{
        ptr::NonNull,
        sync::atomic::{AtomicPtr, AtomicU64},
    };

    bitflags::bitflags! {
        /// Flags in the `x86_64` [`MpResponse`].
        #[repr(transparent)]
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct MpResponseFlags: u32 {
            /// The bootloader enabled x2APIC mode.
            const X2APIC = 1 << 0;
        }
    }

    /// Per-CPU information (`x86_64`).
    #[repr(C)]
    pub struct MpInfo {
        /// ACPI processor UID, as given by the MADT.
        pub processor_id: u32,
        /// Local APIC ID, as given by the MADT.
        pub lapic_id: u32,
        reserved: u64,
        pub(super) goto_address: AtomicPtr<()>,
        pub(super) extra_argument: AtomicU64,
    }

    /// The response structure for the MP Request (`x86_64`).
    #[repr(C)]
    pub struct MpResponse {
        /// The revision of this response structure.
        pub revision: u64,
        /// Flags describing what the bootloader set up.
        pub flags: MpResponseFlags,
        /// The Local APIC ID of the bootstrap processor.
        pub bsp_lapic_id: u32,
        /// The number of CPUs, including the bootstrap processor.
        pub cpu_count: u64,
        pub(super) cpus: NonNull<NonNull<MpInfo>>,
    }

    mp_common!();
}

/// Multiprocessor structures for `AArch64`.
pub mod aarch64 {
    use core::{
        ptr::NonNull,
        sync::atomic::{AtomicPtr, AtomicU64},
    };

    /// Per-CPU information (`AArch64`).
    #[repr(C)]
    pub struct MpInfo {
        /// ACPI processor UID, as given by the MADT, or 0 on systems without ACPI.
        pub processor_id: u32,
        reserved1: u32,
        /// The affinity fields of `MPIDR_EL1`, with all other bits masked off.
        pub mpidr: u64,
        reserved: u64,
        pub(super) goto_address: AtomicPtr<()>,
        pub(super) extra_argument: AtomicU64,
    }

    /// The response structure for the MP Request (`AArch64`).
    #[repr(C)]
    pub struct MpResponse {
        /// The revision of this response structure.
        pub revision: u64,
        /// Always zero.
        pub flags: u64,
        /// The MPIDR of the bootstrap processor, in the same form as [`MpInfo::mpidr`].
        pub bsp_mpidr: u64,
        /// The number of CPUs, including the bootstrap processor.
        pub cpu_count: u64,
        pub(super) cpus: NonNull<NonNull<MpInfo>>,
    }

    mp_common!();
}

/// Multiprocessor structures for RISC-V.
pub mod riscv {
    use core::{
        ptr::NonNull,
        sync::atomic::{AtomicPtr, AtomicU64},
    };

    /// Per-CPU information (RISC-V).
    #[repr(C)]
    pub struct MpInfo {
        /// ACPI processor UID, as given by the MADT, or 0 on systems without ACPI.
        pub processor_id: u64,
        /// Hart ID, as given by the MADT or the device tree.
        pub hart_id: u64,
        reserved: u64,
        pub(super) goto_address: AtomicPtr<()>,
        pub(super) extra_argument: AtomicU64,
    }

    /// The response structure for the MP Request (RISC-V).
    #[repr(C)]
    pub struct MpResponse {
        /// The revision of this response structure.
        pub revision: u64,
        /// Always zero.
        pub flags: u64,
        /// The Hart ID of the bootstrap processor.
        pub bsp_hartid: u64,
        /// The number of CPUs, including the bootstrap processor.
        pub cpu_count: u64,
        pub(super) cpus: NonNull<NonNull<MpInfo>>,
    }

    mp_common!();
}

/// Multiprocessor structures for `LoongArch64`.
pub mod loongarch64 {
    use core::{
        ptr::NonNull,
        sync::atomic::{AtomicPtr, AtomicU64},
    };

    /// Per-CPU information (`LoongArch64`).
    #[repr(C)]
    pub struct MpInfo {
        /// ACPI processor UID, as given by the MADT, or 0 on systems without ACPI.
        pub processor_id: u64,
        /// Physical CPU ID, as given by the MADT or the device tree.
        pub phys_id: u64,
        reserved: u64,
        pub(super) goto_address: AtomicPtr<()>,
        pub(super) extra_argument: AtomicU64,
    }

    /// The response structure for the MP Request (`LoongArch64`).
    #[repr(C)]
    pub struct MpResponse {
        /// The revision of this response structure.
        pub revision: u64,
        /// Always zero.
        pub flags: u64,
        /// The physical CPU ID of the bootstrap processor, as read from `CSR.CPUID`.
        pub bsp_phys_id: u64,
        /// The number of CPUs, including the bootstrap processor.
        pub cpu_count: u64,
        pub(super) cpus: NonNull<NonNull<MpInfo>>,
    }

    mp_common!();
}

#[cfg(target_arch = "x86_64")]
pub use x86_64::*;

#[cfg(target_arch = "aarch64")]
pub use aarch64::*;

#[cfg(target_arch = "riscv64")]
pub use riscv::*;

#[cfg(target_arch = "loongarch64")]
pub use loongarch64::*;

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn flags_match_spec() {
        assert_eq!(MpRequestFlags::X86_64_X2APIC.bits(), 1);
        assert_eq!(x86_64::MpResponseFlags::X2APIC.bits(), 1);
    }

    #[test]
    fn x86_64_layout() {
        assert_eq!(offset_of!(x86_64::MpInfo, lapic_id), 4);
        assert_eq!(offset_of!(x86_64::MpInfo, goto_address), 16);
        assert_eq!(offset_of!(x86_64::MpInfo, extra_argument), 24);
        assert_eq!(size_of::<x86_64::MpInfo>(), 32);
        assert_eq!(offset_of!(x86_64::MpResponse, flags), 8);
        assert_eq!(offset_of!(x86_64::MpResponse, bsp_lapic_id), 12);
        assert_eq!(offset_of!(x86_64::MpResponse, cpu_count), 16);
        assert_eq!(offset_of!(x86_64::MpResponse, cpus), 24);
        assert_eq!(size_of::<x86_64::MpResponse>(), 32);
    }

    #[test]
    fn aarch64_layout() {
        assert_eq!(offset_of!(aarch64::MpInfo, mpidr), 8);
        assert_eq!(offset_of!(aarch64::MpInfo, goto_address), 24);
        assert_eq!(offset_of!(aarch64::MpInfo, extra_argument), 32);
        assert_eq!(size_of::<aarch64::MpInfo>(), 40);
        assert_eq!(offset_of!(aarch64::MpResponse, bsp_mpidr), 16);
        assert_eq!(offset_of!(aarch64::MpResponse, cpus), 32);
        assert_eq!(size_of::<aarch64::MpResponse>(), 40);
    }

    #[test]
    fn riscv_and_loongarch64_layout() {
        assert_eq!(offset_of!(riscv::MpInfo, hart_id), 8);
        assert_eq!(offset_of!(riscv::MpInfo, goto_address), 24);
        assert_eq!(size_of::<riscv::MpInfo>(), 40);
        assert_eq!(offset_of!(riscv::MpResponse, bsp_hartid), 16);
        assert_eq!(size_of::<riscv::MpResponse>(), 40);
        assert_eq!(offset_of!(loongarch64::MpInfo, phys_id), 8);
        assert_eq!(offset_of!(loongarch64::MpInfo, goto_address), 24);
        assert_eq!(size_of::<loongarch64::MpInfo>(), 40);
        assert_eq!(offset_of!(loongarch64::MpResponse, bsp_phys_id), 16);
        assert_eq!(size_of::<loongarch64::MpResponse>(), 40);
    }
}
