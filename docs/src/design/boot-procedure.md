# Boot Procedure

This document defines the contract between boot stubs and the kernel, the exact CPU state required at `kernel_init` entry, and the step-by-step procedure each boot stub follows to establish that state.

**Scope**: x86_64 only. aarch64 boot details will be added later.

## CPU State Contract for `kernel_init`

Every boot stub must establish the following machine state before calling `kernel_init`. Any deviation is a bug.

### Paging

- Paging enabled (CR0.PG = 1)
- Kernel-owned 4-level page tables loaded in CR3 (5-level if LA57)
- **HHDM**: All physical memory mapped at `phys + HHDM_OFFSET` using 2 MiB huge pages
- **Kernel image**: Mapped at `0xFFFF_FFFF_8000_0000` with correct permissions:
  - `.text` = Read + Execute (no write)
  - `.rodata` = Read only (no write, no execute)
  - `.data`, `.bss` = Read + Write + NX (no execute)
- **Identity map**: Boot stub's transition code identity-mapped in low memory (a single 2 MiB huge page is sufficient). The kernel removes this mapping after taking control.
- **Framebuffer**: Mapped at its virtual address with uncacheable (UC) or write-combining (WC) memory type, if a framebuffer is present.

### GDT

Loaded with at minimum:

| Index | Selector | Description |
|-------|----------|-------------|
| 0     | `0x00`   | Null descriptor |
| 1     | `0x08`   | Kernel code (64-bit, DPL 0) |
| 2     | `0x10`   | Kernel data (DPL 0) |

TSS and user-mode segments are deferred to kernel init.

### Segment Registers

| Register | Value |
|----------|-------|
| CS       | `0x08` (kernel code selector) |
| SS       | `0x10` (kernel data selector) |
| DS       | `0x10` (kernel data selector) |
| ES       | `0x10` (kernel data selector) |
| FS       | `0x00` |
| GS       | `0x00` |

### Control Registers

| Register | Bits | Value | Purpose |
|----------|------|-------|---------|
| CR0      | PG   | 1     | Paging enabled |
| CR0      | WP   | 1     | Write protect (ring 0 respects read-only pages) |
| CR0      | PE   | 1     | Protected mode enabled |
| CR4      | PAE  | 1     | Physical address extension (required for long mode) |
| CR4      | PGE  | 1     | Global pages (kernel mappings survive TLB flush) |
| CR4      | OSFXSR | set if SSE target | SSE support (depends on target spec) |
| CR4      | OSXMMEXCPT | set if SSE target | SSE exceptions (depends on target spec) |
| CR4      | LA57 | conditional | Set only if 5-level paging is active |
| EFER     | NXE  | 1     | NX/XD bit support enabled |

### Other State

| Item | State |
|------|-------|
| Interrupts | Disabled (`CLI`). IDT setup is the kernel's responsibility. |
| Stack | RSP points to valid, mapped memory in the higher half. |
| Processor | BSP only. APs are not started. |
| FPU/SSE | Per target spec. `x86_64-unknown-hadron` uses `soft-float`, so no SSE state is required. |
| Serial | COM1 initialized for output (panic handler depends on this). |

## General Boot Steps

Every boot stub follows this sequence, regardless of the underlying bootloader or firmware:

1. **Early serial init** — Enable COM1 for panic output immediately, before anything that might panic.
2. **Collect boot data** — Gather memory map, framebuffer info, RSDP address, kernel load addresses, HHDM offset, and other system information from the bootloader or firmware.
3. **Allocate frames for page tables** — Obtain physical 4 KiB frames for page table construction. Source depends on the stub (bootloader memory services, scanning the memory map for `Usable` regions, etc.).
4. **Build kernel page tables** — Create a PML4 with:
   - **HHDM region**: All usable physical memory mapped with 2 MiB huge pages at `phys + HHDM_OFFSET`
   - **Kernel image region**: `.text`, `.rodata`, `.data`, `.bss` mapped with correct permissions using linker-provided section boundary symbols
   - **Framebuffer mapping**: If present, mapped with uncacheable attributes
   - **Identity map**: Current boot stub code identity-mapped for the CR3 switch transition
