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
    flanterm::FlantermFbInitParams,
    framebuffer::{FramebufferIter, RawFramebuffer},
    memmap::{MemMapEntry, MemMapIter},
    paging::PagingMode,
};

pub use crate::mp::MpResponse;

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
        let inner = unsafe { &*self.inner.get() };
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
    /// RISC-V SBI firmware.
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
    /// The virtual address at which the Higher Half Direct Map starts. Add it to a physical
    /// address to get the HHDM virtual address. This is always a virtual address, for
    /// every base revision.
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
    /// Address of the RSDP. This is an HHDM virtual address, except in base revision 3,
    /// where it is physical.
    pub rsdp_addr: u64,
}

/// The response structure for the SMBIOS Request.
#[repr(C)]
pub struct SmbiosResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Address of the 32-bit SMBIOS entry point, or 0 if not present. This is an HHDM
    /// virtual address, except in base revisions 3 and 4, where it is physical.
    pub entry_32_addr: u64,
    /// Address of the 64-bit SMBIOS entry point, or 0 if not present. This is an HHDM
    /// virtual address, except in base revisions 3 and 4, where it is physical.
    pub entry_64_addr: u64,
}

/// The response structure for the EFI System Table Request.
#[repr(C)]
pub struct EfiSystemTableResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Address of the EFI system table. This is an HHDM virtual address, except in base
    /// revisions 3 and 4, where it is physical.
    pub system_table_addr: u64,
}

/// The response structure for the EFI Memory Map Request.
#[repr(C)]
pub struct EfiMemoryMapResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// HHDM virtual address of the EFI memory map, in bootloader-reclaimable memory.
    /// Subtract the HHDM offset before passing it to `SetVirtualAddressMap()`.
    pub memory_map: NonNull<u8>,
    /// Total size of the memory map in bytes.
    pub memory_map_size: u64,
    /// Size of each memory descriptor in bytes.
    pub descriptor_size: u64,
    /// Version of the memory descriptor format.
    pub descriptor_version: u64,
}

/// The response structure for the Date at Boot Request.
#[repr(C)]
pub struct DateAtBootResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// UNIX timestamp, in seconds, read from the RTC at boot.
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
    /// HHDM virtual address of the Device Tree Blob, in bootloader-reclaimable memory.
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

/// Format of a TPM event log, as found in [`TpmEventLogResponse::format`].
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TpmEventLogFormat(pub u64);

impl TpmEventLogFormat {
    /// TCG 1.2 (SHA-1) event log format.
    pub const TCG_1_2: Self = Self(1);
    /// TCG 2.0 (crypto agile) event log format.
    pub const TCG_2: Self = Self(2);
}

/// The response structure for the TPM Event Log Request.
#[repr(C)]
pub struct TpmEventLogResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Format of the event log.
    pub format: TpmEventLogFormat,
    /// Size in bytes of the raw event data.
    pub size: u64,
    address: *const u8,
}

impl TpmEventLogResponse {
    /// Returns the raw event stream, in the format given by [`Self::format`].
    ///
    /// The bytes live in bootloader-reclaimable memory. The slice is empty if
    /// [`Self::size`] is 0.
    #[must_use]
    pub fn event_log(&self) -> &[u8] {
        if self.size == 0 || self.address.is_null() {
            return &[];
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        // SAFETY: For a non-zero size, the bootloader provides a valid HHDM pointer to
        // `size` bytes that stay valid for the lifetime of the kernel.
        unsafe {
            core::slice::from_raw_parts(self.address, self.size as usize)
        }
    }
}

/// The response structure for the x86-64 Keep IOMMU Request.
///
/// Its presence means the bootloader left firmware-enabled IOMMUs enabled.
#[repr(C)]
pub struct KeepIommuResponse {
    /// The revision of this response structure.
    pub revision: u64,
}

/// The response structure for the TSC Frequency Request.
#[repr(C)]
pub struct TscFrequencyResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// Frequency of the primary timestamp counter, in Hz. The bootloader measures it on a
    /// best-effort basis, so it may not be exact.
    pub frequency: u64,
}

/// The response structure for the Flanterm FB Init Params Request.
#[repr(C)]
pub struct FlantermFbInitParamsResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The number of entries. Equal to the framebuffer count of the framebuffer response.
    pub entry_count: u64,
    entries: NonNull<NonNull<FlantermFbInitParams>>,
}

impl FlantermFbInitParamsResponse {
    /// Returns an iterator over the entries.
    ///
    /// Entry `i` belongs to framebuffer `i` of the framebuffer response. A framebuffer that
    /// cannot hold a Flanterm terminal, such as one that is not 32 bits per pixel, has an
    /// all-zero entry.
    pub fn entries(&self) -> impl Iterator<Item = &FlantermFbInitParams> {
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        // SAFETY: The bootloader provides a valid pointer to an array of `entry_count`
        // non-null pointers to `FlantermFbInitParams` structures.
        let entries = unsafe {
            core::slice::from_raw_parts(self.entries.as_ptr(), self.entry_count as usize)
        };
        // SAFETY: Each pointer in the array points to a valid `FlantermFbInitParams` that
        // lives in bootloader-reclaimable memory for the lifetime of the kernel.
        entries.iter().map(|ptr| unsafe { ptr.as_ref() })
    }
}

/// The response structure for the Entropy Request.
#[repr(C)]
pub struct EntropyResponse {
    /// The revision of this response structure.
    pub revision: u64,
    /// The number of random values provided. This is the requested count, unless that
    /// exceeds a limit set by the bootloader.
    pub value_count: u64,
    values: *const u64,
}

