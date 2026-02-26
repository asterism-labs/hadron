//! Block device implementations.
//!
//! Provides in-memory block devices (e.g., [`RamDisk`](ramdisk::RamDisk)) for
//! testing and scenarios that do not require real hardware.

pub mod ramdisk;
