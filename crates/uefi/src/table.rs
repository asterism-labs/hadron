//! UEFI System Table, Boot Services, and Runtime Services.
//!
//! This module contains the core UEFI table structures: the [`SystemTable`] (the primary entry
//! point into UEFI services), [`BootServices`] (available until `ExitBootServices` is called),
//! and [`RuntimeServices`] (available throughout the OS lifetime).
//!
//! # Function Pointers
//!
//! All function pointers use `unsafe extern "efiapi" fn(...)` with parameter types matching
//! the UEFI specification. They are stored as bare function pointers (not `Option<fn>`) to
//! preserve the correct C struct layout.

use core::ffi::c_void;

use crate::{
    EfiEvent, EfiGuid, EfiHandle, EfiPhysicalAddress, EfiStatus, EfiTpl,
    memory::{EfiAllocateType, EfiMemoryType},
};

// ── Table Header ─────────────────────────────────────────────────────

/// Common header for all UEFI tables.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct TableHeader {
    /// A 64-bit signature that identifies the type of table that follows.
    pub signature: u64,
    /// The revision of the UEFI specification to which this table conforms.
    pub revision: u32,
    /// The size, in bytes, of the entire table including the header.
    pub header_size: u32,
    /// The 32-bit CRC for the entire table.
    pub crc32: u32,
    /// Reserved field; must be zero.
    pub reserved: u32,
}

// ── Configuration Table ──────────────────────────────────────────────

/// An entry in the UEFI configuration table array.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct ConfigurationTable {
    /// The GUID identifying the configuration table.
    pub vendor_guid: EfiGuid,
    /// A pointer to the vendor-specific table data.
    pub vendor_table: *mut c_void,
}

// ── System Table ─────────────────────────────────────────────────────

/// The UEFI System Table, the primary entry point to UEFI services.
///
/// A pointer to this table is passed to the UEFI application entry point. It provides
/// access to boot services, runtime services, console I/O, and configuration tables.
#[repr(C)]
pub struct SystemTable {
    /// The table header.
    pub header: TableHeader,
    /// Pointer to a null-terminated UCS-2 string identifying the firmware vendor.
    pub firmware_vendor: *const u16,
    /// The firmware revision.
    pub firmware_revision: u32,
    /// The handle for the active console input device.
    pub console_in_handle: EfiHandle,
    /// Pointer to the Simple Text Input Protocol for console input.
    pub console_in: *mut c_void,
    /// The handle for the active console output device.
    pub console_out_handle: EfiHandle,
    /// Pointer to the Simple Text Output Protocol for console output.
    pub console_out: *mut c_void,
    /// The handle for the active standard error console device.
    pub standard_error_handle: EfiHandle,
    /// Pointer to the Simple Text Output Protocol for standard error output.
    pub standard_error: *mut c_void,
    /// Pointer to the Runtime Services Table.
    pub runtime_services: *mut RuntimeServices,
    /// Pointer to the Boot Services Table.
    pub boot_services: *mut BootServices,
    /// The number of entries in the configuration table array.
    pub number_of_table_entries: usize,
    /// Pointer to the configuration table array.
    pub configuration_table: *mut ConfigurationTable,
}

impl SystemTable {
    /// Returns the boot services table.
    ///
    /// # Safety
    ///
    /// The caller must ensure that boot services have not been exited
    /// (i.e., `ExitBootServices` has not been called) and that
    /// `self.boot_services` is a valid pointer.
    #[must_use]
    pub unsafe fn boot_services(&self) -> &BootServices {
        unsafe { &*self.boot_services }
    }

    /// Returns the runtime services table.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `self.runtime_services` is a valid pointer.
    #[must_use]
    pub unsafe fn runtime_services(&self) -> &RuntimeServices {
        unsafe { &*self.runtime_services }
    }

    /// Returns the configuration table entries as a slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `self.configuration_table` is a valid pointer
    /// and `self.number_of_table_entries` is correct.
    #[must_use]
    pub unsafe fn configuration_tables(&self) -> &[ConfigurationTable] {
        unsafe {
            core::slice::from_raw_parts(self.configuration_table, self.number_of_table_entries)
        }
    }
}

// ── Supporting Enums ─────────────────────────────────────────────────

/// Timer delay type for `SetTimer`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerDelay {
    /// Cancel a previously set timer.
    Cancel = 0,
    /// Set a periodic timer that fires every `trigger_time` 100ns units.
    Periodic = 1,
    /// Set a one-shot timer that fires once after `trigger_time` 100ns units.
    Relative = 2,
}

