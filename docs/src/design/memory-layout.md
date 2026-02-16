# Memory Layout

This chapter documents the x86_64 virtual and physical memory layout used by Hadron.

## x86_64 Virtual Address Space

x86_64 uses 48-bit virtual addresses with canonical form: addresses must have bits 48-63 either all zeros (lower half) or all ones (upper half). This creates a natural split between userspace and kernel space.

```
0xFFFF_FFFF_FFFF_FFFF  ┌──────────────────────────────┐
                       │                              │
                       │   Kernel Space (upper half)  │
                       │                              │
0xFFFF_8000_0000_0000  ├──────────────────────────────┤ ← Canonical boundary
                       │                              │
                       │   Non-canonical hole          │
                       │   (addresses are invalid)     │
                       │                              │
0x0000_8000_0000_0000  ├──────────────────────────────┤ ← Canonical boundary
                       │                              │
                       │   User Space (lower half)    │
                       │                              │
0x0000_0000_0000_0000  └──────────────────────────────┘
```

Each half provides 128 TiB of addressable space.

## Kernel Virtual Memory Map

```
0xFFFF_FFFF_FFFF_FFFF  ┌──────────────────────────────┐
                       │   Reserved                    │
0xFFFF_FFFF_8000_0000  ├──────────────────────────────┤
                       │   Kernel Image               │  ~16 MiB
                       │   .text, .rodata, .data, .bss│
0xFFFF_FFFF_8000_0000  ├──────────────────────────────┤  ← Kernel base (typical)
                       │   ...                        │
0xFFFF_F000_0000_0000  ├──────────────────────────────┤
                       │   vDSO / VVAR mapping         │  4 KiB - 8 KiB
0xFFFF_E000_0000_0000  ├──────────────────────────────┤
                       │   Per-CPU data regions        │  Per-CPU × N CPUs
0xFFFF_D000_0000_0000  ├──────────────────────────────┤
                       │   MMIO mappings               │  Device MMIO regions
                       │   (APIC, I/O APIC, VirtIO)   │
0xFFFF_C800_0000_0000  ├──────────────────────────────┤
                       │   Kernel Stacks              │  Per-task kernel stacks
                       │   (with guard pages)          │  64 KiB each + 4 KiB guard
0xFFFF_C000_0000_0000  ├──────────────────────────────┤
                       │   Kernel Heap                 │  Initial: 4 MiB, grows
0xFFFF_A000_0000_0000  ├──────────────────────────────┤
                       │   ...                        │
0xFFFF_8000_0000_0000  ├──────────────────────────────┤
                       │   HHDM (Higher Half Direct   │  Maps ALL physical memory
                       │   Map) — 1:1 phys→virt       │  at phys + HHDM_OFFSET
                       │   Provided by Limine         │
0xFFFF_8000_0000_0000  └──────────────────────────────┘ ← HHDM base
```

> **Note**: Exact addresses will be adjusted during implementation. The HHDM base is provided by the Limine bootloader and may vary.

## Higher Half Direct Map (HHDM)

The HHDM is a direct mapping of all physical memory into the kernel's virtual address space. Given a physical address `P`, the corresponding virtual address is `P + HHDM_OFFSET`.

```
Physical Memory:              HHDM Virtual:
0x0000_0000 ┌─────┐         HHDM_OFFSET + 0x0000_0000 ┌─────┐
            │ RAM │    ←→                                │ RAM │
0x1000_0000 ├─────┤         HHDM_OFFSET + 0x1000_0000 ├─────┤
            │ ... │    ←→                                │ ... │
            └─────┘                                      └─────┘
```

**Why HHDM?**
- Walking page tables requires reading physical memory — HHDM makes this trivial
- No need for recursive page table mapping
- Any physical address can be accessed immediately
- Limine provides this automatically, so the kernel doesn't need to set it up

**Translation functions**:
```rust
fn phys_to_virt(phys: PhysAddr) -> VirtAddr {
    VirtAddr::new(phys.as_u64() + HHDM_OFFSET)
}

fn virt_to_phys(virt: VirtAddr) -> PhysAddr {
    PhysAddr::new(virt.as_u64() - HHDM_OFFSET)
}
```

