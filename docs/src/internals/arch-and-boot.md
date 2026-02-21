# Architecture & Boot

This chapter documents how Hadron abstracts architecture-specific code and the
sequence of events from bootloader handoff to a running kernel with all CPUs
online.

## Overview

The architecture layer lives under `kernel/hadron-kernel/src/arch/`. A thin
facade in `arch/mod.rs` re-exports a uniform API from the active architecture
module (`arch/x86_64/` or `arch/aarch64/`). Three functions make up the
facade:

| Facade function         | Purpose |
|-------------------------|---------|
| `cpu_init()`            | Per-CPU setup: GDT, IDT, GS base, SYSCALL MSRs |
| `platform_init(boot_info)` | ACPI, PCI enumeration, interrupt controllers, timers, driver probing |
| `spawn_platform_tasks()` | Launch arch-specific async tasks after the executor starts |

All architecture-specific code is gated behind `#[cfg(target_arch = "...")]`,
and the facade functions dispatch to the correct implementation at compile
time. The rest of the kernel calls the facade -- never an arch module
directly -- with a few well-scoped exceptions in boot and SMP code.

### x86_64 Module Layout

```
arch/x86_64/
  mod.rs            Re-exports + module declarations
  gdt.rs            Global Descriptor Table + TSS
  idt.rs            Interrupt Descriptor Table wiring
  acpi.rs           ACPI table parsing, APIC setup, timer calibration
  smp.rs            Application Processor bootstrap
  syscall.rs        SYSCALL/SYSRET MSR programming + naked entry stub
  userspace.rs      Ring-3 entry helpers
  instructions/     Safe wrappers around x86 instructions
    interrupts.rs   CLI, STI, HLT, INT3, without_interrupts
    port.rs         Typed port I/O (Port<T>, ReadOnlyPort, WriteOnlyPort)
    segmentation.rs Segment register loads and reads
    tables.rs       LGDT, LIDT, LTR
    tlb.rs          INVLPG, full TLB flush
  registers/        CPU register accessors
    control.rs      CR0, CR2, CR3, CR4 (bitflags + read/write)
    model_specific.rs MSR read/write, IA32_EFER, STAR, LSTAR, SFMASK, GS_BASE
    rflags.rs       RFLAGS bitflags + pushfq reader
  structures/       Data-structure definitions consumed by instructions
    gdt.rs          GlobalDescriptorTable, Descriptor, SegmentSelector, TSS
    idt.rs          InterruptDescriptorTable, IDT entries
    paging.rs       PageTable, PageTableEntry, PageTableFlags
    machine_state.rs MachineState snapshot (for context save/restore)
  paging/           Page table mapper
    mapper.rs       PageTableMapper: map/unmap/translate for 4K/2M/1G pages
  hw/               Hardware controller drivers (kernel-internal)
    local_apic.rs   Local APIC MMIO wrapper
    io_apic.rs      I/O APIC: redirection entries, masking
    pic.rs          Legacy 8259 PIC remap and disable
    pit.rs          Programmable Interval Timer (calibration fallback)
    hpet.rs         High Precision Event Timer
    tsc.rs          Time Stamp Counter utilities
  interrupts/       Interrupt subsystem
    dispatch.rs     Handler table + stub generation for vectors 32-255
    handlers.rs     CPU exception handlers (vectors 0-31)
    timer_stub.rs   Custom naked timer stub with ring-3 preemption support
```


## GDT and TSS

**File:** `arch/x86_64/gdt.rs`

The GDT is built once at boot via a `LazyLock<(GlobalDescriptorTable, Selectors)>`.
It contains five entries in a specific order dictated by the SYSRET mechanism:

| Index | Descriptor | Selector |
|-------|------------|----------|
| 1     | Kernel code (64-bit) | `0x08` |
| 2     | Kernel data          | `0x10` |
| 3     | User data            | `0x18` |
| 4     | User code (64-bit)   | `0x20` |
| 5-6   | TSS (16-byte system descriptor) | `0x28` |

User data is placed *before* user code because `SYSRET` derives SS from
`STAR[63:48] + 8` and CS from `STAR[63:48] + 16`. With `STAR[63:48] = 0x10`,
this yields `SS = 0x18 | 3` (user data) and `CS = 0x20 | 3` (user code).