/// Interface type for `InstallProtocolInterface`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterfaceType {
    /// Native interface.
    NativeInterface = 0,
}

/// Search type for `LocateHandle`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocateSearchType {
    /// Retrieve all handles in the handle database.
    AllHandles = 0,
    /// Retrieve the next handle that supports the specified registration key.
    ByRegisterNotify = 1,
    /// Retrieve all handles that support the specified protocol.
    ByProtocol = 2,
}

/// Reset type for `ResetSystem`.
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetType {
    /// Causes a system-wide reset, equivalent to power cycling.
    Cold = 0,
    /// Causes a system-wide initialization, equivalent to a warm reset.
    Warm = 1,
    /// Causes the system to enter a platform-specific shutdown state.
    Shutdown = 2,
    /// Causes a platform-specific reset type.
    PlatformSpecific = 3,
}

// ── Supporting Structs ───────────────────────────────────────────────

/// Information about an agent that has opened a protocol.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct OpenProtocolInformationEntry {
    /// The agent that opened the protocol.
    pub agent_handle: EfiHandle,
    /// The controller associated with the open.
    pub controller_handle: EfiHandle,
    /// Attributes used to open the protocol.
    pub attributes: u32,
    /// The number of times the protocol was opened by this agent.
    pub open_count: u32,
}

/// UEFI time representation.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EfiTime {
    /// Year (1900–9999).
    pub year: u16,
    /// Month (1–12).
    pub month: u8,
    /// Day (1–31).
    pub day: u8,
    /// Hour (0–23).
    pub hour: u8,
    /// Minute (0–59).
    pub minute: u8,
    /// Second (0–59).
    pub second: u8,
    /// Padding byte.
    pub pad1: u8,
    /// Nanoseconds (0–999,999,999).
    pub nanosecond: u32,
    /// Time zone offset in minutes from UTC (−1440 to 1440), or 0x7FF if unspecified.
    pub time_zone: i16,
    /// Daylight saving time flags.
    pub daylight: u8,
    /// Padding byte.
    pub pad2: u8,
}

/// Capabilities of the real time clock device.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct EfiTimeCapabilities {
    /// Resolution of the real time clock in counts per second.
    pub resolution: u32,
    /// Accuracy of the real time clock in parts per million.
    pub accuracy: u32,
    /// `true` if a time set operation clears the time below the resolution level.
    pub sets_to_zero: bool,
}

/// Header for a capsule, used in `UpdateCapsule`.
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct CapsuleHeader {
    /// GUID identifying the capsule.
    pub capsule_guid: EfiGuid,
    /// Size of the capsule header.
    pub header_size: u32,
    /// Capsule flags.
    pub flags: u32,
    /// Size of the entire capsule in bytes.
    pub capsule_image_size: u32,
}

// ── Boot Services ────────────────────────────────────────────────────

/// The UEFI Boot Services Table.
///
/// Boot services are available from the time the firmware passes control to the UEFI
/// application until `ExitBootServices` is called. After that point, only runtime services
/// remain available.
///
/// All function pointers use `unsafe extern "efiapi"` and match the UEFI specification
/// parameter types exactly.
#[repr(C)]
pub struct BootServices {
    /// The table header.
    pub header: TableHeader,

    // ── Task Priority Services ───────────────────────────────────
    /// Raises a task's priority level.
    pub raise_tpl: unsafe extern "efiapi" fn(new_tpl: EfiTpl) -> EfiTpl,
    /// Restores a task's priority level.
    pub restore_tpl: unsafe extern "efiapi" fn(old_tpl: EfiTpl),

    // ── Memory Services ──────────────────────────────────────────
    /// Allocates memory pages.
    pub allocate_pages: unsafe extern "efiapi" fn(
        alloc_type: EfiAllocateType,
        memory_type: EfiMemoryType,
        pages: usize,
        memory: *mut EfiPhysicalAddress,
    ) -> EfiStatus,
    /// Frees memory pages.
    pub free_pages:
        unsafe extern "efiapi" fn(memory: EfiPhysicalAddress, pages: usize) -> EfiStatus,
    /// Returns the current memory map.
    pub get_memory_map: unsafe extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut u8,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> EfiStatus,
    /// Allocates pool memory.
    pub allocate_pool: unsafe extern "efiapi" fn(
        pool_type: EfiMemoryType,
        size: usize,
        buffer: *mut *mut c_void,
    ) -> EfiStatus,
    /// Returns pool memory to the system.
    pub free_pool: unsafe extern "efiapi" fn(buffer: *mut c_void) -> EfiStatus,

