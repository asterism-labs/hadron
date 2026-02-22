# hadron-boot-limine

Limine bootloader entry point for the Hadron kernel. This binary crate declares Limine protocol requests, converts bootloader responses into the kernel's `BootInfo` types, builds kernel-owned page tables with per-section permissions, switches CR3, and calls `kernel_init` to enter the kernel proper.

## Features

- Initializes COM1 serial output before any other boot step for early debug logging
- Builds kernel page tables with precise per-section permissions: `.text` (executable, read-only), `.rodata` (read-only, NX), `.data`/`.bss` (read-write, NX)
- Maps the full physical address space via 2 MiB huge pages into the Higher Half Direct Map (HHDM) with sequential PD fill for efficient O(n/512) table allocation
- Maps framebuffer regions with write-combine caching via PAT entry programming
- Configures CPU control bits: EFER.NXE, CR4.PGE, CR0.WP, and PAT MSR
- Collects ACPI RSDP, SMBIOS, DTB, framebuffer, SMP, and initrd information from Limine responses into a unified `BootInfoData` struct
- Parks Application Processors on kernel page tables immediately after SMP discovery to prevent execution environment corruption during BSP initialization
