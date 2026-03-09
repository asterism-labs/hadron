# Memory Layout

Hadron uses the standard x86_64 canonical address split: user processes occupy the lower canonical
half and the kernel occupies the upper canonical half. The boundary is enforced by hardware: any
address with bits 63:48 not matching bit 47 is non-canonical and generates a general protection
fault on dereference.

---

## Address Space Overview

```
0xFFFF_FFFF_FFFF_FFFF ─┐
                        │  Kernel Space (upper canonical half)
0xFFFF_8000_0000_0000 ─┘

  [ non-canonical gap: 0x0000_8000_0000_0000 – 0xFFFF_7FFF_FFFF_FFFF ]

0x0000_7FFF_FFFF_FFFF ─┐
                        │  User Space (lower canonical half)
0x0000_0000_0000_0000 ─┘
```

The non-canonical gap of approximately 127 TiB in the middle is inaccessible: any pointer that
falls in this range faults immediately. This provides a natural guard between user and kernel
pointers.

---

## User Space Layout

User space covers `0x0000_0000_0000_1000` to `0x0000_7FFF_FFFF_FFFF` (roughly 128 TiB). The
first page (`0x0000_0000_0000_0000`) is never mapped — a null pointer dereference always faults.

The layout within user space is managed by the VMAR (Virtual Memory Address Region) tree. Each
process has a root VMAR covering the entire user space. Child VMARs carve out sub-regions, and
VMO mappings are placed within leaf VMARs.

```
0x0000_7FFF_FFFF_FFFF ─┐
                        │  Stack (grows downward, per-thread)
                        │
                        │  ... (unmapped gap, detected by guard page)
                        │
                        │  mmap region (anonymous, shared memory, file maps)
                        │
                        │  Heap (brk grows upward)
                        │
                        │  Loaded ELF segments (.bss, .data, .rodata, .text)
                        │
0x0000_0000_0040_0000 ─┘  (typical ELF load address)
                        │
0x0000_0000_0000_1000 ─┘  (null guard page)
```

The VMAR tree enforces that no two mappings overlap. The kernel does not impose a fixed layout
beyond requiring the null guard and keeping the stack below the top of user space. The ELF loader,
`mem_map`, and `mem_brk` are responsible for placing segments, heap, and anonymous mappings.

### VMAR Tree

The root VMAR is created when a process is created. VMO mappings are inserted into it via
`vmar_map`. A VMAR can be subdivided via `vmar_allocate` to create a child VMAR covering a
sub-range; the parent VMAR cannot be used to map over a child VMAR's range.

A child VMAR can be destroyed with `vmar_destroy`, which atomically unmaps all mappings within it.
This is used by the ELF loader to tear down a failed load without leaking partial mappings.

---

## Kernel Space Layout

Kernel space occupies `0xFFFF_8000_0000_0000` to `0xFFFF_FFFF_FFFF_FFFF` (64 TiB). It is divided
into fixed regions defined by the `MemoryLayout` struct in `kernel/mm/src/layout.rs`.

### Higher Half Direct Map (HHDM)

```
Base:     0xFFFF_8000_0000_0000  (set by UEFI boot stub)
Size:     covers all physical memory (e.g., 16 GiB for a 16 GiB machine)
```

The HHDM is an identity-offset mapping of all physical memory. Physical address `P` is accessible
at virtual address `HHDM_BASE + P`. This allows any kernel code to convert between physical and
virtual addresses with a single addition or subtraction, without needing a separate page table
walk.

The boot stub maps the HHDM using 2 MiB huge pages (PD entries with the PSE bit set) wherever
the physical memory range is at least 2 MiB in size. 4 KiB pages are used for the remaining
fractions. Using huge pages reduces the number of TLB entries consumed by the HHDM by a factor of
512 and makes physical memory access fast on systems with large amounts of RAM.

ACPI tables, VT-d context tables, per-CPU LAPIC mappings, and all other physical-address-based
accesses go through the HHDM.

### Kernel Regions (KASLR-ready)

The remaining kernel regions are defined relative to a `regions_base` address. In a non-KASLR
build this is the constant `0xFFFF_C000_0000_0000`. A future KASLR implementation will randomize
this base at boot time.

```
regions_base + 0 TiB   ─┐
                         │  Kernel Heap  (max 2 TiB)
regions_base + 2 TiB   ─┘

regions_base + 8 TiB   ─┐
                         │  Kernel Stacks  (max 512 GiB)
regions_base + 8.5 TiB ─┘

regions_base + 16 TiB  ─┐
                         │  MMIO Mappings  (max 1 TiB)
regions_base + 17 TiB  ─┘

regions_base + 32 TiB  ─┐
                         │  Per-CPU Data  (max 1 TiB)
regions_base + 33 TiB  ─┘

regions_base + 48 TiB  ─┐
                         │  vDSO / VVAR  (max 2 MiB)
regions_base + 48 TiB  ─┘
```

