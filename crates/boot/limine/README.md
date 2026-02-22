# limine

Type-safe `no_std` Rust bindings for the Limine bootloader protocol, providing a request-response interface for kernels to retrieve system information and configure the boot environment.

## Features

- Zero-cost abstractions over the raw Limine protocol with `#[repr(C)]` types
- Request structures for memory map, framebuffer, HHDM, RSDP, MP info, paging mode, modules, and more
- Iterator-based access to memory map entries, framebuffers, CPU info, and loaded files
- Architecture-specific support for x86_64, AArch64, and RISC-V
- No heap allocations -- all data is provided by the bootloader in static memory
- Protocol revision negotiation via `BaseRevision` with start/end request markers