    // ── Event & Timer Services ───────────────────────────────────
    /// Creates an event.
    pub create_event: unsafe extern "efiapi" fn(
        event_type: u32,
        notify_tpl: EfiTpl,
        notify_function: Option<unsafe extern "efiapi" fn(event: EfiEvent, context: *mut c_void)>,
        notify_context: *mut c_void,
        event: *mut EfiEvent,
    ) -> EfiStatus,
    /// Sets the type of timer and the trigger time for a timer event.
    pub set_timer: unsafe extern "efiapi" fn(
        event: EfiEvent,
        timer_type: TimerDelay,
        trigger_time: u64,
    ) -> EfiStatus,
    /// Stops execution until an event is signaled.
    pub wait_for_event: unsafe extern "efiapi" fn(
        number_of_events: usize,
        event: *mut EfiEvent,
        index: *mut usize,
    ) -> EfiStatus,
    /// Signals an event.
    pub signal_event: unsafe extern "efiapi" fn(event: EfiEvent) -> EfiStatus,
    /// Closes an event.
    pub close_event: unsafe extern "efiapi" fn(event: EfiEvent) -> EfiStatus,
    /// Checks whether an event is in the signaled state.
    pub check_event: unsafe extern "efiapi" fn(event: EfiEvent) -> EfiStatus,

