//! Response structures for the Limine boot protocol.
//!
//! This module contains all response structures that the Limine bootloader fills in
//! after processing the corresponding requests. Each response contains the information
//! requested by the kernel.
//!
//! # Response Access
//!
//! Responses are accessed through the `.response()` method on request structures,
//! which returns an `Option<&ResponseType>`. The response will be `None` if the
//! bootloader did not fill in the response (either because the request was not
//! recognized or because the requested feature is not available).
//!
//! # Example
//!
//! ```no_run
//! use limine::MemMapRequest;
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MEMMAP_REQUEST: MemMapRequest = MemMapRequest::new();
//!
//! fn process_memory_map() {
//!     if let Some(response) = MEMMAP_REQUEST.response() {
//!         println!("Found {} memory map entries", response.entry_count);
//!         for entry in response.entries() {
//!             // Process each entry
//!         }
//!     }
//! }
//! ```

use core::{cell::UnsafeCell, ffi::c_char, ptr::NonNull};

use crate::{
    file::{File, FileIter},
    framebuffer::{FramebufferIter, RawFramebuffer},
    memmap::{MemMapEntry, MemMapIter},
    mp::MpInfo,
    paging::PagingMode,
};

/// A wrapper around a response pointer that may be null.
///
/// Used internally to safely handle optional responses from the bootloader.
#[repr(transparent)]
pub(crate) struct Response<T> {
    inner: UnsafeCell<Option<NonNull<T>>>,
}

// SAFETY: Responses are written by the bootloader before the kernel starts and are
// only read (never written) afterward. No concurrent mutation occurs during kernel execution.
unsafe impl<T> Sync for Response<T> {}

impl<T> Response<T> {
    /// Creates an empty response.
    pub const fn empty() -> Self {
        Self {
            inner: UnsafeCell::new(None),
        }
    }

    /// Gets a reference to the response data, if present.
    pub fn get(&self) -> Option<&T> {
        // SAFETY: The bootloader writes to this cell before control is passed to the kernel.
        // After boot, this is only read and never written, so no data races can occur.
        let inner = unsafe { self.inner.as_ref_unchecked() };
        // SAFETY: The NonNull pointer, if present, was set by the bootloader and points to
        // a valid response structure that lives for the lifetime of the kernel.
        inner.as_ref().map(|ptr| unsafe { ptr.as_ref() })
    }
}

/// The response structure for the Bootloader Info Request.
#[repr(C)]
pub struct BootloaderInfoResponse {
    /// The revision of this response structure.
    pub revision: u64,
    name: *const c_char,
    version: *const c_char,
}

impl BootloaderInfoResponse {
    /// Gets the bootloader name as a string slice.
    ///
    /// Returns `"Unknown"` if the name pointer is null.
    ///
    /// # Panics
    ///
    /// Panics if the bootloader name is not valid UTF-8.
    #[must_use]
    pub fn name(&self) -> &str {
        if self.name.is_null() {
            return "Unknown";
        }
        // SAFETY: The bootloader provides a valid null-terminated C string for the name.
        let c_str = unsafe { core::ffi::CStr::from_ptr(self.name) };
        c_str.to_str().expect("bootloader name is not valid utf-8")
    }

    /// Gets the bootloader version as a string slice.
    ///
    /// Returns `"Unknown"` if the version pointer is null.
    ///
    /// # Panics
    ///
    /// Panics if the bootloader version is not valid UTF-8.
    #[must_use]
    pub fn version(&self) -> &str {
        if self.version.is_null() {
            return "Unknown";
        }
        // SAFETY: The bootloader provides a valid null-terminated C string for the version.
        let c_str = unsafe { core::ffi::CStr::from_ptr(self.version) };
        c_str
            .to_str()
            .expect("bootloader version is not valid utf-8")
    }
}

/// The response structure for the Executable Command Line Request.
#[repr(C)]
pub struct ExecutableCmdlineResponse {
    /// The revision of this response structure.
    pub revision: u64,
    cmdline: *const c_char,
}

impl ExecutableCmdlineResponse {
    /// Gets the command line as a string slice.
    ///
    /// # Panics
    ///
    /// Panics if the command line is not valid UTF-8.
    #[must_use]
    pub fn cmdline(&self) -> &str {
        // SAFETY: The bootloader provides a valid null-terminated C string for the cmdline.
        let c_str = unsafe { core::ffi::CStr::from_ptr(self.cmdline) };
        c_str.to_str().expect("cmdline is not valid utf-8")
    }
}

/// The type of firmware used to boot the system.
#[non_exhaustive]
#[repr(u64)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirmwareType {
    /// X86 BIOS firmware.
    Bios = 0,
    /// UEFI 32-bit firmware.
    Efi32 = 1,
    /// UEFI 64-bit firmware.
    Efi64 = 2,
    /// ARM 64-bit firmware with SBI.
    Sbi = 3,
}

/// The response structure for the Firmware Type Request.
#[repr(C)]
pub struct FirmwareTypeResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The type of firmware used to boot the system.
    pub firmware_type: FirmwareType,
}

/// The response structure for the Stack Size Request.
#[repr(C)]
pub struct StackSizeResponse {
    /// The revision of this response structure.
    pub revision: u64,
}

/// The response structure for the Higher Half Direct Mapping Request.
#[repr(C)]
pub struct HhdmResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Virtual base address of the Higher Half Direct Mapping (unless revision == 3, then
    /// physical).
    pub hhdm_base: u64,
}

/// The response structure for the Framebuffer Request.
#[repr(C)]
pub struct FramebufferResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The number of framebuffers available.
    pub framebuffer_count: u64,
    framebuffers: NonNull<NonNull<RawFramebuffer>>,
}

