//! Request structures for the Limine boot protocol.
//!
//! This module contains all request structures that a kernel can use to request information
//! from the Limine bootloader. Each request structure has a unique ID that the bootloader
//! uses to identify and process the request.
//!
//! # Request-Response Pattern
//!
//! All requests follow a similar pattern:
//! 1. Create a static request structure with the `.new()` constructor
//! 2. Place it in the `.requests` section using the `#[link_section]` attribute
//! 3. Mark it with `#[used]` to prevent the linker from removing it
//! 4. After boot, call `.response()` to get the filled-in response
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
//! fn use_memory_map() {
//!     if let Some(response) = MEMMAP_REQUEST.response() {
//!         for entry in response.entries() {
//!             // Process memory map entry
//!         }
//!     }
//! }
//! ```

use core::cell::UnsafeCell;

use crate::{
    module::InternalModule,
    paging::PagingMode,
    response::{self as res, Response},
};

/// Macro to generate Limine IDs.
///
/// Each request type has a unique 4-part ID consisting of a magic number and a request-specific
/// identifier.
macro_rules! limine_id {
    ($part1:expr, $part2: expr) => {
        [
            0xc7b1_dd30_df4c_8b88u64,
            0x0a82_e883_a194_f07bu64,
            $part1,
            $part2,
        ]
    };
}

/// Marker placed at the start of the requests structure.
///
/// Used by the bootloader to speedup searching for requests.
/// If a start marker is used, an end marker must also be used.
#[repr(C, align(8))]
pub struct RequestsStartMarker([u64; 4]);

impl Default for RequestsStartMarker {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestsStartMarker {
    /// The Limine ID for the Requests Start Marker.
    pub const ID: [u64; 4] = [
        0xf6b8_f4b3_9de7_d1ae,
        0xfab9_1a69_40fc_b9cf,
        0x785c_6ed0_15d3_e316,
        0x181e_920a_7852_b9d9,
    ];

    /// Creates a new `RequestsStartMarker`.
    #[must_use]
    pub const fn new() -> Self {
        Self(Self::ID)
    }
}

/// Marker placed at the end of the requests structure.
#[repr(C, align(8))]
pub struct RequestsEndMarker([u64; 2]);

impl Default for RequestsEndMarker {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestsEndMarker {
    /// The Limine ID for the Requests End Marker.
    pub const ID: [u64; 2] = [0xadc0_e053_1bb1_0d03, 0x9572_709f_3176_4c62];

    /// Creates a new `RequestsEndMarker`.
    #[must_use]
    pub const fn new() -> Self {
        Self(Self::ID)
    }
}

/// Structure representing the base revision of the Limine protocol.
#[repr(C, align(8))]
pub struct BaseRevision(UnsafeCell<[u64; 3]>);

impl Default for BaseRevision {
    fn default() -> Self {
        Self::new()
    }
}

impl BaseRevision {
    /// The Limine ID for the Base Revision.
    pub const ID: [u64; 2] = [0xf956_2b2d_5c95_a6c8, 0x6a7b_3849_4453_6bdc];

    /// Creates a new `BaseRevision` with the recommended base revision (4).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_base_revision(4)
    }

    /// Creates a new `BaseRevision` with the specified base revision.
    ///
    /// Some revisions may not work nicely with this crate, and must be handled manually.
    #[must_use]
    pub const fn with_base_revision(revision: u64) -> Self {
        Self(UnsafeCell::new([Self::ID[0], Self::ID[1], revision]))
    }

    /// Returns the loaded base revision.
    #[must_use]
    pub fn loaded_revision(&self) -> u64 {
        // SAFETY: The bootloader writes to this cell before control is passed to the kernel,
        // and no concurrent writes occur after that point.
        unsafe { self.0.as_ref_unchecked()[1] }
    }

    /// Returns the requested revision.
    #[must_use]
    pub fn revision(&self) -> u64 {
        // SAFETY: The bootloader writes to this cell before control is passed to the kernel,
        // and no concurrent writes occur after that point.
        unsafe { self.0.as_ref_unchecked()[2] }
    }

    /// Checks if the requested revision is supported by this crate.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.revision() == 0
    }
}

// SAFETY: BaseRevision is only written by the bootloader before the kernel starts,
// and is only read (never written) by the kernel afterward. It is not used in a
// multithreaded context during boot.
unsafe impl Sync for BaseRevision {}