    // ── Protocol Handler Services ────────────────────────────────
    /// Installs a protocol interface on a device handle.
    pub install_protocol_interface: unsafe extern "efiapi" fn(
        handle: *mut EfiHandle,
        protocol: *const EfiGuid,
        interface_type: InterfaceType,
        interface: *mut c_void,
    ) -> EfiStatus,
    /// Reinstalls a protocol interface on a device handle.
    pub reinstall_protocol_interface: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        old_interface: *mut c_void,
        new_interface: *mut c_void,
    ) -> EfiStatus,
    /// Removes a protocol interface from a device handle.
    pub uninstall_protocol_interface: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut c_void,
    ) -> EfiStatus,
    /// Queries a handle to determine if it supports a specified protocol.
    pub handle_protocol: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut *mut c_void,
    ) -> EfiStatus,
    /// Reserved field. Must be `null`.
    pub reserved: *mut c_void,
    /// Creates an event that is to be signaled whenever an interface is installed
    /// for a specified protocol.
    pub register_protocol_notify: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        event: EfiEvent,
        registration: *mut *mut c_void,
    ) -> EfiStatus,
    /// Returns an array of handles that support a specified protocol.
    pub locate_handle: unsafe extern "efiapi" fn(
        search_type: LocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        buffer_size: *mut usize,
        buffer: *mut EfiHandle,
    ) -> EfiStatus,
    /// Locates the handle to a device on the device path that supports a specified protocol.
    pub locate_device_path: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        device_path: *mut *mut c_void,
        device: *mut EfiHandle,
    ) -> EfiStatus,
    /// Adds, updates, or removes a configuration table entry.
    pub install_configuration_table:
        unsafe extern "efiapi" fn(guid: *const EfiGuid, table: *mut c_void) -> EfiStatus,

    // ── Image Services ───────────────────────────────────────────
    /// Loads an EFI image into memory.
    pub load_image: unsafe extern "efiapi" fn(
        boot_policy: bool,
        parent_image_handle: EfiHandle,
        device_path: *mut c_void,
        source_buffer: *const c_void,
        source_size: usize,
        image_handle: *mut EfiHandle,
    ) -> EfiStatus,
    /// Transfers control to a loaded image's entry point.
    pub start_image: unsafe extern "efiapi" fn(
        image_handle: EfiHandle,
        exit_data_size: *mut usize,
        exit_data: *mut *mut u16,
    ) -> EfiStatus,
    /// Terminates a loaded EFI image and returns control to boot services.
    pub exit: unsafe extern "efiapi" fn(
        image_handle: EfiHandle,
        exit_status: EfiStatus,
        exit_data_size: usize,
        exit_data: *mut u16,
    ) -> EfiStatus,
    /// Unloads an image.
    pub unload_image: unsafe extern "efiapi" fn(image_handle: EfiHandle) -> EfiStatus,
    /// Terminates all boot services.
    pub exit_boot_services:
        unsafe extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus,

    // ── Miscellaneous Services ───────────────────────────────────
    /// Returns a monotonically increasing count for the platform.
    pub get_next_monotonic_count: unsafe extern "efiapi" fn(count: *mut u64) -> EfiStatus,
    /// Induces a fine-grained stall.
    pub stall: unsafe extern "efiapi" fn(microseconds: usize) -> EfiStatus,
    /// Sets the system's watchdog timer.
    pub set_watchdog_timer: unsafe extern "efiapi" fn(
        timeout: usize,
        watchdog_code: u64,
        data_size: usize,
        watchdog_data: *const u16,
    ) -> EfiStatus,

    // ── Driver Support Services ──────────────────────────────────
    /// Connects one or more drivers to a controller.
    pub connect_controller: unsafe extern "efiapi" fn(
        controller_handle: EfiHandle,
        driver_image_handle: *mut EfiHandle,
        remaining_device_path: *mut c_void,
        recursive: bool,
    ) -> EfiStatus,
    /// Disconnects one or more drivers from a controller.
    pub disconnect_controller: unsafe extern "efiapi" fn(
        controller_handle: EfiHandle,
        driver_image_handle: EfiHandle,
        child_handle: EfiHandle,
    ) -> EfiStatus,

    // ── Open and Close Protocol Services ─────────────────────────
    /// Opens a protocol interface on a handle.
    pub open_protocol: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut *mut c_void,
        agent_handle: EfiHandle,
        controller_handle: EfiHandle,
        attributes: u32,
    ) -> EfiStatus,
    /// Closes a protocol that was opened with `OpenProtocol`.
    pub close_protocol: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        agent_handle: EfiHandle,
        controller_handle: EfiHandle,
    ) -> EfiStatus,
    /// Retrieves the list of agents that currently have a protocol interface opened.
    pub open_protocol_information: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        entry_buffer: *mut *mut OpenProtocolInformationEntry,
        entry_count: *mut usize,
    ) -> EfiStatus,

    // ── Library Services ─────────────────────────────────────────
    /// Retrieves the list of protocol interface GUIDs installed on a handle.
    pub protocols_per_handle: unsafe extern "efiapi" fn(
        handle: EfiHandle,
        protocol_buffer: *mut *mut *mut EfiGuid,
        protocol_buffer_count: *mut usize,
    ) -> EfiStatus,
    /// Returns an array of handles that support the requested protocol.
    pub locate_handle_buffer: unsafe extern "efiapi" fn(
        search_type: LocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        no_handles: *mut usize,
        buffer: *mut *mut EfiHandle,
    ) -> EfiStatus,
    /// Returns the first protocol instance that matches the given protocol.
    pub locate_protocol: unsafe extern "efiapi" fn(
        protocol: *const EfiGuid,
        registration: *mut c_void,
        interface: *mut *mut c_void,
    ) -> EfiStatus,
    /// Installs one or more protocol interfaces on a handle.
    pub install_multiple_protocol_interfaces:
        unsafe extern "efiapi" fn(handle: *mut EfiHandle, ...) -> EfiStatus,
    /// Removes one or more protocol interfaces from a handle.
    pub uninstall_multiple_protocol_interfaces:
        unsafe extern "efiapi" fn(handle: EfiHandle, ...) -> EfiStatus,

    // ── 32-bit CRC Service ───────────────────────────────────────
    /// Computes and returns a 32-bit CRC for a data buffer.
    pub calculate_crc32: unsafe extern "efiapi" fn(
        data: *const c_void,
        data_size: usize,
        crc32: *mut u32,
    ) -> EfiStatus,

    // ── Memory Utility Services ──────────────────────────────────
    /// Copies the contents of one buffer to another buffer.
    pub copy_mem:
        unsafe extern "efiapi" fn(destination: *mut c_void, source: *const c_void, length: usize),
    /// Fills a buffer with a specified value.
    pub set_mem: unsafe extern "efiapi" fn(buffer: *mut c_void, size: usize, value: u8),

    // ── CreateEventEx ────────────────────────────────────────────
    /// Creates an event in a group.
    pub create_event_ex: unsafe extern "efiapi" fn(
        event_type: u32,
        notify_tpl: EfiTpl,
        notify_function: Option<unsafe extern "efiapi" fn(event: EfiEvent, context: *mut c_void)>,
        notify_context: *const c_void,
        event_group: *const EfiGuid,
        event: *mut EfiEvent,
    ) -> EfiStatus,
}