## User Virtual Memory Map

```
0x0000_7FFF_FFFF_F000  ┌──────────────────────────────┐
                       │   User Stack                  │  Grows downward
                       │   (starts at top, grows down) │
0x0000_7FFF_FFF0_0000  ├──────────────────────────────┤  ← Initial RSP
                       │   ...                        │
                       │   (available for mmap)        │
                       │   ...                        │
0x0000_7000_0000_0000  ├──────────────────────────────┤
                       │   vDSO mapping               │  Kernel-provided shared lib
0x0000_7000_0000_0000  ├──────────────────────────────┤
                       │   ...                        │
                       │   (mmap region — grows down)  │
                       │   Shared libraries, anon mmap │
                       │   ...                        │
0x0000_0001_0000_0000  ├──────────────────────────────┤
                       │   Heap (brk region)          │  Grows upward
0x0000_0000_0060_0000  ├──────────────────────────────┤  ← Program break
                       │   BSS (zero-initialized)      │
                       │   Data (initialized globals)  │
                       │   Text (code, read-only)      │
0x0000_0000_0040_0000  ├──────────────────────────────┤  ← ELF load address
                       │   Null page (unmapped)        │  Catches null derefs
0x0000_0000_0000_0000  └──────────────────────────────┘
```

### Address Regions

| Region | Start | End | Purpose |
|--------|-------|-----|---------|
| Null guard | `0x0000` | `0x0FFF` | Unmapped — traps null pointer dereferences |
| ELF load | `0x0040_0000` | varies | Default ELF base address |
| Heap (brk) | end of BSS | upward | `brk()`/`sbrk()` region |
| mmap | `~0x7000_0000_0000` | downward | `mmap()` allocations |
| vDSO | `~0x7000_0000_0000` | fixed | Kernel-provided shared library |
| Stack | `~0x7FFF_FFF0_0000` | upward (grows down) | User stack, ~8 MiB default |

## Page Table Structure

x86_64 uses 4-level page tables (with optional 5-level for 57-bit addresses):

```
CR3 → PML4 (Page Map Level 4)
       │
       ├── Entry 0-255: User space (lower half)
       │    └── PDPT → PD → PT → 4 KiB pages
       │
       └── Entry 256-511: Kernel space (upper half)
            └── PDPT → PD → PT → 4 KiB pages
            (Shared across all processes)
```

### Page Sizes

| Level | Size | Use Case |
|-------|------|----------|
| PT (4 KiB) | Standard pages | General use |
| PD (2 MiB) | Huge pages | Kernel HHDM, large allocations |
| PDPT (1 GiB) | Giant pages | HHDM for large memory systems |

### Kernel/User Split

The upper half of the PML4 (entries 256-511) is shared across all processes by copying these entries from the kernel's PML4 into every new process's PML4. This means:

- Kernel mappings are identical in every address space
- Switching between processes only affects the lower half
- No TLB flush needed for kernel entries (use GLOBAL flag)

## Physical Memory Layout (Typical QEMU)

```
0x0000_0000  ┌──────────────────┐
             │ Real Mode IVT    │  First 1 KiB
0x0000_0400  ├──────────────────┤
             │ BIOS Data Area   │
0x0000_0500  ├──────────────────┤
             │ Usable (low)     │
0x0008_0000  ├──────────────────┤
             │ EBDA / Reserved  │
0x000A_0000  ├──────────────────┤
             │ VGA Buffer       │
0x000C_0000  ├──────────────────┤
             │ BIOS ROM         │
0x0010_0000  ├──────────────────┤  ← 1 MiB mark
             │ Usable (main)    │  Bulk of RAM
             │ ...              │
0x0FE0_0000  ├──────────────────┤  ← ~254 MiB (for 256 MiB system)
             │ ACPI / Reserved  │
0x1000_0000  └──────────────────┘
```

The bootloader's memory map tells us exactly which regions are usable. The PMM bitmap allocator (Phase 3) tracks this.