5. **Configure CPU state** — Load the minimal GDT, set EFER.NXE, set CR4 flags (PAE, PGE, etc.), set CR0.WP.
6. **Switch CR3** — Write the physical address of the new PML4 to CR3. TLB is implicitly flushed. The identity map allows execution to continue across the switch.
7. **Populate `BootInfoData`** — Convert all collected boot data into kernel types.
8. **Call `kernel_init(&boot_info)`** — Transfer control to the kernel. This call never returns.

## Limine Boot Stub

**Source**: `kernel/boot/limine/src/main.rs`

### Starting State

When the Limine boot stub's `_start` entry point runs:

- 64-bit long mode is active
- Paging is enabled with Limine-owned page tables
- HHDM is mapped (all physical memory at `phys + HHDM_OFFSET`)
- Kernel image is loaded and mapped at `0xFFFF_FFFF_8000_0000`
- Low memory is identity-mapped
- Interrupts are disabled

### The Problem

Limine's page tables are **not owned by the kernel**. The Limine protocol specification does not guarantee the internal layout of these tables, nor does it give the kernel write access to modify them. The kernel must not depend on Limine's page tables persisting beyond boot. To fully control paging (add mappings, modify permissions, implement per-process address spaces), the kernel needs its own page table hierarchy.

### Strategy

Build a new PML4 from scratch using physical frames found in the Limine-provided memory map (regions marked `Usable`). Use Limine's existing HHDM to read/write physical memory (the new page table frames are accessed via `HHDM_OFFSET + phys`). Once the new tables are ready, atomically switch CR3.

### Detailed Steps

1. **Init serial** — Initialize COM1 for early panic output. Verify `BASE_REVISION` is supported. (This is the existing code in `_start`.)

2. **Read HHDM offset** — Query `HHDM_REQUEST.response()` to get `hhdm_base`. This offset will be replicated in the new page tables.

3. **Read memory map** — Query `MEMMAP_REQUEST.response()` to get the physical memory map. Each entry has `base`, `length`, and `type_` (from `limine::memmap::MemMapEntryType`).

4. **Find free physical frames** — Scan the memory map for `Usable` regions. Implement a simple bump allocator that allocates 4 KiB frames from the **end** of usable regions (to avoid stomping on important structures in low memory).
   - Each page table level requires one 4 KiB physical frame
   - Typical frame count for 256 MiB RAM: PML4 (1) + PDPT for HHDM (~1-4) + PD entries for HHDM (~N based on RAM size) + kernel mapping tables (~4-8) = under 32 frames total

5. **Zero allocated frames** — Every allocated page table frame must be zeroed before use. Access them through the existing HHDM: write zeros to `hhdm_base + phys_addr`.

6. **Build PML4**:

   - **HHDM mapping**: For each physical memory region (usable, reserved, ACPI, etc.), create 2 MiB huge page entries at virtual address `HHDM_OFFSET + phys`. Walk PML4 -> PDPT -> PD, allocating intermediate table frames as needed. Set the PS (Page Size) bit in PD entries for 2 MiB pages. Flags: Present + Writable + Global + NX.

   - **Kernel image mapping**: Read the kernel's physical and virtual base from `EXECUTABLE_ADDRESS_REQUEST.response()`. Use linker-provided section boundary symbols (`__text_start`/`__text_end`, `__rodata_start`/`__rodata_end`, `__data_start`/`__data_end`, `__bss_start`/`__bss_end`) to map each section with correct permissions:
     - `.text`: Present + Global (executable, read-only)
     - `.rodata`: Present + Global + NX (read-only, no execute)
     - `.data`/`.bss`: Present + Writable + Global + NX (read-write, no execute)

   - **Framebuffer**: If `FRAMEBUFFER_REQUEST.response()` provides a framebuffer, map it at its virtual address with Present + Writable + NX + PCD (Page Cache Disable) or PAT for write-combining.

   - **Identity map**: Map the physical address range containing the boot stub's CR3-switch code as a single 2 MiB huge page. This prevents a page fault when CR3 changes and the old tables disappear. The kernel removes this mapping later.

