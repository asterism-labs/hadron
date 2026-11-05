//! Request structures for the Limine boot protocol.
//!
//! This module contains all request structures that a kernel can use to request information
//! from the Limine bootloader. Each request structure has a unique ID that the bootloader
//! uses to identify and process the request.
//!
//! # Request-Response Pattern
//!
//! All requests follow a similar pattern:
//! 1. Declare every request in one [`requests!`](crate::requests) block,
//!    which places them between the start and end markers
//! 2. After boot, call `.response()` to get the filled-in response
//!
//! # Example
//!
//! ```no_run
//! use limine::{BaseRevision, MemMapRequest};
//!
//! limine::requests! {
//!     static REQUESTS: Requests {
//!         base_revision: BaseRevision = BaseRevision::new(),
//!         memmap: MemMapRequest = MemMapRequest::new(),
//!     }
//! }
//!
//! fn use_memory_map() {
//!     if let Some(response) = REQUESTS.memmap.response() {
//!         for entry in response.entries() {
//!             // Process memory map entry
//!         }
//!     }
//! }
//! ```

use core::cell::UnsafeCell;

use crate::{
    module::InternalModule,
    mp::MpRequestFlags,
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

/// Declares a struct that holds the given requests between a
/// [`RequestsStartMarker`] and a [`RequestsEndMarker`], and a `#[used]`
/// static of it.
///
/// Limine scans only the memory between the two markers, so every request
/// the executable makes, the [`BaseRevision`] included, belongs in one
/// `requests!` block. The struct is `#[repr(C)]`, so the fields stay in
/// order between the markers, and the static needs no linker section.
///
/// ```no_run
/// use limine::{BaseRevision, HhdmRequest};
///
/// limine::requests! {
///     static REQUESTS: Requests {
///         base_revision: BaseRevision = BaseRevision::new(),
///         hhdm: HhdmRequest = HhdmRequest::new(),
///     }
/// }
///
/// fn hhdm_offset() -> Option<u64> {
///     REQUESTS.hhdm.response().map(|hhdm| hhdm.offset)
/// }
/// ```
#[macro_export]
macro_rules! requests {
    (
        $(#[$meta:meta])*
        $vis:vis static $name:ident: $ty:ident {
            $(
                $(#[$field_meta:meta])*
                $field_vis:vis $field:ident: $field_ty:ty = $init:expr
            ),* $(,)?
        }
    ) => {
        /// Limine's requests, between the start and end markers.
        // Limine reads every field. The kernel may never read some.
        #[allow(dead_code)]
        #[repr(C)]
        $vis struct $ty {
            __start_marker: $crate::RequestsStartMarker,
            $(
                $(#[$field_meta])*
                $field_vis $field: $field_ty,
            )*
            __end_marker: $crate::RequestsEndMarker,
        }

        $(#[$meta])*
        #[used]
        $vis static $name: $ty = $ty {
            __start_marker: $crate::RequestsStartMarker::new(),
            $($field: $init,)*
            __end_marker: $crate::RequestsEndMarker::new(),
        };
    };
}

/// The base revision tag, which tells the bootloader which base revision of the
/// protocol the executable expects.
///
/// The tag is three `u64` slots. The first two hold the magic in [`Self::ID`]. The third
/// holds the requested revision. The bootloader sets the third slot to 0 if it supports
/// that revision, and from base revision 3 it also writes the revision it actually used
/// into the second slot.
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

    /// The base revision that [`Self::new`] requests.
    ///
    /// Base revisions 0 to 5 are deprecated, so this is 6.
    pub const LATEST: u64 = 6;

    /// Creates a new `BaseRevision` that requests base revision 6.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_base_revision(Self::LATEST)
    }

    /// Creates a new `BaseRevision` that requests `revision`.
    ///
    /// The doc comments in this crate describe base revision 6. Older revisions change
    /// some behaviour, such as whether addresses are physical or virtual, and the caller
    /// must handle those differences.
    #[must_use]
    pub const fn with_base_revision(revision: u64) -> Self {
        Self(UnsafeCell::new([Self::ID[0], Self::ID[1], revision]))
    }

    /// Returns the base revision the bootloader used to load the executable.
    ///
    /// Returns `None` if the bootloader did not write it. Bootloaders that support base
    /// revision 3 or later always write it; older ones leave the magic in place.
    #[must_use]
    pub fn loaded_revision(&self) -> Option<u64> {
        // SAFETY: The bootloader writes to this cell before control is passed to the kernel,
        // and no concurrent writes occur after that point.
        let loaded = unsafe { (*self.0.get())[1] };
        (loaded != Self::ID[1]).then_some(loaded)
    }

    /// Returns the raw third slot of the tag.
    ///
    /// Before boot, and after boot if the bootloader does not support the requested
    /// revision, this is the requested revision. After boot it is 0 if the bootloader
    /// supports the requested revision. Use [`Self::is_supported`] to test for support.
    #[must_use]
    pub fn revision_slot(&self) -> u64 {
        // SAFETY: The bootloader writes to this cell before control is passed to the kernel,
        // and no concurrent writes occur after that point.
        unsafe { (*self.0.get())[2] }
    }

    /// Returns whether the bootloader supports the requested base revision.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.revision_slot() == 0
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

    /// Creates a new `FramebufferRequest` with the recommended revision (0).
    ///
    /// The video mode list comes with response revision 1, which needs no newer request.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `FramebufferRequest` with the specified revision.
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
    /// Flags that change how the bootloader starts the processors.
    pub flags: MpRequestFlags,
}

impl MpRequest {
    /// The Limine ID for the MP Request.
    pub const ID: [u64; 4] = limine_id!(0x95a6_7b81_9a1b_857e, 0xa0b6_1b72_3b6a_73e0);

    /// Creates a new `MpRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new(flags: MpRequestFlags) -> Self {
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
    pub const fn with_revision(revision: u64, flags: MpRequestFlags) -> Self {
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
    /// The address to jump to as the entry point. It must be a function inside the
    /// loaded executable image.
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

    /// The number of internal modules to request. Read from request revision 1.
    pub internal_module_count: u64,
    /// Pointer to an array of `internal_module_count` pointers to internal module
    /// specifications. Read from request revision 1.
    pub internal_modules: *const *const InternalModule,
}

// SAFETY: ModuleRequest is only written by the bootloader before the kernel starts.
// The raw pointer field `internal_modules` points to static data that does not change
// after initialization, so sharing across threads is safe.
unsafe impl Sync for ModuleRequest {}

impl ModuleRequest {
    /// The Limine ID for the Module Request.
    pub const ID: [u64; 4] = limine_id!(0x3e7e_2797_02be_32af, 0xca1c_4f3b_d128_0cee);

    /// Creates a new `ModuleRequest` with the recommended revision (1).
    ///
    /// Revision 1 is the first revision in which the bootloader reads the internal
    /// modules.
    #[must_use]
    pub const fn new(
        internal_modules: *const *const InternalModule,
        internal_module_count: u64,
    ) -> Self {
        Self::with_revision(1, internal_modules, internal_module_count)
    }

    /// Creates a new `ModuleRequest` with the specified revision.
    ///
    /// Only revision 0 and 1 are currently defined. Revision 0 ignores the internal
    /// modules.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is greater than 1.
    #[must_use]
    pub const fn with_revision(
        revision: u64,
        internal_modules: *const *const InternalModule,
        internal_module_count: u64,
    ) -> Self {
        assert!(
            revision <= 1,
            "Only revision 0 and 1 are currently defined."
        );
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

/// Request structure for the TPM event log.
///
/// This request retrieves a copy of the firmware's TCG event log, captured through
/// `EFI_TCG2_PROTOCOL.GetEventLog()` before boot services exit. No response is provided
/// without a TPM or without `EFI_TCG2_PROTOCOL`.
#[repr(C, align(8))]
pub struct TpmEventLogRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::TpmEventLogResponse>,
}

impl Default for TpmEventLogRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl TpmEventLogRequest {
    /// The Limine ID for the TPM Event Log Request.
    pub const ID: [u64; 4] = limine_id!(0x98e0_94fc_7e76_e979, 0xee8d_8775_c54e_1d1f);

    /// Creates a new `TpmEventLogRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `TpmEventLogRequest` with the specified revision.
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

    /// Returns a reference to the `TpmEventLogResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::TpmEventLogResponse> {
        self.response.get()
    }
}

/// Request structure that asks the bootloader to keep IOMMUs enabled (`x86_64` only).
///
/// From base revision 5, the bootloader disables every Intel VT-d and AMD-Vi IOMMU before
/// handoff unless this request is present. No response is provided on other
/// architectures.
#[repr(C, align(8))]
pub struct KeepIommuRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::KeepIommuResponse>,
}

impl Default for KeepIommuRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl KeepIommuRequest {
    /// The Limine ID for the x86-64 Keep IOMMU Request.
    pub const ID: [u64; 4] = limine_id!(0x8eba_abe5_1f49_0179, 0x2aa8_6a59_ffb4_ab0f);

    /// Creates a new `KeepIommuRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `KeepIommuRequest` with the specified revision.
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

    /// Returns a reference to the `KeepIommuResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::KeepIommuResponse> {
        self.response.get()
    }
}

/// Request structure for the frequency of the primary timestamp counter.
///
/// The primary timestamp counter is the one read by `RDTSC` on `x86_64`, `CNTPCT_EL0` on
/// `AArch64`, `RDTIME` on RISC-V and `RDTIME.D` on `LoongArch64`. No response is provided
/// if the bootloader cannot determine the frequency.
#[repr(C, align(8))]
pub struct TscFrequencyRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::TscFrequencyResponse>,
}

impl Default for TscFrequencyRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl TscFrequencyRequest {
    /// The Limine ID for the TSC Frequency Request.
    pub const ID: [u64; 4] = limine_id!(0x10f2_ee1d_87d1_95e4, 0xf747_a2b7_8f6d_db31);

    /// Creates a new `TscFrequencyRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `TscFrequencyRequest` with the specified revision.
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

    /// Returns a reference to the `TscFrequencyResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::TscFrequencyResponse> {
        self.response.get()
    }
}