/// The request structure for the Bootloader Info Request.
#[repr(C, align(8))]
pub struct BootloaderInfoRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::BootloaderInfoResponse>,
}

impl Default for BootloaderInfoRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl BootloaderInfoRequest {
    /// The Limine ID for the Bootloader Info Request.
    pub const ID: [u64; 4] = limine_id!(0xf550_38d8_e2a1_202f, 0x2794_26fc_f5f5_9740);

    /// Creates a new `BootloaderInfoRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `BootloaderInfoRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `BootloaderInfoResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::BootloaderInfoResponse> {
        self.response.get()
    }
}

/// The request structure for the Executable Cmdline Request.
#[repr(C, align(8))]
pub struct ExecutableCmdlineRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::ExecutableCmdlineResponse>,
}

impl Default for ExecutableCmdlineRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutableCmdlineRequest {
    /// The Limine ID for the Executable Cmdline Request.
    pub const ID: [u64; 4] = limine_id!(0x4b16_1536_e598_651e, 0xb390_ad4a_2f1f_303a);

    /// Creates a new `ExecutableCmdlineRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `ExecutableCmdlineRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `ExecutableCmdlineResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::ExecutableCmdlineResponse> {
        self.response.get()
    }
}

/// Request structure for querying the firmware type.
///
/// This request allows the kernel to determine what type of firmware was used to boot the system
/// (BIOS, UEFI 32-bit, UEFI 64-bit, or SBI for ARM).
#[repr(C, align(8))]
pub struct FirmwareTypeRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::FirmwareTypeResponse>,
}

impl Default for FirmwareTypeRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl FirmwareTypeRequest {
    /// The Limine ID for the Firmware Type Request.
    pub const ID: [u64; 4] = limine_id!(0x8c2f_75d9_0bef_28a8, 0x7045_a468_8eac_00c3);

    /// Creates a new `FirmwareTypeRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `FirmwareTypeRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `FirmwareTypeResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::FirmwareTypeResponse> {
        self.response.get()
    }
}

/// The request structure for the Stack Size Request.
#[repr(C, align(8))]
pub struct StackSizeRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::StackSizeResponse>,
    /// The requested stack size in bytes.
    pub stack_size: u64,
}

impl StackSizeRequest {
    /// The Limine ID for the Stack Size Request.
    pub const ID: [u64; 4] = limine_id!(0x224e_f046_0a8e_8926, 0xe1cb_0fc2_5f46_ea3d);

    /// Creates a new `StackSizeRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new(stack_size: u64) -> Self {
        Self::with_revision(0, stack_size)
    }

    /// Creates a new `StackSizeRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64, stack_size: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
            stack_size,
        }
    }

    /// Returns a reference to the `StackSizeResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::StackSizeResponse> {
        self.response.get()
    }
}

/// The request structure for the HHDM Request.
#[repr(C, align(8))]
pub struct HhdmRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::HhdmResponse>,
}

impl Default for HhdmRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl HhdmRequest {
    /// The Limine ID for the HHDM Request.
    pub const ID: [u64; 4] = limine_id!(0x48dc_f1cb_8ad2_b852, 0x6398_4e95_9a98_244b);

    /// Creates a new `HhdmRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `HhdmRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `HhdmResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::HhdmResponse> {
        self.response.get()
    }
}

/// Request structure for framebuffer information.
///
/// This request asks the bootloader to provide one or more framebuffers for graphical output.
/// The response includes information about available video modes and framebuffer addresses.
#[repr(C, align(8))]
pub struct FramebufferRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::FramebufferResponse>,
}

impl Default for FramebufferRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl FramebufferRequest {
    /// The Limine ID for the Framebuffer Request.
    pub const ID: [u64; 4] = limine_id!(0x9d58_27dc_d881_dd75, 0xa314_8604_f6fa_b11b);