### Kernel Image

```
Base:     0xFFFF_FFFF_8000_0000  (fixed, not KASLR-shifted)
Max size: 128 MiB
```

The kernel image is linked to a fixed high address and is not subject to KASLR. The `.text` and
`.rodata` sections are mapped read-only (or read-execute); `.data` and `.bss` are read-write.

Placing the image at a fixed address simplifies the boot stub's page table setup and ensures
that any kernel code pointer with bits 63:31 all set is recognizable as a kernel address.

### Per-CPU Stacks

Each CPU (BSP and APs) has a kernel stack allocated within the stacks region. Stacks are typically
64 KiB with a 4 KiB guard page immediately below the base to catch overflows. The TSS `RSP0` field
points to the top of each CPU's kernel stack.

IST (Interrupt Stack Table) stacks for double faults and NMIs are separate, smaller stacks
(8 KiB) allocated at fixed positions during early boot. Using separate IST stacks ensures that
a stack overflow does not prevent the double-fault handler from running.

### MMIO Region

Device MMIO ranges (PCI BARs) are mapped on demand into the kernel MMIO region when a driver
creates an `MmioFrame` VMO. The MMIO mapper allocates virtual address ranges sequentially within
the 1 TiB MMIO window and maps them as uncacheable (PAT = UC).

### Per-CPU Data

The per-CPU region holds the CPU-local executor state, GDT, TSS, and the `CpuLocal<T>` variables
used by synchronization primitives, scheduler queues, and interrupt counters. Access to per-CPU
data uses `GS_BASE` (the `gs` segment register is set to point to the current CPU's base address
on context switch).

---

## Page Table Structure

x86_64 uses a four-level page table hierarchy (five-level with LA57, not currently enabled):

```
CR3 → PML4 (Page Map Level 4)
        └── PDPT (Page Directory Pointer Table)
               └── PD (Page Directory)
                     └── PT (Page Table)
                           └── Physical Page Frame
```

Each level is a 4 KiB page containing 512 eight-byte entries. An entry at any level can be a
"large page" entry that maps a 1 GiB (PDPT), 2 MiB (PD), or 4 KiB (PT) region directly.

| Level | Coverage per entry | Typical use |
|-------|-------------------|-------------|
| PML4  | 512 GiB | One entry per 512 GiB region |
| PDPT  | 1 GiB   | 1 GiB huge pages for HHDM (optional) |
| PD    | 2 MiB   | 2 MiB huge pages for HHDM |
| PT    | 4 KiB   | Standard pages for kernel heap, user mappings |

The kernel's page table mapper is in `kernel/mm/src/mapper.rs`. It walks the table via HHDM
addresses (using `phys_to_virt` to follow each level's physical pointer) and allocates new table
pages from the PMM when needed.

### Kernel vs. User Page Tables

The kernel maintains one set of kernel page table entries (HHDM, kernel image, heap, stacks, MMIO,
per-CPU) shared across all processes. Each process has its own PML4 page. The upper half of the
PML4 (entries 256–511, covering addresses `0xFFFF_8000_0000_0000` and above) is synchronized
across all processes so that kernel mappings are visible from any process context.

When a process is created, its PML4 is allocated and the upper-half entries are copied from the
kernel's master PML4. When the kernel adds a new mapping in the upper half (e.g., a new MMIO
device), it updates the master and all existing process PML4s.

The lower half of the PML4 (entries 0–255) is process-private and contains only user space
mappings. Context switching (`mov cr3, <new_pml4>`) invalidates TLB entries for user mappings
automatically; kernel TLB entries with the global bit (`PGE`) are retained.

---

## Summary Table

| Region | Virtual Range | Size Limit | Page Size | Notes |
|--------|--------------|------------|-----------|-------|
| User space | `0x0001_0000` – `0x7FFF_FFFF_FFFF` | ~128 TiB | 4 KiB | VMAR-managed per process |
| HHDM | `0xFFFF_8000_0000_0000` + phys | All of RAM | 2 MiB / 4 KiB | Boot stub maps at init |
| Kernel heap | `0xFFFF_C000_0000_0000` | 2 TiB | 4 KiB | Grows on demand |
| Kernel stacks | `+8 TiB` | 512 GiB | 4 KiB | Per-CPU, guard pages |
| MMIO mappings | `+16 TiB` | 1 TiB | 4 KiB | Uncacheable (PAT=UC) |
| Per-CPU data | `+32 TiB` | 1 TiB | 4 KiB | GS_BASE-addressed |
| vDSO / VVAR | `+48 TiB` | 2 MiB | 4 KiB | Mapped into user space too |
| Kernel image | `0xFFFF_FFFF_8000_0000` | 128 MiB | 4 KiB | Fixed, not KASLR |