/// Request structure for the parameters the bootloader used to set up its Flanterm
/// framebuffer terminals.
///
/// This feature needs a [`FramebufferRequest`] as well. No response is provided if no
/// framebuffer is available.
#[repr(C, align(8))]
pub struct FlantermFbInitParamsRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::FlantermFbInitParamsResponse>,
}

impl Default for FlantermFbInitParamsRequest {
    fn default() -> Self {
        Self::new()
    }
}

impl FlantermFbInitParamsRequest {
    /// The Limine ID for the Flanterm FB Init Params Request.
    pub const ID: [u64; 4] = limine_id!(0x3259_399f_e7c5_f126, 0xe01c_1c8c_5db9_d1a9);

    /// Creates a new `FlantermFbInitParamsRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new() -> Self {
        Self::with_revision(0)
    }

    /// Creates a new `FlantermFbInitParamsRequest` with the specified revision.
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

    /// Returns a reference to the `FlantermFbInitParamsResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::FlantermFbInitParamsResponse> {
        self.response.get()
    }
}

/// Request structure for random 64-bit values.
///
/// The bootloader draws the values from the strongest source it has, such as CPU
/// instructions or firmware services, and falls back to weaker sources where nothing
/// stronger exists. They suit seeding an RNG.
#[repr(C, align(8))]
pub struct EntropyRequest {
    id: [u64; 4],
    revision: u64,
    response: Response<res::EntropyResponse>,
    /// The number of random 64-bit values requested.
    pub value_count: u64,
}