The `Selectors` struct caches all five selectors. On init, `gdt::init()`
loads the GDT with `lgdt`, reloads all segment registers (CS via `retfq`,
DS/SS to kernel data, ES/FS/GS to null), and loads the TSS with `ltr`.

### Task State Segment

The TSS provides two critical fields:

- **`privilege_stack_table[0]` (RSP0):** The stack pointer loaded on any
  interrupt or exception from ring 3. Initially set to an early BSS stack;
  replaced by a VMM-allocated guarded stack during `kernel_init`. Updated
  via `set_tss_rsp0()` during context switches.

- **`interrupt_stack_table[0]` (IST1):** A 16 KiB dedicated stack used
  exclusively by the double-fault handler. This ensures double faults
  are recoverable even if the kernel stack is corrupted.

### AP Initialization

Each Application Processor gets its own GDT and TSS via `gdt::init_ap()`.
The function heap-allocates both structures (leaked for `'static` lifetime),
allocates kernel and double-fault stacks through the VMM, and loads
everything the same way as the BSP.


## IDT and Exception Handling

**File:** `arch/x86_64/idt.rs`

The IDT is a single `LazyLock<InterruptDescriptorTable>` shared across all
CPUs (the IDT is immutable after construction). Initialization registers:

- **Vectors 0-31 (CPU exceptions):** Individual handler functions from
  `interrupts/handlers.rs`. The double-fault handler (vector 8) uses IST1
  and is diverging (never returns). The page-fault handler (vector 14)
  receives an error code.

- **Vectors 32-255 (hardware interrupts):** Macro-generated
  `extern "x86-interrupt"` stubs from `interrupts/dispatch.rs`. Each stub
  calls `dispatch_interrupt(vector)`, which performs a table lookup and
  sends LAPIC EOI. Vector 254 (LAPIC timer) is overridden with a custom
  naked stub (`timer_stub.rs`) that handles ring-3 preemption by saving
  user register state before entering the Rust handler.


## Interrupt Dispatch

**File:** `arch/x86_64/interrupts/dispatch.rs`

The dispatch subsystem maintains a static table of 224 `AtomicPtr<()>` slots
(vectors 32-255). Key types and functions:

```rust
type InterruptHandler = fn(u8);

fn register_handler(vector: u8, handler: InterruptHandler) -> Result<(), InterruptError>;
fn unregister_handler(vector: u8);
fn alloc_vector() -> Result<u8, InterruptError>;
```

Well-known vector assignments are defined in the `vectors` module:

| Constant        | Value | Purpose |
|-----------------|-------|---------|
| `TIMER`         | 254   | LAPIC timer |
| `SPURIOUS`      | 255   | Spurious interrupt |
| `DYNAMIC_START` | 48    | First dynamically allocable vector |
| `DYNAMIC_END`   | 239   | Last dynamically allocable vector |
| `IPI_START`     | 240   | First inter-processor interrupt vector |
| `IPI_END`       | 253   | Last inter-processor interrupt vector |

ISA IRQs 0-15 map to vectors 32-47 via `vectors::isa_irq_vector(irq)`.

Handler registration uses `compare_exchange` on the atomic pointer, preventing
double-registration. Drivers obtain vectors either by requesting a specific
ISA IRQ number or by calling `alloc_vector()` to claim the next free slot in
the dynamic range.


## ACPI Integration

**File:** `arch/x86_64/acpi.rs`

ACPI initialization is driven by `acpi::init(rsdp_phys)`, called from
`platform_init()`. It performs six steps:

1. **Parse ACPI tables** via the `hadron-acpi` crate using an `HhdmAcpiHandler`
   that translates physical addresses through the HHDM (`phys + hhdm_offset`).
   The RSDP leads to the RSDT/XSDT, from which the MADT, HPET, and MCFG are
   extracted.

2. **Disable legacy PIC** by remapping it to vectors 32-47 and masking all
   lines (`hw/pic.rs`).

3. **Initialize Local APIC.** The MADT provides the LAPIC base physical
   address, which is mapped as a 4 KiB MMIO region via the VMM. The LAPIC is
   enabled with the spurious interrupt vector (255), and TPR is set to 0
   (accept all priorities).