    /// Creates a new `FramebufferRequest` with the recommended revision (1).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(1)
    }

    /// Creates a new `FramebufferRequest` with the specified revision.
    ///
    /// Only revision 0 and 1 are currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is greater than 1.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(
            revision <= 1,
            "Only revision 0 and 1 are currently defined."
        );
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `FramebufferResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::FramebufferResponse> {
        self.response.get()
    }
}

/// Request structure for configuring the paging mode.
///
/// This request allows the kernel to specify what paging mode it wants to use.
/// The bootloader will set up the requested paging mode if possible, within the
/// constraints specified by `min_mode` and `max_mode`.
///
/// On `x86_64`, this can be used to request 4-level or 5-level paging.
/// On RISC-V, this can be used to request Sv39, Sv48, or Sv57 paging modes.
#[repr(C, align(8))]
pub struct PagingModeRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::PagingModeResponse>,
    /// The preferred paging mode.
    pub mode: PagingMode,
    /// The maximum acceptable paging mode.
    pub max_mode: PagingMode,
    /// The minimum acceptable paging mode.
    pub min_mode: PagingMode,
}

impl PagingModeRequest {
    /// The Limine ID for the Paging Mode Request.
    pub const ID: [u64; 4] = limine_id!(0x95c1_a0ed_ab09_44cb, 0xa4e5_cb38_42f7_488a);

    /// Creates a new `PagingModeRequest` with the recommended revision (1).
    #[must_use]
    pub const fn new(mode: PagingMode, min_mode: PagingMode, max_mode: PagingMode) -> Self {
        Self::with_revision(1, mode, min_mode, max_mode)
    }

    /// Creates a new `PagingModeRequest` with the specified revision.
    ///
    /// Only revision 0 and 1 are currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is greater than 1.
    #[must_use]
    pub const fn with_revision(
        revision: u64,
        mode: PagingMode,
        min_mode: PagingMode,
        max_mode: PagingMode,
    ) -> Self {
        assert!(
            revision <= 1,
            "Only revision 0 and 1 are currently defined."
        );
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
            mode,
            min_mode,
            max_mode,
        }
    }

    /// Returns a reference to the `PagingModeResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::PagingModeResponse> {
        self.response.get()
    }
}

/// Request structure for multiprocessor information.
///
/// This request retrieves information about all processors/cores in the system,
/// including the bootstrap processor (BSP) and application processors (APs).
#[repr(C, align(8))]
pub struct MpRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::MpResponse>,
    /// Configuration flags for multiprocessor setup.
    ///
    /// Bit 0: Enable x2APIC mode if supported (`x86_64` only)
    pub flags: u64,
}

impl MpRequest {
    /// The Limine ID for the MP Request.
    pub const ID: [u64; 4] = limine_id!(0x95a6_7b81_9a1b_857e, 0xa0b6_1b72_3b6a_73e0);

    /// Creates a new `MpRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new(flags: u64) -> Self {
        Self::with_revision(0, flags)
    }

    /// Creates a new `MpRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64, flags: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
            flags,
        }
    }

    /// Returns a reference to the `MpResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::MpResponse> {
        self.response.get()
    }
}

/// Request structure for the BSP Hart ID on RISC-V systems.
///
/// This RISC-V-specific request retrieves the Hart ID of the bootstrap processor.
/// Hart IDs are used in RISC-V systems to identify hardware threads.
#[repr(C, align(8))]
pub struct BspHartIdRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::BspHartIdResponse>,
}

impl Default for BspHartIdRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl BspHartIdRequest {
    /// The Limine ID for the BSP Hart ID Request.
    pub const ID: [u64; 4] = limine_id!(0x1369_359f_0255_25f9, 0x2ff2_a561_7839_1bb6);

    /// Creates a new `BspHartIdRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `BspHartIdRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `BspHartIdResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::BspHartIdResponse> {
        self.response.get()
    }
}

/// Request structure for the system memory map.
///
/// This request retrieves the memory map of the system, describing which physical memory
/// regions are available, reserved, or have special purposes. This is crucial for
/// memory management in the kernel.
#[repr(C, align(8))]
pub struct MemMapRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::MemMapResponse>,
}

impl Default for MemMapRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl MemMapRequest {
    /// The Limine ID for the Memmap Request.
    pub const ID: [u64; 4] = limine_id!(0x67cf_3d9d_378a_806f, 0xe304_acdf_c50c_3c62);

    /// Creates a new `MemMapRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `MemMapRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `MemMapResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::MemMapResponse> {
        self.response.get()
    }
}

/// Request structure for specifying a custom entry point.
///
/// This request allows the kernel to specify an alternative entry point address
/// that the bootloader should jump to instead of the default entry point specified
/// in the executable headers.
#[repr(C, align(8))]
pub struct EntryPointRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::EntryPointResponse>,
    /// The address to jump to as the entry point.
    pub entry_point_address: u64,
}

