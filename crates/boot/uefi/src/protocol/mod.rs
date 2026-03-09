//! UEFI protocol definitions.
//!
//! Protocols are the primary mechanism for inter-component communication in UEFI.
//! Each protocol is identified by a unique [`EfiGuid`](crate::EfiGuid) and provides
//! a set of function pointers for interacting with a specific class of firmware
//! service or hardware device.
//!
//! This module contains bindings for the following protocols:
//! - [`block_io`] — Block I/O Protocol for disk access
//! - [`device_path`] — Device Path Protocol for device identification
//! - [`file`] — Simple File System and File protocols for FAT file system access
//! - [`gop`] — Graphics Output Protocol for framebuffer access
//! - [`loaded_image`] — Loaded Image Protocol for image information
//! - [`simple_text`] — Simple Text Output Protocol for console output
//! - [`simple_text_input`] — Simple Text Input Protocol for keyboard input

pub mod block_io;
pub mod device_path;
pub mod file;
pub mod gop;
pub mod loaded_image;
pub mod simple_text;
pub mod simple_text_input;