impl EntropyRequest {
    /// The Limine ID for the Entropy Request.
    pub const ID: [u64; 4] = limine_id!(0x65ea_8025_5d56_82c5, 0x9117_2407_23f4_93eb);

    /// Creates a new `EntropyRequest` with the recommended revision (0).
    #[must_use]
    pub const fn new(value_count: u64) -> Self {
        Self::with_revision(0, value_count)
    }

    /// Creates a new `EntropyRequest` with the specified revision.
    ///
    /// Only revision 0 is currently defined.
    ///
    /// # Panics
    ///
    /// Panics if `revision` is not 0.
    #[must_use]
    pub const fn with_revision(revision: u64, value_count: u64) -> Self {
        assert!(revision == 0, "Only revision 0 is currently defined.");
        Self {
            id: Self::ID,
            revision,
            response: Response::empty(),
            value_count,
        }
    }

    /// Returns a reference to the `EntropyResponse` if it is available.
    #[must_use]
    pub fn response(&self) -> Option<&res::EntropyResponse> {
        self.response.get()
    }
}

#[cfg(test)]
#[expect(
    clippy::unreadable_literal,
    reason = "IDs are written as in the spec, so they can be compared to it by eye"
)]
mod tests {
    use core::mem::offset_of;

    use super::*;