impl EntryPointRequest {
    /// The Limine ID for the Entry Point Request.
    pub const ID: [u64; 4] = limine_id!(0x13d8_6c03_5a1c_d3e1, 0x2b0c_aa89_d8f3_026a);

    /// Creates a new `EntryPointRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new(entry_point_address: u64) -> Self {
        Self::with_revision(0, entry_point_address)
    }

    /// Creates a new `EntryPointRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64, entry_point_address: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
            entry_point_address,
        }
    }

    /// Returns a reference to the `EntryPointResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::EntryPointResponse> {
        self.response.get()
    }
}

/// Request structure for executable file information.
///
/// This request retrieves information about the executable file that the kernel
/// was loaded from, including its address in memory and metadata.
#[repr(C, align(8))]
pub struct ExecutableFileRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::ExecutableFileResponse>,
}

impl Default for ExecutableFileRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutableFileRequest {
    /// The Limine ID for the Executable File Request.
    pub const ID: [u64; 4] = limine_id!(0xad97_e90e_83f1_ed67, 0x31eb_5d1c_5ff2_3b69);

    /// Creates a new `ExecutableFileRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `ExecutableFileRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `ExecutableFileResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::ExecutableFileResponse> {
        self.response.get()
    }
}

/// Request structure for kernel module files.
///
/// This request allows the kernel to specify which additional modules should be loaded
/// and to retrieve information about them after boot. Modules can be used for drivers,
/// configuration files, or other resources needed by the kernel.
#[repr(C, align(8))]
pub struct ModuleRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::ModuleResponse>,

    /// The number of internal modules to request.
    pub internal_module_count: u64,
    /// Pointer to an array of internal module specifications.
    pub internal_modules: *const *const InternalModule,
}

// SAFETY: ModuleRequest is only written by the bootloader before the kernel starts.
// The raw pointer field `internal_modules` points to static data that does not change
// after initialization, so sharing across threads is safe.
unsafe impl Sync for ModuleRequest {}

impl ModuleRequest {
    /// The Limine ID for the Module Request.
    pub const ID: [u64; 4] = limine_id!(0x3e7e_2797_02be_32af, 0xca1c_4f3b_d128_0cee);

    /// Creates a new `ModuleRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new(
        internal_modules: *const *const InternalModule,
        internal_module_count: u64,
    ) -> Self {
        Self::with_revision(0, internal_modules, internal_module_count)
    }

    /// Creates a new `ModuleRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(
        revision: u64,
        internal_modules: *const *const InternalModule,
        internal_module_count: u64,
    ) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
            internal_module_count,
            internal_modules,
        }
    }

    /// Returns a reference to the `ModuleResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::ModuleResponse> {
        self.response.get()
    }
}

/// Request structure for the ACPI RSDP (Root System Description Pointer).
///
/// This request retrieves the address of the RSDP structure, which is the entry point
/// to the ACPI tables. This is essential for discovering and configuring hardware
/// through ACPI.
#[repr(C, align(8))]
pub struct RsdpRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::RsdpResponse>,
}

impl Default for RsdpRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl RsdpRequest {
    /// The Limine ID for the RSDP Request.
    pub const ID: [u64; 4] = limine_id!(0xc5e7_7b6b_397e_7b43, 0x2763_7845_accd_cf3c);

    /// Creates a new `RsdpRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `RsdpRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `RsdpResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::RsdpResponse> {
        self.response.get()
    }
}

/// Request structure for SMBIOS (System Management BIOS) tables.
///
/// This request retrieves the address of the SMBIOS entry point, which provides
/// information about the system hardware, BIOS, and configuration.
#[repr(C, align(8))]
pub struct SmbiosRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::SmbiosResponse>,
}

impl Default for SmbiosRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl SmbiosRequest {
    /// The Limine ID for the SMBIOS Request.
    pub const ID: [u64; 4] = limine_id!(0x9e90_46f1_1e09_5391, 0xaa4a_520f_efbd_e5ee);

    /// Creates a new `SmbiosRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `SmbiosRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `SmbiosResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::SmbiosResponse> {
        self.response.get()
    }
}

/// Request structure for the EFI System Table.
///
/// This request retrieves the address of the UEFI System Table, which provides
/// access to UEFI runtime services and boot services. Only available when booting
/// via UEFI firmware.
#[repr(C, align(8))]
pub struct EfiSystemTableRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::EfiSystemTableResponse>,
}

