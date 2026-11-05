//! Rust bindings with safe abstractions for the Limine bootloader protocol.
//!
//! This crate provides type-safe Rust bindings for the [Limine boot protocol](https://github.com/limine-bootloader/limine),
//! enabling kernels to communicate with the Limine bootloader to retrieve system information and
//! configure the boot environment. Used internally by Hadron OS.
//!
//! # Overview
//!
//! The Limine protocol works through a request-response mechanism:
//! 1. The kernel declares static request structures in a special section
//! 2. The bootloader fills in the corresponding response structures before passing control to the kernel
//! 3. The kernel can then query the responses to get information about the system
//!
//! # Architecture
//!
//! The crate is organized into several modules:
//! - [`request`] - Request structures that the kernel uses to ask for information
//! - [`response`] - Response structures that the bootloader fills in
//! - [`file`] - File representations provided by the bootloader
//! - [`framebuffer`] - Framebuffer and video mode structures
//! - [`memmap`] - Memory map entries and iterators
//! - [`module`] - Module definitions for kernel modules
//! - [`mp`] - Multiprocessor information for different architectures
//! - [`paging`] - Paging mode definitions
//!
//! # Usage Example
//!
//! ```no_run
//! use limine::*;
//!
//! // Declare requests as static variables
//! #[used]
//! #[link_section = ".requests"]
//! static BASE_REVISION: BaseRevision = BaseRevision::new();
//!
//! #[used]
//! #[link_section = ".requests"]
//! static MEMMAP_REQUEST: MemMapRequest = MemMapRequest::new();
//!
//! #[used]
//! #[link_section = ".requests"]
//! static FRAMEBUFFER_REQUEST: FramebufferRequest = FramebufferRequest::new();
//!
//! // In your kernel entry point, query the responses
//! fn kernel_main() {
//!     // Check base revision compatibility
//!     if !BASE_REVISION.is_supported() {
//!         panic!("Unsupported Limine protocol revision");
//!     }
//!
//!     // Get memory map
//!     if let Some(memmap_response) = MEMMAP_REQUEST.response() {
//!         for entry in memmap_response.entries() {
//!             // Process memory map entries
//!         }
//!     }
//!
//!     // Get framebuffer
//!     if let Some(fb_response) = FRAMEBUFFER_REQUEST.response() {
//!         for framebuffer in fb_response.framebuffers() {
//!             // Use framebuffer
//!         }
//!     }
//! }
//! ```
//!
//! # Features
//!
//! - Zero-cost abstractions over the raw Limine protocol
//! - Type-safe request and response handling
//! - Architecture-specific support (`x86_64`, `aarch64`, `riscv64`)
//! - Iterator-based interfaces for lists of entries
//! - No heap allocations required

#![no_main]
#![no_std]
#![feature(unsafe_cell_access)]

mod request;
mod response;

pub mod file;
pub mod framebuffer;
pub mod memmap;
pub mod module;
pub mod mp;
pub mod paging;

pub use request::*;
pub use response::*;
