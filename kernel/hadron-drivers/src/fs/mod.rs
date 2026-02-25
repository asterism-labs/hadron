//! Filesystem implementations for Hadron OS.
//!
//! Provides concrete filesystem drivers that are registered via linker-section
//! entries and discovered by the kernel at boot.

#[cfg(hadron_driver_fs_fat)]
pub mod fat;
#[cfg(hadron_driver_fs_initramfs)]
pub mod initramfs;
#[cfg(hadron_driver_fs_iso9660)]
pub mod iso9660;
#[cfg(hadron_driver_fs_ramfs)]
pub mod ramfs;