impl Default for EfiSystemTableRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl EfiSystemTableRequest {
    /// The Limine ID for the EFI System Table Request.
    pub const ID: [u64; 4] = limine_id!(0x5ceb_a516_3eaa_f6d6, 0x0a69_8161_0cf6_5fcc);

    /// Creates a new `EfiSystemTableRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `EfiSystemTableRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `EfiSystemTableResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::EfiSystemTableResponse> {
        self.response.get()
    }
}

/// Request structure for the EFI memory map.
///
/// This request retrieves the UEFI memory map, which describes memory layout
/// from the firmware's perspective. Only available when booting via UEFI firmware.
#[repr(C, align(8))]
pub struct EfiMemoryMapRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::EfiMemoryMapResponse>,
}

impl Default for EfiMemoryMapRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl EfiMemoryMapRequest {
    /// The Limine ID for the EFI Memory Map Request.
    pub const ID: [u64; 4] = limine_id!(0x7df6_2a43_1d68_72d5, 0xa4fc_dfb3_e573_06c8);

    /// Creates a new `EfiMemoryMapRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `EfiMemoryMapRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `EfiMemoryMapResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::EfiMemoryMapResponse> {
        self.response.get()
    }
}

/// Request structure for the date and time at boot.
///
/// This request retrieves the current date and time as a UNIX timestamp
/// at the moment the bootloader passes control to the kernel.
#[repr(C, align(8))]
pub struct DateAtBootRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::DateAtBootResponse>,
}

impl Default for DateAtBootRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl DateAtBootRequest {
    /// The Limine ID for the Date At Boot Request.
    pub const ID: [u64; 4] = limine_id!(0x5027_46e1_84c0_88aa, 0xfbc5_ec83_e632_7893);

    /// Creates a new `DateAtBootRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `DateAtBootRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `DateAtBootResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::DateAtBootResponse> {
        self.response.get()
    }
}

/// Request structure for executable address information.
///
/// This request retrieves both the physical and virtual base addresses where
/// the kernel executable has been loaded into memory.
#[repr(C, align(8))]
pub struct ExecutableAddressRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::ExecutableAddressResponse>,
}

impl Default for ExecutableAddressRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecutableAddressRequest {
    /// The Limine ID for the Executable Address Request.
    pub const ID: [u64; 4] = limine_id!(0x71ba_7686_3cc5_5f63, 0xb264_4a48_c516_a487);

    /// Creates a new `ExecutableAddressRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `ExecutableAddressRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `ExecutableAddressResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::ExecutableAddressResponse> {
        self.response.get()
    }
}

/// Request structure for the Device Tree Blob (DTB).
///
/// This request retrieves the address of the Device Tree Blob, which describes
/// the hardware layout on systems that use device trees (typically ARM and RISC-V).
#[repr(C, align(8))]
pub struct DeviceTreeBlobRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::DeviceTreeBlobResponse>,
}

impl Default for DeviceTreeBlobRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl DeviceTreeBlobRequest {
    /// The Limine ID for the Device Tree Blob Request.
    pub const ID: [u64; 4] = limine_id!(0xb40d_db48_fb54_bac7, 0x5450_8149_3f81_ffb7);

    /// Creates a new `DeviceTreeBlobRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `DeviceTreeBlobRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `DeviceTreeBlobResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::DeviceTreeBlobResponse> {
        self.response.get()
    }
}

/// Request structure for bootloader performance metrics.
///
/// This request retrieves timing information about the boot process, including
/// time spent in reset, initialization, and handoff to the kernel. Useful for
/// boot time optimization and profiling.
#[repr(C, align(8))]
pub struct BootloaderPerformanceRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::BootloaderPerformanceResponse>,
}

impl Default for BootloaderPerformanceRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl BootloaderPerformanceRequest {
    /// The Limine ID for the Bootloader Performance Request.
    pub const ID: [u64; 4] = limine_id!(0x6b50_ad9b_f36d_13ad, 0xdc4c_7e88_fc75_9e17);

    /// Creates a new `BootloaderPerformanceRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `BootloaderPerformanceRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
        }
    }

    /// Returns a reference to the `BootloaderPerformanceResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::BootloaderPerformanceResponse> {
        self.response.get()
    }
}
