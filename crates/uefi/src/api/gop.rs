use core::marker::PhantomData;

use crate::protocol::gop::{GraphicsOutputModeInformation, GraphicsOutputProtocol};
use crate::EfiStatus;

/// Safe wrapper around the UEFI Graphics Output Protocol.
pub struct Gop<'st> {
    raw: *mut GraphicsOutputProtocol,
    _lifetime: PhantomData<&'st ()>,
}

impl<'st> Gop<'st> {
    /// Create a new GOP wrapper from a protocol reference obtained via `locate_protocol`.
    pub fn new(raw: &'st mut GraphicsOutputProtocol) -> Self {
        Self {
            raw: raw as *mut _,
            _lifetime: PhantomData,
        }
    }

    /// Returns information about the current graphics mode.
    pub fn current_mode(&self) -> &GraphicsOutputModeInformation {
        unsafe {
            let mode = &*(*self.raw).mode;
            &*mode.info
        }
    }

    /// Returns the physical base address of the framebuffer.
    pub fn frame_buffer_base(&self) -> u64 {
        unsafe { (*(*self.raw).mode).frame_buffer_base }
    }

    /// Returns the size of the framebuffer in bytes.
    pub fn frame_buffer_size(&self) -> usize {
        unsafe { (*(*self.raw).mode).frame_buffer_size }
    }

    /// Set the graphics mode by mode number.
    pub fn set_mode(&self, mode_number: u32) -> Result<(), EfiStatus> {
        let status = unsafe { ((*self.raw).set_mode)(self.raw, mode_number) };
        status.to_result()
    }

    /// Query information about a specific mode number.
    pub fn query_mode(&self, mode_number: u32) -> Result<&GraphicsOutputModeInformation, EfiStatus> {
        let mut size: usize = 0;
        let mut info: *mut GraphicsOutputModeInformation = core::ptr::null_mut();
        let status = unsafe {
            ((*self.raw).query_mode)(self.raw, mode_number, &mut size, &mut info)
        };
        status.to_result()?;
        Ok(unsafe { &*info })
    }

    /// Returns the maximum mode number (exclusive upper bound for `set_mode` / `query_mode`).
    pub fn max_mode(&self) -> u32 {
        unsafe { (*(*self.raw).mode).max_mode }
    }
}