    #[test]
    fn request_ids_match_spec() {
        /// Checks a request ID against `LIMINE_COMMON_MAGIC` and two words from the spec.
        fn check(actual: [u64; 4], a: u64, b: u64) {
            assert_eq!(actual, [0xc7b1dd30df4c8b88, 0x0a82e883a194f07b, a, b]);
        }
        check(
            BootloaderInfoRequest::ID,
            0xf55038d8e2a1202f,
            0x279426fcf5f59740,
        );
        check(
            ExecutableCmdlineRequest::ID,
            0x4b161536e598651e,
            0xb390ad4a2f1f303a,
        );
        check(
            FirmwareTypeRequest::ID,
            0x8c2f75d90bef28a8,
            0x7045a4688eac00c3,
        );
        check(StackSizeRequest::ID, 0x224ef0460a8e8926, 0xe1cb0fc25f46ea3d);
        check(HhdmRequest::ID, 0x48dcf1cb8ad2b852, 0x63984e959a98244b);
        check(
            FramebufferRequest::ID,
            0x9d5827dcd881dd75,
            0xa3148604f6fab11b,
        );
        check(
            PagingModeRequest::ID,
            0x95c1a0edab0944cb,
            0xa4e5cb3842f7488a,
        );
        check(MpRequest::ID, 0x95a67b819a1b857e, 0xa0b61b723b6a73e0);
        check(BspHartIdRequest::ID, 0x1369359f025525f9, 0x2ff2a56178391bb6);
        check(MemMapRequest::ID, 0x67cf3d9d378a806f, 0xe304acdfc50c3c62);
        check(
            EntryPointRequest::ID,
            0x13d86c035a1cd3e1,
            0x2b0caa89d8f3026a,
        );
        check(
            ExecutableFileRequest::ID,
            0xad97e90e83f1ed67,
            0x31eb5d1c5ff23b69,
        );
        check(ModuleRequest::ID, 0x3e7e279702be32af, 0xca1c4f3bd1280cee);
        check(RsdpRequest::ID, 0xc5e77b6b397e7b43, 0x27637845accdcf3c);
        check(SmbiosRequest::ID, 0x9e9046f11e095391, 0xaa4a520fefbde5ee);
        check(
            EfiSystemTableRequest::ID,
            0x5ceba5163eaaf6d6,
            0x0a6981610cf65fcc,
        );
        check(
            TpmEventLogRequest::ID,
            0x98e094fc7e76e979,
            0xee8d8775c54e1d1f,
        );
        check(
            EfiMemoryMapRequest::ID,
            0x7df62a431d6872d5,
            0xa4fcdfb3e57306c8,
        );
        check(
            DateAtBootRequest::ID,
            0x502746e184c088aa,
            0xfbc5ec83e6327893,
        );
        check(
            ExecutableAddressRequest::ID,
            0x71ba76863cc55f63,
            0xb2644a48c516a487,
        );
        check(
            DeviceTreeBlobRequest::ID,
            0xb40ddb48fb54bac7,
            0x545081493f81ffb7,
        );
        check(
            BootloaderPerformanceRequest::ID,
            0x6b50ad9bf36d13ad,
            0xdc4c7e88fc759e17,
        );
        check(KeepIommuRequest::ID, 0x8ebaabe51f490179, 0x2aa86a59ffb4ab0f);
        check(
            TscFrequencyRequest::ID,
            0x10f2ee1d87d195e4,
            0xf747a2b78f6ddb31,
        );
        check(
            FlantermFbInitParamsRequest::ID,
            0x3259399fe7c5f126,
            0xe01c1c8c5db9d1a9,
        );
        check(EntropyRequest::ID, 0x65ea80255d5682c5, 0x9117240723f493eb);
    }