impl EntropyResponse {
    /// Returns the random values. The slice is empty if [`Self::value_count`] is 0.
    #[must_use]
    pub fn values(&self) -> &[u64] {
        if self.value_count == 0 || self.values.is_null() {
            return &[];
        }
        #[expect(
            clippy::cast_possible_truncation,
            reason = "Limine protocol fields fit in target width"
        )]
        // SAFETY: For a non-zero count, the bootloader provides a valid, aligned HHDM
        // pointer to `value_count` values that stay valid for the lifetime of the kernel.
        unsafe {
            core::slice::from_raw_parts(self.values, self.value_count as usize)
        }
    }
}

#[cfg(test)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn response_header_sizes() {
        assert_eq!(size_of::<StackSizeResponse>(), 8);
        assert_eq!(size_of::<EntryPointResponse>(), 8);
        assert_eq!(size_of::<KeepIommuResponse>(), 8);
        assert_eq!(size_of::<ExecutableCmdlineResponse>(), 16);
        assert_eq!(size_of::<FirmwareTypeResponse>(), 16);
        assert_eq!(size_of::<HhdmResponse>(), 16);
        assert_eq!(size_of::<PagingModeResponse>(), 16);
        assert_eq!(size_of::<BspHartIdResponse>(), 16);
        assert_eq!(size_of::<ExecutableFileResponse>(), 16);
        assert_eq!(size_of::<RsdpResponse>(), 16);
        assert_eq!(size_of::<EfiSystemTableResponse>(), 16);
        assert_eq!(size_of::<DateAtBootResponse>(), 16);
        assert_eq!(size_of::<DeviceTreeBlobResponse>(), 16);
        assert_eq!(size_of::<TscFrequencyResponse>(), 16);
    }

    #[test]
    fn bootloader_info_layout() {
        assert_eq!(offset_of!(BootloaderInfoResponse, name), 8);
        assert_eq!(offset_of!(BootloaderInfoResponse, version), 16);
        assert_eq!(size_of::<BootloaderInfoResponse>(), 24);
    }

    #[test]
    fn list_response_layouts() {
        assert_eq!(offset_of!(FramebufferResponse, framebuffer_count), 8);
        assert_eq!(offset_of!(FramebufferResponse, framebuffers), 16);
        assert_eq!(size_of::<FramebufferResponse>(), 24);
        assert_eq!(offset_of!(MemMapResponse, entry_count), 8);
        assert_eq!(offset_of!(MemMapResponse, entries), 16);
        assert_eq!(size_of::<MemMapResponse>(), 24);
        assert_eq!(offset_of!(ModuleResponse, module_count), 8);
        assert_eq!(offset_of!(ModuleResponse, modules), 16);
        assert_eq!(size_of::<ModuleResponse>(), 24);
        assert_eq!(offset_of!(FlantermFbInitParamsResponse, entry_count), 8);
        assert_eq!(offset_of!(FlantermFbInitParamsResponse, entries), 16);
        assert_eq!(size_of::<FlantermFbInitParamsResponse>(), 24);
        assert_eq!(offset_of!(EntropyResponse, value_count), 8);
        assert_eq!(offset_of!(EntropyResponse, values), 16);
        assert_eq!(size_of::<EntropyResponse>(), 24);
    }

    #[test]
    fn smbios_layout() {
        assert_eq!(offset_of!(SmbiosResponse, entry_32_addr), 8);
        assert_eq!(offset_of!(SmbiosResponse, entry_64_addr), 16);
        assert_eq!(size_of::<SmbiosResponse>(), 24);
    }

    #[test]
    fn efi_memory_map_layout() {
        assert_eq!(offset_of!(EfiMemoryMapResponse, memory_map), 8);
        assert_eq!(offset_of!(EfiMemoryMapResponse, memory_map_size), 16);
        assert_eq!(offset_of!(EfiMemoryMapResponse, descriptor_size), 24);
        assert_eq!(offset_of!(EfiMemoryMapResponse, descriptor_version), 32);
        assert_eq!(size_of::<EfiMemoryMapResponse>(), 40);
    }

    #[test]
    fn address_and_timing_layouts() {
        assert_eq!(offset_of!(ExecutableAddressResponse, phys_base), 8);
        assert_eq!(offset_of!(ExecutableAddressResponse, virt_base), 16);
        assert_eq!(size_of::<ExecutableAddressResponse>(), 24);
        assert_eq!(offset_of!(BootloaderPerformanceResponse, reset_us), 8);
        assert_eq!(offset_of!(BootloaderPerformanceResponse, init_us), 16);
        assert_eq!(offset_of!(BootloaderPerformanceResponse, exec_us), 24);
        assert_eq!(size_of::<BootloaderPerformanceResponse>(), 32);
    }

    #[test]
    fn tpm_event_log_layout() {
        assert_eq!(offset_of!(TpmEventLogResponse, format), 8);
        assert_eq!(offset_of!(TpmEventLogResponse, size), 16);
        assert_eq!(offset_of!(TpmEventLogResponse, address), 24);
        assert_eq!(size_of::<TpmEventLogResponse>(), 32);
        assert_eq!(TpmEventLogFormat::TCG_1_2.0, 1);
        assert_eq!(TpmEventLogFormat::TCG_2.0, 2);
    }

    #[test]
    fn firmware_type_values() {
        assert_eq!(FirmwareType::Bios as u64, 0);
        assert_eq!(FirmwareType::Efi32 as u64, 1);
        assert_eq!(FirmwareType::Efi64 as u64, 2);
        assert_eq!(FirmwareType::Sbi as u64, 3);
    }
}