impl BootServices {
    /// Allocates `pages` pages of `memory_type` memory.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the firmware fails to allocate the requested memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure boot services are still active and that the
    /// returned memory is properly freed when no longer needed.
    pub unsafe fn allocate_pages(
        &self,
        alloc_type: EfiAllocateType,
        memory_type: EfiMemoryType,
        pages: usize,
    ) -> Result<EfiPhysicalAddress, EfiStatus> {
        let mut address: EfiPhysicalAddress = 0;
        let status =
            unsafe { (self.allocate_pages)(alloc_type, memory_type, pages, &raw mut address) };
        status.to_result().map(|()| address)
    }

    /// Frees `pages` pages starting at `memory`.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the firmware fails to free the memory.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `memory` was allocated by `allocate_pages`
    /// and that the page count matches.
    pub unsafe fn free_pages(
        &self,
        memory: EfiPhysicalAddress,
        pages: usize,
    ) -> Result<(), EfiStatus> {
        let status = unsafe { (self.free_pages)(memory, pages) };
        status.to_result()
    }

    /// Locates the first protocol instance matching `protocol`.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the protocol is not found.
    ///
    /// # Safety
    ///
    /// The caller must ensure boot services are still active and that the
    /// returned pointer is used according to the protocol's specification.
    pub unsafe fn locate_protocol(&self, protocol: &EfiGuid) -> Result<*mut c_void, EfiStatus> {
        let mut interface: *mut c_void = core::ptr::null_mut();
        let status =
            unsafe { (self.locate_protocol)(protocol, core::ptr::null_mut(), &raw mut interface) };
        status.to_result().map(|()| interface)
    }

    /// Stalls the processor for the given number of microseconds.
    ///
    /// # Errors
    ///
    /// Returns `Err(EfiStatus)` if the stall operation fails.
    ///
    /// # Safety
    ///
    /// The caller must ensure boot services are still active.
    pub unsafe fn stall(&self, microseconds: usize) -> Result<(), EfiStatus> {
        let status = unsafe { (self.stall)(microseconds) };
        status.to_result()
    }
}

// ── Runtime Services ─────────────────────────────────────────────────

/// The UEFI Runtime Services Table.
///
/// Runtime services remain available after `ExitBootServices` has been called.
/// They provide access to time, variable storage, virtual memory mapping, and
/// system reset functionality.
#[repr(C)]
pub struct RuntimeServices {
    /// The table header.
    pub header: TableHeader,

    // ── Time Services ────────────────────────────────────────────
    /// Returns the current time and date, and the time-keeping capabilities
    /// of the hardware platform.
    pub get_time: unsafe extern "efiapi" fn(
        time: *mut EfiTime,
        capabilities: *mut EfiTimeCapabilities,
    ) -> EfiStatus,
    /// Sets the current local time and date information.
    pub set_time: unsafe extern "efiapi" fn(time: *const EfiTime) -> EfiStatus,
    /// Returns the current wakeup alarm clock setting.
    pub get_wakeup_time: unsafe extern "efiapi" fn(
        enabled: *mut bool,
        pending: *mut bool,
        time: *mut EfiTime,
    ) -> EfiStatus,
    /// Sets the system wakeup alarm clock time.
    pub set_wakeup_time: unsafe extern "efiapi" fn(enable: bool, time: *const EfiTime) -> EfiStatus,

    // ── Virtual Memory Services ──────────────────────────────────
    /// Changes the runtime addressing mode of EFI firmware from physical to virtual.
    pub set_virtual_address_map: unsafe extern "efiapi" fn(
        memory_map_size: usize,
        descriptor_size: usize,
        descriptor_version: u32,
        virtual_map: *mut u8,
    ) -> EfiStatus,
    /// Determines the new virtual address that is after the virtual address
    /// map has been applied.
    pub convert_pointer:
        unsafe extern "efiapi" fn(debug_disposition: usize, address: *mut *mut c_void) -> EfiStatus,

    // ── Variable Services ────────────────────────────────────────
    /// Returns the value of a variable.
    pub get_variable: unsafe extern "efiapi" fn(
        variable_name: *const u16,
        vendor_guid: *const EfiGuid,
        attributes: *mut u32,
        data_size: *mut usize,
        data: *mut c_void,
    ) -> EfiStatus,
    /// Enumerates the current variable names.
    pub get_next_variable_name: unsafe extern "efiapi" fn(
        variable_name_size: *mut usize,
        variable_name: *mut u16,
        vendor_guid: *mut EfiGuid,
    ) -> EfiStatus,
    /// Sets the value of a variable.
    pub set_variable: unsafe extern "efiapi" fn(
        variable_name: *const u16,
        vendor_guid: *const EfiGuid,
        attributes: u32,
        data_size: usize,
        data: *const c_void,
    ) -> EfiStatus,

