# uefi

`no_std` Rust bindings for the UEFI specification, providing both raw `#[repr(C)]` FFI types and safe wrapper APIs for interacting with UEFI firmware services.

## Features

- Complete System Table, Boot Services, and Runtime Services bindings with `extern "efiapi"` function pointers
- GUID type with well-known constants for common protocols (GOP, Simple Text, File System, Block I/O, Loaded Image)
- Memory types, descriptors, and attribute flags matching the UEFI memory model
- Protocol definitions for graphics output, console I/O, file system access, block I/O, and device paths
- Safe high-level API wrappers using type-state and RAII patterns for boot services, console, filesystem, GOP, and memory
- Status code type with all standard UEFI error/warning codes and `Result`-based error handling