7. **Load GDT** — Set up a minimal GDT (null + kernel code + kernel data) and load it with `lgdt`. Reload segment registers: far jump to set CS, then `mov` to set SS, DS, ES.

8. **Set CPU control bits**:
   - Set EFER.NXE (MSR `0xC000_0080`, bit 11) to enable the NX bit in page table entries
   - Set CR4.PGE (bit 7) to enable global pages
   - Set CR0.WP (bit 16) to enforce write protection in ring 0

9. **Switch CR3** — Write the physical address of the new PML4 to CR3. This atomically switches to the kernel-owned page tables and flushes the TLB. The identity map ensures the next instruction fetch succeeds.

10. **Build `BootInfoData`** — Convert all Limine responses into kernel boot info types. This is the existing `build_boot_info()` logic, extended to include `page_table_root`.

11. **Mark page table frames** — In the memory map passed to the kernel, mark the physical frames used for the new page tables so the kernel's physical memory manager knows not to allocate them. Options:
    - Add them as `BootloaderReclaimable` entries (not reclaimable in practice since they hold the active page tables)
    - Add a new `MemoryRegionKind::PageTable` variant
    - Pass the frame list separately via `BootInfo`

12. **Call `kernel_init(&boot_info)`** — Never returns.

### Limine Request Types Used

| Request | Purpose |
|---------|---------|
| `BaseRevision` | Protocol version check |
| `HhdmRequest` | HHDM offset for physical memory access |
| `MemMapRequest` | Physical memory map (frame allocation + HHDM mapping) |
| `ExecutableAddressRequest` | Kernel physical/virtual base addresses |
| `PagingModeRequest` | 4-level vs 5-level paging mode |
| `FramebufferRequest` | Framebuffer address and mode info |
| `RsdpRequest` | ACPI RSDP address |
| `ExecutableCmdlineRequest` | Kernel command line |
| `DeviceTreeBlobRequest` | DTB address (aarch64) |
| `SmbiosRequest` | SMBIOS entry points |

## UEFI Boot Stub

**Source**: `kernel/boot/uefi/` (planned)

> The kernel loading mechanism (two-binary ELF loader vs single-binary UEFI PE/COFF app) is **TBD for Phase 2 implementation**.

### Starting State

- UEFI application running in 64-bit long mode
- Identity-mapped address space (virtual = physical)
- UEFI Boot Services available
- Firmware owns page tables
- The kernel is **not** at its link address (`0xFFFF_FFFF_8000_0000`) — it is wherever UEFI loaded it in physical memory

### The Problem

The UEFI stub must:
1. Gather all system information while Boot Services are still available
2. Exit Boot Services (which invalidates all firmware memory except `EfiRuntimeServicesData` and `EfiLoaderData`)
3. Build page tables from scratch and configure CPU state
4. Relocate or map the kernel to its link address in the higher half

Crucially, after `ExitBootServices`, the firmware's memory map is final but most firmware memory is no longer accessible. All page table frames must be pre-allocated.

### Strategy

Use UEFI Boot Services to allocate page table frames and gather system info **before** exiting boot services. Exit boot services. Then configure CPU state and switch to kernel-owned page tables.

### High-Level Steps

1. **`efi_main(handle, system_table)`** — Entry point. Wrap raw pointers with `SystemTable<Boot>` from `crates/uefi/`.

2. **Disable watchdog timer** — Prevent the firmware from resetting the machine during boot.

3. **Query GOP** — Use `GraphicsOutputProtocol` (via `crates/uefi/src/protocol/gop.rs`) to get framebuffer address and mode info.

4. **Find ACPI RSDP** — Search the UEFI Configuration Table for the ACPI 2.0 RSDP GUID.