    // ── Miscellaneous Services ───────────────────────────────────
    /// Returns the next high 32 bits of the platform's monotonic counter.
    pub get_next_high_monotonic_count: unsafe extern "efiapi" fn(high_count: *mut u32) -> EfiStatus,
    /// Resets the entire platform.
    pub reset_system: unsafe extern "efiapi" fn(
        reset_type: ResetType,
        reset_status: EfiStatus,
        data_size: usize,
        reset_data: *const c_void,
    ) -> !,

    // ── Capsule Services ─────────────────────────────────────────
    /// Passes capsules to the firmware with both virtual and physical mapping.
    pub update_capsule: unsafe extern "efiapi" fn(
        capsule_header_array: *mut *mut CapsuleHeader,
        capsule_count: usize,
        scatter_gather_list: EfiPhysicalAddress,
    ) -> EfiStatus,
    /// Returns information about whether the platform can support capsule updates.
    pub query_capsule_capabilities: unsafe extern "efiapi" fn(
        capsule_header_array: *mut *mut CapsuleHeader,
        capsule_count: usize,
        maximum_capsule_size: *mut u64,
        reset_type: *mut ResetType,
    ) -> EfiStatus,

    // ── Variable Information ─────────────────────────────────────
    /// Returns information about the UEFI variable store.
    pub query_variable_info: unsafe extern "efiapi" fn(
        attributes: u32,
        maximum_variable_storage_size: *mut u64,
        remaining_variable_storage_size: *mut u64,
        maximum_variable_size: *mut u64,
    ) -> EfiStatus,
}

impl RuntimeServices {
    /// Resets the system.
    ///
    /// # Safety
    ///
    /// This function does not return. The caller must ensure all necessary
    /// cleanup has been performed before calling.
    pub unsafe fn reset_system(&self, reset_type: ResetType, status: EfiStatus) -> ! {
        unsafe { (self.reset_system)(reset_type, status, 0, core::ptr::null()) }
    }
}

// ── Compile-time layout assertions ──────────────────────────────────

// Architecture-independent structs (no pointers)
const _: () = {
    assert!(core::mem::size_of::<TableHeader>() == 24);
    assert!(core::mem::size_of::<EfiTime>() == 16);
    assert!(core::mem::size_of::<EfiTimeCapabilities>() == 12);
    assert!(core::mem::size_of::<CapsuleHeader>() == 28);
};

// Architecture-dependent structs (contain pointers or usize)
#[cfg(target_pointer_width = "64")]
const _: () = {
    assert!(core::mem::size_of::<ConfigurationTable>() == 24);
    assert!(core::mem::size_of::<OpenProtocolInformationEntry>() == 24);

    // SystemTable: 4 bytes padding after firmware_revision (u32) before console_in_handle (ptr)
    assert!(core::mem::size_of::<SystemTable>() == 120);
    assert!(core::mem::offset_of!(SystemTable, header) == 0);
    assert!(core::mem::offset_of!(SystemTable, firmware_vendor) == 24);
    assert!(core::mem::offset_of!(SystemTable, firmware_revision) == 32);
    assert!(core::mem::offset_of!(SystemTable, console_in_handle) == 40);
    assert!(core::mem::offset_of!(SystemTable, console_in) == 48);
    assert!(core::mem::offset_of!(SystemTable, console_out_handle) == 56);
    assert!(core::mem::offset_of!(SystemTable, console_out) == 64);
    assert!(core::mem::offset_of!(SystemTable, standard_error_handle) == 72);
    assert!(core::mem::offset_of!(SystemTable, standard_error) == 80);
    assert!(core::mem::offset_of!(SystemTable, runtime_services) == 88);
    assert!(core::mem::offset_of!(SystemTable, boot_services) == 96);
    assert!(core::mem::offset_of!(SystemTable, number_of_table_entries) == 104);
    assert!(core::mem::offset_of!(SystemTable, configuration_table) == 112);

    // BootServices: header (24) + 44 fn-pointer-sized fields (44 × 8 = 352)
    assert!(core::mem::size_of::<BootServices>() == 376);
    // RuntimeServices: header (24) + 14 fn pointers (14 × 8 = 112)
    assert!(core::mem::size_of::<RuntimeServices>() == 136);
};