    #[test]
    fn markers_match_spec() {
        assert_eq!(
            RequestsStartMarker::ID,
            [
                0xf6b8f4b39de7d1ae,
                0xfab91a6940fcb9cf,
                0x785c6ed015d3e316,
                0x181e920a7852b9d9
            ]
        );
        assert_eq!(
            RequestsEndMarker::ID,
            [0xadc0e0531bb10d03, 0x9572709f31764c62]
        );
        assert_eq!(BaseRevision::ID, [0xf9562b2d5c95a6c8, 0x6a7b384944536bdc]);
    }

    #[test]
    fn base_revision_requests_six() {
        let tag = BaseRevision::new();
        // SAFETY: The test owns `tag`, so nothing else reads or writes it.
        assert_eq!(
            unsafe { *tag.0.get() },
            [0xf9562b2d5c95a6c8, 0x6a7b384944536bdc, 6]
        );
        assert_eq!(tag.revision_slot(), 6);
        assert!(!tag.is_supported());
    }

    #[test]
    fn loaded_revision_is_none_until_written() {
        let tag = BaseRevision::new();
        assert_eq!(tag.loaded_revision(), None);
        // SAFETY: The test owns `tag`, so nothing else reads or writes it. This mimics
        // what the bootloader writes when it supports the requested revision.
        unsafe { *tag.0.get() = [BaseRevision::ID[0], 6, 0] };
        assert_eq!(tag.loaded_revision(), Some(6));
        assert!(tag.is_supported());
    }

    #[test]
    fn request_layouts_match_spec() {
        assert_eq!(size_of::<BaseRevision>(), 24);
        assert_eq!(size_of::<RequestsStartMarker>(), 32);
        assert_eq!(size_of::<RequestsEndMarker>(), 16);
        assert_eq!(offset_of!(HhdmRequest, revision), 32);
        assert_eq!(offset_of!(HhdmRequest, response), 40);
        assert_eq!(size_of::<HhdmRequest>(), 48);
        assert_eq!(offset_of!(StackSizeRequest, stack_size), 48);
        assert_eq!(offset_of!(PagingModeRequest, mode), 48);
        assert_eq!(offset_of!(PagingModeRequest, max_mode), 56);
        assert_eq!(offset_of!(PagingModeRequest, min_mode), 64);
        assert_eq!(offset_of!(MpRequest, flags), 48);
        assert_eq!(size_of::<MpRequest>(), 56);
        assert_eq!(offset_of!(EntryPointRequest, entry_point_address), 48);
        assert_eq!(offset_of!(ModuleRequest, internal_module_count), 48);
        assert_eq!(offset_of!(ModuleRequest, internal_modules), 56);
        assert_eq!(offset_of!(EntropyRequest, value_count), 48);
    }

    #[test]
    fn default_revisions() {
        assert_eq!(FramebufferRequest::new().revision, 0);
        assert_eq!(
            PagingModeRequest::new(PagingMode::DEFAULT, PagingMode::MIN, PagingMode::DEFAULT)
                .revision,
            1
        );
        assert_eq!(ModuleRequest::new(core::ptr::null(), 0).revision, 1);
    }
}