impl FramebufferResponse {
    /// Returns an iterator over the available framebuffers.
    #[must_use]
    pub fn framebuffers(&self) -> FramebufferIter<'_> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        FramebufferIter::new(
            self.revision,
            self.framebuffer_count as usize,
            self.framebuffers,
        )
    }
}

/// The response structure for the Paging Mode Request.
#[repr(C)]
pub struct PagingModeResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The paging mode selected by the bootloader.
    pub paging_mode: PagingMode,
}

/// The response structure for the Multiprocessor Information Request.
#[repr(C)]
pub struct MpResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Bitflags indicating CPU features and capabilities.
    ///
    /// Bit0: x2APIC has been enabled (x86 only)
    pub flags: u64,
    /// The Local APIC ID of the bootstrap processor (`x86_64` only).
    #[cfg(target_arch = "x86_64")]
    pub bsp_lapic_id: u32,
    /// The MPIDR value of the bootstrap processor (`AArch64` only).
    #[cfg(target_arch = "aarch64")]
    pub bsp_mpidr: u64,
    /// The Hart ID of the bootstrap processor (RISC-V only).
    #[cfg(target_arch = "riscv64")]
    pub bsp_hartid: u64,
    /// The number of CPUs detected in the system.
    pub cpu_count: u64,
    cpus: NonNull<NonNull<MpInfo>>,
}

impl MpResponse {
    /// Returns an iterator over the CPU information entries.
    pub fn cpus(&self) -> impl Iterator<Item = &'static MpInfo> {
        // SAFETY: The bootloader provides a valid pointer to an array of `cpu_count`
        // NonNull<MpInfo> pointers, all pointing to valid MpInfo structures.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        let cpus_slice =
            unsafe { core::slice::from_raw_parts(self.cpus.as_ptr(), self.cpu_count as usize) };
        // SAFETY: Each NonNull<MpInfo> in the slice points to a valid, static MpInfo structure
        // provided by the bootloader.
        cpus_slice.iter().map(|ptr| unsafe { ptr.as_ref() })
    }
}

/// RISC-V-specific response structure for retrieving the BSP hart ID.
#[repr(C)]
pub struct BspHartIdResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The Hart ID of the bootstrap processor.
    pub hart_id: u64,
}

/// Response structure containing the system memory map.
///
/// This response provides a list of memory regions describing the physical memory
/// layout of the system, including usable RAM, reserved regions, and special-purpose
/// memory areas.
#[repr(C)]
pub struct MemMapResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The number of memory map entries.
    pub entry_count: u64,
    entries: NonNull<NonNull<MemMapEntry>>,
}

impl MemMapResponse {
    /// Returns an iterator over the memory map entries.
    #[must_use]
    pub fn entries(&self) -> MemMapIter<'_> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        MemMapIter::new(self.entry_count as usize, self.entries)
    }
}

/// The response structure for the Entry Point Request.
#[repr(C)]
pub struct EntryPointResponse {
    /// The revision of this response structure.
    pub revision: u64,
}

/// The response structure for the Executable File Request.
#[repr(C)]
pub struct ExecutableFileResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Reference to the executable file loaded by the bootloader.
    pub file: &'static File,
}

/// The response structure for the Module Files Request.
#[repr(C)]
pub struct ModuleResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The number of modules loaded.
    pub module_count: u64,
    modules: NonNull<NonNull<File>>,
}

impl ModuleResponse {
    /// Returns an iterator over the module files.
    #[must_use]
    pub fn modules(&self) -> FileIter<'_> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        FileIter::new(self.modules, self.module_count as usize)
    }
}

/// The response structure for the RSDP Request.
#[repr(C)]
pub struct RsdpResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Virtual address of the RSDP structure if `base_revision` != 3.
    pub rsdp_addr: u64,
}

/// The response structure for the SMBIOS Request.
#[repr(C)]
pub struct SmbiosResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Address of the 32-bit SMBIOS entry point, or 0 if not available.
    pub entry_32_addr: u32,
    /// Address of the 64-bit SMBIOS entry point, or 0 if not available.
    pub entry_64_addr: u64,
}

/// The response structure for the EFI System Table Request.
#[repr(C)]
pub struct EfiSystemTableResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Physical address of the EFI System Table if `base_revision` >= 3.
    pub system_table_addr: u64,
}

/// The response structure for the EFI Memory Map Request.
#[repr(C)]
pub struct EfiMemoryMapResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Pointer to the EFI memory map data.
    pub memory_map: NonNull<u8>,
    /// Total size of the memory map in bytes.
    pub memory_map_size: u64,
    /// Size of each memory descriptor entry.
    pub descriptor_size: u64,
    /// Version of the memory descriptor format.
    pub descriptor_version: u32,
}

/// The response structure for the Date at Boot Request.
#[repr(C)]
pub struct DateAtBootResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// UNIX timestamp representing the date at boot time.
    pub timestamp: i64,
}

/// The response structure for the Executable Address Request.
#[repr(C)]
pub struct ExecutableAddressResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Physical base address where the kernel executable was loaded.
    pub phys_base: u64,
    /// Virtual base address where the kernel executable was mapped.
    pub virt_base: u64,
}

/// The response structure for the Device Tree Blob Request.
#[repr(C)]
pub struct DeviceTreeBlobResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Virtual address of the Device Tree Blob.
    pub dtb_addr: u64,
}

/// The response structure for the Bootloader Performance Request.
#[repr(C)]
pub struct BootloaderPerformanceResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Time of system reset in microseconds.
    pub reset_us: u64,
    /// Time of bootloader initialization start in microseconds.
    pub init_us: u64,
    /// Time of handoff to the kernel in microseconds.
    pub exec_us: u64,
}