4. **Configure I/O APIC.** Each I/O APIC from the MADT is mapped via MMIO.
   All redirection entries are initially masked. ISA IRQs 0-15 are routed to
   the BSP using identity mapping (GSI = IRQ), with polarity and trigger mode
   adjusted according to MADT Interrupt Source Override entries. Entries remain
   masked until drivers explicitly unmask them.

5. **Initialize HPET** from the ACPI HPET table, if present. The HPET serves
   as the kernel's time source (`crate::time`) and is used for LAPIC timer
   calibration. If no HPET is available, the PIT is used as a fallback for
   calibration.

6. **Calibrate and start LAPIC timer.** A one-shot timer counts ticks over a
   10 ms HPET (or PIT) window to determine the LAPIC frequency. The timer is
   then started in periodic mode at ~1 kHz (1 ms interval). Calibration
   results (initial count and divide value) are stored in atomics so APs can
   start their timers with the same configuration.

The consolidated platform state (`AcpiPlatformState`) is stored in an
`IrqSpinLock<Option<...>>` for access by the interrupt dispatch path
(`send_lapic_eoi()`) and by drivers that need to unmask I/O APIC lines
(`with_io_apic()`).


## SMP Support

**File:** `arch/x86_64/smp.rs`

AP bootstrap uses a two-phase approach to avoid race conditions with
bootloader memory:

### Phase 1: Parking (boot stub)

Immediately after the boot stub switches CR3 to kernel page tables, it calls
`smp::park_aps()`. This writes `ap_early_park` as the entry address to each
AP's Limine `goto_address`, causing APs to leave the bootloader's spin loop.
Each AP:

1. Loads the kernel CR3 (stored in `AP_KERNEL_CR3`)
2. Reads its LAPIC ID from the bootloader's `MpInfo` struct
3. Increments `AP_PARKED_COUNT`
4. Spins on `AP_RELEASE`

At this point, APs are safely running on kernel page tables but have no
per-CPU state.

### Phase 2: Initialization (kernel\_init)

After the BSP completes platform init, `smp::boot_aps()` is called. For each AP:

1. Heap-allocates a `PerCpu` struct (leaked for `'static`)
2. Populates it with CPU ID and LAPIC ID
3. Stores its address in `AP_PERCPU_TABLE[lapic_id]`

Then `AP_RELEASE` is set, unblocking all parked APs simultaneously. Each AP
executes `ap_entry()`, which performs full initialization:

1. `gdt::init_ap()` -- allocates per-CPU GDT, TSS, kernel stack, double-fault stack
2. Sets `IA32_GS_BASE` and `IA32_KERNEL_GS_BASE` to the AP's `PerCpu` address
3. Loads the shared IDT
4. Initializes SYSCALL/SYSRET MSRs
5. Populates per-CPU pointers for assembly stubs (`user_context_ptr`, etc.)
6. Enables Local APIC and starts periodic timer with BSP-calibrated values
7. Signals readiness via `AP_READY_COUNT`
8. Enables interrupts and enters the executor loop

The BSP waits for all APs to signal readiness (with a spin-loop timeout)
before continuing.


## Paging

**Files:** `arch/x86_64/paging/mapper.rs`, `arch/x86_64/structures/paging.rs`

### Page Table Structures

`PageTable` is a `#[repr(C, align(4096))]` array of 512 `PageTableEntry`
values, matching the hardware layout. `PageTableEntry` wraps a `u64` and
provides:

- `address()` -- extracts the physical frame address (bits 12-51)
- `flags()` -- returns `PageTableFlags`
- `is_present()` -- tests the `PRESENT` bit

`PageTableFlags` is a `bitflags` type covering:
`PRESENT`, `WRITABLE`, `USER`, `WRITE_THROUGH`, `CACHE_DISABLE`,
`HUGE_PAGE`, `GLOBAL`, `NO_EXECUTE`.

### PageTableMapper

`PageTableMapper` is the central type for manipulating page tables. It holds
the HHDM offset and provides methods for all three page sizes:

| Method | Page size | Walk depth |
|--------|-----------|------------|
| `map_4k()` | 4 KiB | PML4 -> PDPT -> PD -> PT |
| `map_2mib()` | 2 MiB | PML4 -> PDPT -> PD (HUGE_PAGE) |
| `map_1gib()` | 1 GiB | PML4 -> PDPT (HUGE_PAGE) |
| `unmap_4k()` / `unmap_2mib()` / `unmap_1gib()` | Corresponding unmap |
| `translate()` | Any | Full walk, returns `TranslateResult` |
| `translate_addr()` | Any | Walk + offset calculation |
| `update_flags_*()` | Any | Modify PTE flags without changing address |

Intermediate page table entries (PML4E, PDPTE, PDE) are always created with
`PRESENT | WRITABLE`, plus `USER` if the leaf mapping includes `USER`.
Newly allocated intermediate tables are zeroed to prevent stale entries.

The mapper also implements the arch-independent `PageMapper<S>` and
`PageTranslator` traits (from `mm::mapper`), converting `MapFlags` to native
`PageTableFlags`. This allows the VMM to operate on page tables without
knowing the underlying architecture.


## Instructions and Registers

The kernel provides safe (or minimally-unsafe) Rust wrappers for every CPU
instruction it uses. All inline assembly lives exclusively in the
`instructions/` and `registers/` modules -- no other module is permitted to
contain `asm!` blocks for these operations.

### Instructions (`arch/x86_64/instructions/`)

**`interrupts.rs`** -- `enable()`, `disable()`, `are_enabled()`, `hlt()`,
`enable_and_hlt()`, `int3()`, and `without_interrupts(f)` (a
save/disable/restore guard).

**`port.rs`** -- Type-safe port I/O built on the `PortRead` / `PortWrite`
traits, implemented for `u8`, `u16`, and `u32`. Three port types:
- `Port<T>` -- read-write
- `ReadOnlyPort<T>` -- read-only
- `WriteOnlyPort<T>` -- write-only

All are `const`-constructible and store only the 16-bit port number.

**`segmentation.rs`** -- Load functions (`set_cs`, `load_ds`, `load_ss`,
`load_es`, `load_fs`, `load_gs`, `load_tss`) and read functions (`read_cs`,
`read_ds`, etc.). The `set_cs` implementation uses a far return (`retfq`)
because `mov cs, ...` is not valid in long mode.

**`tables.rs`** -- `lgdt()`, `lidt()`, `ltr()`.

**`tlb.rs`** -- `flush(addr)` (INVLPG for a single address) and `flush_all()`
(CR3 reload).

### Registers (`arch/x86_64/registers/`)

**`control.rs`** -- Types `Cr0`, `Cr2`, `Cr3`, `Cr4` with `read()` and
`write()` methods. `Cr0Flags` and `Cr4Flags` are bitflags types for the
relevant control bits. `Cr3::write()` accepts a `PhysAddr` and is used for
page table switching.

**`model_specific.rs`** -- `Msr` type with `read()` and `write()` methods
wrapping `rdmsr`/`wrmsr`. Pre-defined constants: `IA32_EFER`, `IA32_PAT`,
`MSR_STAR`, `MSR_LSTAR`, `MSR_SFMASK`, `IA32_GS_BASE`,
`IA32_KERNEL_GS_BASE`. `EferFlags` provides the `SYSTEM_CALL_ENABLE`,
`LONG_MODE_ENABLE`, and `NO_EXECUTE_ENABLE` bits.

**`rflags.rs`** -- `RFlags` bitflags and a `read()` function using
`pushfq; pop`.


## Per-CPU State

**File:** `kernel/hadron-kernel/src/percpu.rs`

### PerCpu Struct

Each CPU has a `PerCpu` instance accessed via the GS segment base. The struct
is `#[repr(C)]` with deterministic field offsets used by assembly stubs:

| Offset | Field | Type | Used by |
|--------|-------|------|---------|
| 0  | `self_ptr` | `u64` | `current_cpu()` reads `GS:[0]` |
| 8  | `kernel_rsp` | `u64` | SYSCALL entry stub: `mov rsp, gs:[8]` |
| 16 | `user_rsp` | `u64` | SYSCALL entry stub: `mov gs:[16], rsp` |
| 24 | `cpu_id` | `AtomicU32` | Logical CPU identifier |
| 28 | `apic_id` | `AtomicU8` | Local APIC ID |
| 29 | `initialized` | `AtomicBool` | Init-complete flag |
| 32 | `user_context_ptr` | `u64` | Timer preemption stub: `GS:[32]` |
| 40 | `saved_kernel_rsp_ptr` | `u64` | Timer preemption stub: `GS:[40]` |
| 48 | `trap_reason_ptr` | `u64` | Timer preemption stub: `GS:[48]` |
| 56 | `saved_regs_ptr` | `u64` | SYSCALL entry stub: `GS:[56]` |

### BSP vs AP Initialization

The BSP uses a static `BSP_PERCPU` instance (in BSS). During early boot,
`init_gs_base()` sets `IA32_GS_BASE` and `IA32_KERNEL_GS_BASE` to its
address, writes the `self_ptr`, and initializes `kernel_rsp` to an early
16 KiB BSS stack. Both MSRs are set to the same value so that `swapgs` is
a no-op before any user process exists.

APs receive heap-allocated `PerCpu` instances during `boot_aps()`. The AP
entry code sets GS base *after* GDT init (because `load_gs(null)` in GDT
init clears the GS base MSR on Intel CPUs).

### `CpuLocal<T>`

`CpuLocal<T>` wraps a `[T; MAX_CPUS]` array indexed by the current CPU ID
(obtained from `current_cpu().get_cpu_id()`). It provides:

- `get()` -- returns the current CPU's element
- `get_for(cpu_id)` -- returns a specific CPU's element

This is used for per-CPU state that needs static allocation, such as
`SYSCALL_SAVED_REGS` (user registers saved on syscall entry for blocking
syscall resume).


## Boot Flow

### BootInfo Trait

**File:** `kernel/hadron-kernel/src/boot.rs`

The `BootInfo` trait abstracts over different bootloaders. Each boot stub
converts its native data structures into kernel-canonical types and calls
`kernel_init()`. The trait provides:

```rust
trait BootInfo {
    fn memory_map(&self) -> &[MemoryRegion];
    fn hhdm_offset(&self) -> u64;
    fn kernel_address(&self) -> KernelAddressInfo;
    fn paging_mode(&self) -> PagingMode;
    fn framebuffers(&self) -> &[FramebufferInfo];
    fn rsdp_address(&self) -> Option<PhysAddr>;
    fn dtb_address(&self) -> Option<PhysAddr>;
    fn command_line(&self) -> Option<&str>;
    fn smbios_address(&self) -> (Option<PhysAddr>, Option<PhysAddr>);
    fn page_table_root(&self) -> PhysAddr;
    fn initrd(&self) -> Option<InitrdInfo>;
    fn smp_cpus(&self) -> &[SmpCpuEntry];
    fn bsp_lapic_id(&self) -> u32;
}
```

`BootInfoData` is the concrete container, using stack-allocated `ArrayVec`s
(max 256 memory regions, 4 framebuffers, 32 SMP CPUs) to avoid heap use
during early boot.

`SmpCpuEntry` describes an AP with its LAPIC ID and pointers to the
bootloader's `goto_address` and `extra_argument` fields. Its `start()` method
atomically launches an AP by writing the entry function address.

### kernel\_init Sequence

`kernel_init(boot_info)` is the single entry point for all bootloaders. It is
a diverging function (`-> !`) that performs initialization in a strict order:

