use core::ffi::c_void;

use crate::memory::{EfiAllocateType, EfiMemoryType};
use crate::table;
use crate::{EfiGuid, EfiHandle, EfiPhysicalAddress, EfiStatus};

use super::Protocol;
use super::memory::MemoryMap;

/// Safe wrapper around UEFI Boot Services.
pub struct BootServices<'st> {
    raw: &'st table::BootServices,
    image_handle: EfiHandle,
}

impl<'st> BootServices<'st> {
    pub(crate) fn new(raw: &'st table::BootServices, image_handle: EfiHandle) -> Self {
        Self { raw, image_handle }
    }

    /// Locate a protocol interface registered in the handle database.
    pub fn locate_protocol<P: Protocol>(&self) -> Result<&'st mut P::Raw, EfiStatus> {
        let mut interface: *mut c_void = core::ptr::null_mut();
        let status = unsafe {
            (self.raw.locate_protocol)(
                &P::GUID as *const EfiGuid,
                core::ptr::null_mut(),
                &mut interface,
            )
        };
        status.to_result()?;
        if interface.is_null() {
            return Err(EfiStatus::NOT_FOUND);
        }
        Ok(unsafe { &mut *(interface as *mut P::Raw) })
    }

    /// Allocate pages from the system memory.
    pub fn allocate_pages(
        &self,
        alloc_type: EfiAllocateType,
        memory_type: EfiMemoryType,
        pages: usize,
    ) -> Result<EfiPhysicalAddress, EfiStatus> {
        let mut address: EfiPhysicalAddress = 0;
        let status =
            unsafe { (self.raw.allocate_pages)(alloc_type, memory_type, pages, &mut address) };
        status.to_result()?;
        Ok(address)
    }

    /// Free previously allocated pages.
    pub fn free_pages(&self, address: EfiPhysicalAddress, pages: usize) -> Result<(), EfiStatus> {
        let status = unsafe { (self.raw.free_pages)(address, pages) };
        status.to_result()
    }

    /// Get the current UEFI memory map.
    ///
    /// The caller provides a buffer that will be filled with memory descriptors.
    /// The buffer should be aligned to `EfiMemoryDescriptor` alignment.
    pub fn get_memory_map<'buf>(&self, buf: &'buf mut [u8]) -> Result<MemoryMap<'buf>, EfiStatus> {
        let mut map_size = buf.len();
        let mut map_key: usize = 0;
        let mut desc_size: usize = 0;
        let mut desc_version: u32 = 0;

        let status = unsafe {
            (self.raw.get_memory_map)(
                &mut map_size,
                buf.as_mut_ptr(),
                &mut map_key,
                &mut desc_size,
                &mut desc_version,
            )
        };
        status.to_result()?;

        Ok(MemoryMap::new(
            &buf[..map_size],
            map_key,
            desc_size,
            desc_version,
        ))
    }

    /// Stall (busy-wait) for the given number of microseconds.
    pub fn stall(&self, microseconds: usize) -> Result<(), EfiStatus> {
        let status = unsafe { (self.raw.stall)(microseconds) };
        status.to_result()
    }

    /// Set the system watchdog timer.
    pub fn set_watchdog_timer(&self, timeout: usize, watchdog_code: u64) -> Result<(), EfiStatus> {
        let status =
            unsafe { (self.raw.set_watchdog_timer)(timeout, watchdog_code, 0, core::ptr::null()) };
        status.to_result()
    }

    /// Returns the image handle.
    pub fn image_handle(&self) -> EfiHandle {
        self.image_handle
    }
}