5. **Choose HHDM offset** — Use `0xFFFF_8000_0000_0000` (matching Limine's typical choice for consistency).

6. **Get memory map** — Call `GetMemoryMap` to determine the physical memory extent and find regions for page table allocation.

7. **Pre-allocate page table frames** — Use `AllocatePages` with `EfiLoaderData` memory type. `EfiLoaderData` survives `ExitBootServices`, ensuring the page table frames remain valid.

8. **Build page tables** — Construct PML4 with:
   - HHDM mapping (all physical memory)
   - Kernel image mapped at `0xFFFF_FFFF_8000_0000`
   - Identity map of the transition code
   - Stack mapping in the higher half

9. **Exit Boot Services** — Use `SystemTable::exit_boot_services()` (in `crates/uefi/src/api/mod.rs`) with retry logic for stale map keys.

10. **Post-ExitBootServices setup**:
    - Init serial (COM1) for panic output
    - Load minimal GDT
    - Set EFER.NXE, CR4.PAE/PGE, CR0.WP
    - Switch CR3 to the new PML4
    - Jump to higher-half code (the kernel is now at its link address)

11. **Build `BootInfoData`** — Populate from collected data.

12. **Call `kernel_init(&boot_info)`** — Never returns.

### Open Questions (Phase 2)

- **Kernel loading mechanism**: Does the UEFI stub load a separate kernel ELF from the filesystem, or is the kernel compiled as a UEFI PE/COFF application?
- **Kernel placement**: Should the UEFI stub copy the kernel to a known physical address, or map it in place at whatever physical address UEFI chose?
- **RuntimeServices**: How should `SetVirtualAddressMap` be used (if at all) to allow the kernel to call UEFI Runtime Services after paging is reconfigured?

## BootInfo Interface Design

The `BootInfo` trait (defined in `kernel/hadron-kernel/src/boot.rs`) divides boot-time setup into two categories:

### Pre-Configured by the Boot Stub

These are **already done** when `kernel_init` is called. The kernel does not need to set them up:

| Item | Description |
|------|-------------|
| Paging | Kernel-owned page tables with HHDM and kernel image mappings loaded in CR3 |
| GDT | Minimal GDT loaded (null + kernel CS + kernel DS) |
| CPU feature bits | EFER.NXE, CR4.PGE, CR0.WP all set |
| Serial output | COM1 initialized for early panic output |

### Provided as Data via BootInfo

These are **data values** passed to the kernel for further initialization:

| Method | Type | Purpose |
|--------|------|---------|
| `memory_map()` | `&[MemoryRegion]` | Physical memory layout for PMM initialization |
| `hhdm_offset()` | `u64` | HHDM offset used in the pre-built page tables |
| `kernel_address()` | `KernelAddressInfo` | Physical and virtual base of the kernel image |
| `paging_mode()` | `PagingMode` | 4-level or 5-level (kernel needs this for its own page table code) |
| `framebuffer()` | `Option<&FramebufferInfo>` | Framebuffer info (already mapped by boot stub) |
| `rsdp_address()` | `Option<u64>` | ACPI RSDP physical address |
| `dtb_address()` | `Option<u64>` | Device tree blob address (aarch64) |
| `command_line()` | `Option<&str>` | Kernel command line |
| `smbios_address()` | `(Option<u64>, Option<u64>)` | SMBIOS 32-bit and 64-bit entry points |
| `page_table_root()` | `u64` | **New.** Physical address of the PML4 the boot stub created. The kernel needs this to modify or extend the active page tables. |

### Rationale

The boot stub handles paging setup rather than the kernel because:
- The kernel cannot safely modify page tables it doesn't own (Limine's tables have an unspecified layout)
- The UEFI stub must build tables before jumping to the higher half
- A clean ownership boundary means the kernel always starts in a known, consistent state regardless of which bootloader was used

## Code Changes Required

These changes are prescribed by this design and will be implemented separately:

| File | Change |
|------|--------|
| `kernel/hadron-kernel/src/boot.rs` | Rename `kernel_main` to `kernel_init`. Add `page_table_root() -> u64` to the `BootInfo` trait and `BootInfoData` struct. |
| `kernel/hadron-kernel/src/lib.rs` | Update `pub use boot::kernel_main` to `pub use boot::kernel_init`. |
| `kernel/boot/limine/src/main.rs` | Add page table construction, GDT loading, CR3 switch before calling `kernel_init`. |
| `targets/x86_64-unknown-hadron.ld` | Add section boundary symbols: `__text_start`/`__text_end`, `__rodata_start`/`__rodata_end`, `__data_start`/`__data_end`. (Currently only defines `__bss_start`, `__bss_end`, `__kernel_end`.) |