| Step | Action | Key function |
|------|--------|--------------|
| 1 | CPU init (GDT, IDT, GS base, SYSCALL MSRs) | `arch::cpu_init()` |
| 2 | HHDM offset registration | `mm::hhdm::init()` |
| 2b | Backtrace support from embedded HKIF | `backtrace::init_from_embedded()` |
| 3 | PMM init (bitmap from memory map) | `mm::pmm::init()` |
| 4 | VMM init (wraps root page table) | `mm::vmm::init()` |
| 4b | Allocate guarded kernel stack, replace BSS stack | `vmm.alloc_kernel_stack()` |
| 5 | Heap allocator init | `mm::heap::init()` |
| 5b | Device registry init | `drivers::device_registry::init()` |
| 6 | Full logger init (replaces early serial) | `log::init_logger()` |
| 7 | Framebuffer log sink (if available) | `log::add_sink()` |
| 8 | Platform init (ACPI, PCI, drivers) | `arch::platform_init()` |
| 9 | Switch to device-registry framebuffer if probed | Bochs VGA check |
| 8b | SMP wakeup IPI init + boot APs | `sched::smp::init()`, `smp::boot_aps()` |
| 9b | Spawn platform tasks + heartbeat | `arch::spawn_platform_tasks()` |
| 10 | Extract initrd via HHDM | CPIO archive from bootloader |
| 10b | VFS init, mount ramfs + devfs + block devices | `fs::vfs::init()` |
| 11 | Console input init, kernel CR3 save | `fs::console_input::init()` |
| 12 | Populate BSP per-CPU assembly pointers | `user_context_ptr`, etc. |
| 13 | Spawn init process | `proc::spawn_init()` |
| 14 | Enable BSP interrupts | `instructions::interrupts::enable()` |
| 15 | Enter executor (never returns) | `sched::executor().run()` |

Interrupts remain disabled throughout the entire init sequence and are only
enabled at step 14, after all subsystems and APs are online. The LAPIC timer
is started earlier (during ACPI init) but interrupts are simply held pending
until STI.

### Boot Stub (Limine)

The Limine boot stub at `kernel/boot/limine/` implements `BootInfo` by
translating Limine protocol responses. Before calling `kernel_init`, the stub:

1. Reads the Limine memory map, framebuffers, RSDP, etc.
2. Switches CR3 to the kernel-owned page tables
3. Calls `smp::park_aps()` to move APs off bootloader page tables
4. Calls `kernel_init()` with the populated `BootInfoData`


## Key Types and Traits Summary

| Type / Trait | Location | Purpose |
|-------------|----------|---------|
| `BootInfo` | `boot.rs` | Bootloader abstraction trait |
| `BootInfoData` | `boot.rs` | Concrete boot info container |
| `MemoryRegion` / `MemoryRegionKind` | `boot.rs` | Physical memory map entries |
| `SmpCpuEntry` | `boot.rs` | Per-AP bootstrap descriptor |
| `PerCpu` | `percpu.rs` | Per-CPU state (GS-based access) |
| `CpuLocal<T>` | `percpu.rs` | Per-CPU array indexed by CPU ID |
| `GlobalDescriptorTable` | `structures/gdt.rs` | GDT builder |
| `Selectors` | `gdt.rs` | Cached GDT segment selectors |
| `TaskStateSegment` | `structures/gdt.rs` | TSS with RSP0 and IST |
| `InterruptDescriptorTable` | `structures/idt.rs` | IDT with per-vector configuration |
| `InterruptHandler` | `interrupts/dispatch.rs` | `fn(u8)` handler signature |
| `PageTableMapper` | `paging/mapper.rs` | HHDM-based page table walker |
| `PageTable` / `PageTableEntry` | `structures/paging.rs` | Hardware page table types |
| `PageTableFlags` | `structures/paging.rs` | PTE flag bits |
| `TranslateResult` | `paging/mapper.rs` | Translation outcome (4K/2M/1G/NotMapped) |
| `Port<T>` / `ReadOnlyPort<T>` / `WriteOnlyPort<T>` | `instructions/port.rs` | Typed port I/O |
| `Msr` | `registers/model_specific.rs` | Model-Specific Register accessor |
| `Cr0` / `Cr2` / `Cr3` / `Cr4` | `registers/control.rs` | Control register accessors |
| `RFlags` | `registers/rflags.rs` | CPU flags register |
| `LocalApic` | `hw/local_apic.rs` | LAPIC MMIO interface |
| `IoApic` | `hw/io_apic.rs` | I/O APIC configuration |
| `Hpet` | `hw/hpet.rs` | High Precision Event Timer |
| `AcpiPlatformState` | `acpi.rs` | LAPIC/IOAPIC base addresses |
| `SyscallSavedRegs` | `syscall.rs` | User registers saved on SYSCALL entry |
| `MachineState` | `structures/machine_state.rs` | Full register snapshot |
