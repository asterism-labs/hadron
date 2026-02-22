# Driver Model

Hadron uses a layered driver model that separates API definition (in `hadron-kernel`) from driver implementation (in `hadron-drivers`). Drivers register themselves into linker sections at compile time, and the kernel discovers and probes them at boot without any explicit dependency from the kernel on the driver crate.

## Architecture Overview

The driver API is defined in `kernel/hadron-kernel/src/driver_api/` and consists of four layers plus supporting infrastructure:

```
Layer 3 -- Interface traits       SerialPort, Framebuffer, BlockDevice, KeyboardDevice, ...
Layer 2 -- Category traits        PlatformDriver (lifecycle + probe pattern)
Layer 1 -- Base driver trait      Driver (identity + metadata via DriverInfo)
Layer 0 -- Resource types         IoPortRange, MmioRegion, IrqLine
           -------
           Capabilities           IrqCapability, MmioCapability, DmaCapability, ...
           Probe Contexts         PciProbeContext, PlatformProbeContext
           Lifecycle              ManagedDriver (suspend/resume/shutdown)
```

- **Layer 0** (`driver_api/resource.rs`) -- Resource types representing exclusive hardware claims. `IoPortRange` wraps a contiguous range of x86 I/O ports, `MmioRegion` wraps a mapped physical-to-virtual memory region, and `IrqLine` wraps a Global System Interrupt number. All have `unsafe` constructors enforcing that only the kernel can create them.

- **Layer 1** (`driver_api/driver.rs`) -- The `Driver` trait provides identity via `DriverInfo`, which carries a name, `DriverType` enum variant, and human-readable description. `DriverState` tracks the lifecycle externally: `Registered -> Probing -> Active -> Suspended <-> Active -> Shutdown` (or `Probing -> Failed`).

- **Layer 2** (`driver_api/category.rs`) -- Category traits define how a driver is discovered and initialized. Currently `PlatformDriver` is the sole category trait, providing an async `probe()` method that consumes a `Resources` associated type and returns `Self`.

- **Layer 3** (`driver_api/serial.rs`, `driver_api/framebuffer.rs`, `driver_api/block.rs`, `driver_api/input.rs`, `driver_api/hw.rs`) -- Interface traits describe device functionality. Key traits include `SerialPort` (async byte I/O), `Framebuffer` (pixel-level display), `BlockDevice` (async sector I/O), `KeyboardDevice`, `MouseDevice`, `InterruptController`, `ClockSource`, and `Timer`.

## Capability System

Defined in `driver_api/capability.rs`, capabilities are typed tokens that grant scoped access to kernel subsystems. Each capability has `pub(crate)` constructors, so only `hadron-kernel` can mint them -- drivers cannot forge access.

### Capability Types

| Type | Purpose |
|------|---------|
| `IrqCapability` | Register/unregister interrupt handlers, allocate vectors, mask/unmask I/O APIC entries, send EOI |
| `MmioCapability` | Map physical MMIO regions into kernel virtual space, translate physical-to-virtual via HHDM |
| `DmaCapability` | Allocate/free contiguous physical frames for DMA transfers |
| `PciConfigCapability` | Read/write PCI configuration space, scoped to a single BDF address |
| `TaskSpawner` | Spawn async tasks on the kernel executor |
| `TimerCapability` | Read the current timer tick count |

### Compile-Time Enforcement

The capability system uses a sealed trait pattern to provide compile-time enforcement:

```rust
// Sealed marker trait -- only capability types in hadron-kernel implement this
pub trait CapabilityToken: sealed::Sealed {}

// Proof that a context grants access to capability C
pub trait HasCapability<C: CapabilityToken> {
    fn get(&self) -> &C;
}

// Ergonomic accessor on all types
pub trait CapabilityAccess {
    fn capability<C: CapabilityToken>(&self) -> &C
    where
        Self: HasCapability<C>;
}
```

If a driver attempts `ctx.capability::<DmaCapability>()` but did not declare `Dma` in its capabilities list, the code fails to compile because the generated context type does not implement `HasCapability<DmaCapability>`.

### Runtime Auditing

`CapabilityFlags` is a `bitflags` bitmap stored in each driver's linker-section entry for runtime logging and auditing. The flags are: `IRQ`, `MMIO`, `DMA`, `PCI_CONFIG`, `TASK_SPAWNER`, `TIMER`.

## Probe Contexts

Defined in `driver_api/probe_context.rs`, probe contexts bundle capability tokens for driver initialization. The kernel constructs a context with the appropriate tokens before calling a driver's probe function.

**`PciProbeContext`** -- Contains `PciDeviceInfo` for the matched device, plus `PciConfigCapability` (scoped to the device's BDF), `IrqCapability`, `MmioCapability`, `DmaCapability`, `TaskSpawner`, and `TimerCapability`.

**`PlatformProbeContext`** -- Contains `IrqCapability`, `MmioCapability`, `TaskSpawner`, and `TimerCapability` (no PCI-specific fields).

The kernel's internal `pci_probe_context()` and `platform_probe_context()` functions mint fresh capability tokens and assemble these contexts. Drivers never see these constructors.

## The `#[hadron_driver]` Proc Macro

The `hadron-driver-macros` crate (at `crates/driver/hadron-driver-macros/`) provides the `#[hadron_driver(...)]` attribute macro that generates all boilerplate for driver registration. Drivers declare their capabilities, and the macro generates:

1. A per-driver context struct containing only the declared capability fields
2. `HasCapability<T>` impls for each declared capability
3. A `device()` method on the context (PCI drivers only)
4. The original impl block with `DriverContext` replaced by the generated type
5. A wrapper probe function that adapts the full probe context to the narrowed one
6. A `#[link_section]` static entry for the appropriate linker section

### Attribute Syntax

```rust
#[hadron_driver(
    name = "<driver-name>",
    kind = pci | platform,
    capabilities = [Irq, Mmio, Dma, PciConfig, Spawner, Timer],
    pci_ids = &ID_TABLE,          // required for kind = pci
    compatible = "<compat-str>",  // required for kind = platform
)]
impl DriverStruct {
    fn probe(ctx: DriverContext) -> Result<...Registration, DriverError> {
        // DriverContext is replaced by the generated type at compile time
    }
}
```

### PCI Driver Example (AHCI)

From `kernel/hadron-drivers/src/ahci/mod.rs`:

```rust
static ID_TABLE: [PciDeviceId; 2] = [
    PciDeviceId::new(0x8086, 0x2922),            // Intel ICH9 AHCI
    PciDeviceId::with_class_progif(0x01, 0x06, 0x01), // Any AHCI controller
];

struct AhciDriver;

#[hadron_driver(
    name = "ahci",
    kind = pci,
    capabilities = [Irq, Mmio, Dma, PciConfig],
    pci_ids = &ID_TABLE,
)]
impl AhciDriver {
    fn probe(ctx: DriverContext) -> Result<PciDriverRegistration, DriverError> {
        let info = ctx.device();
        let pci_config = ctx.capability::<PciConfigCapability>();
        let mmio_cap = ctx.capability::<MmioCapability>();
        let irq_cap = ctx.capability::<IrqCapability>();
        let dma = ctx.capability::<DmaCapability>();

        // Map BAR5 (ABAR), enable bus mastering, init HBA, enumerate ports...

        let mut devices = DeviceSet::new();
        devices.add_block_device(path, disk);
        Ok(PciDriverRegistration { devices, lifecycle: None })
    }
}
```

### What the Macro Generates

For the AHCI example above, the macro generates roughly:

```rust
// 1. Context struct with only declared capabilities
pub struct AhciDriverContext {
    device: PciDeviceInfo,
    irq: IrqCapability,
    mmio: MmioCapability,
    dma: DmaCapability,
    pci_config: PciConfigCapability,
}

// 2. HasCapability impls for each declared capability
impl HasCapability<IrqCapability> for AhciDriverContext { ... }
impl HasCapability<MmioCapability> for AhciDriverContext { ... }
// ...

// 3. device() method
impl AhciDriverContext {
    pub fn device(&self) -> &PciDeviceInfo { &self.device }
}

// 4. Wrapper function adapting PciProbeContext -> AhciDriverContext
fn __ahci_driver_probe_wrapper(ctx: PciProbeContext) -> Result<...> {
    let ctx = AhciDriverContext {
        device: ctx.device,
        irq: ctx.irq,
        mmio: ctx.mmio,
        dma: ctx.dma,
        pci_config: ctx.pci_config,
    };
    AhciDriver::probe(ctx)
}

// 5. Linker-section static entry
#[used]
#[link_section = ".hadron_pci_drivers"]
static __AHCIDRIVER_PCI_ENTRY: PciDriverEntry = PciDriverEntry {
    name: "ahci",
    id_table: &ID_TABLE,
    capabilities: CapabilityFlags::IRQ | CapabilityFlags::MMIO | ...,
    probe: __ahci_driver_probe_wrapper,
};
```

## Linker-Section Registration

Drivers place static entry structs into dedicated linker sections. The linker collects all entries into contiguous arrays bounded by `__hadron_*_start` / `__hadron_*_end` symbols.

### Linker Sections

| Section | Entry Type | Purpose |
|---------|-----------|---------|
| `.hadron_pci_drivers` | `PciDriverEntry` | PCI device drivers |
| `.hadron_platform_drivers` | `PlatformDriverEntry` | Platform (firmware/hardcoded) drivers |
| `.hadron_block_fs` | `BlockFsEntry` | Block-device-backed filesystems (FAT, ISO 9660) |
| `.hadron_virtual_fs` | `VirtualFsEntry` | Memory-backed filesystems (ramfs) |
| `.hadron_initramfs` | `InitramFsEntry` | Initramfs archive unpackers (CPIO) |

### Entry Structures

**`PciDriverEntry`** (`driver_api/registration.rs`) -- Contains the driver `name`, an `id_table` slice of `PciDeviceId`, `CapabilityFlags`, and a `probe` function pointer receiving `PciProbeContext`.

**`PlatformDriverEntry`** -- Contains the driver `name`, a `compatible` string for matching, `CapabilityFlags`, and an `init` function pointer receiving `PlatformProbeContext`.

Both return a registration bundle (`PciDriverRegistration` / `PlatformDriverRegistration`) containing a `DeviceSet` and an optional `Arc<dyn ManagedDriver>` lifecycle handle.

### Filesystem Registration Macros

Filesystem entries use declarative macros rather than the proc macro:

```rust
block_fs_entry!(FAT_FS, BlockFsEntry { name: "fat", mount: fat_mount });
virtual_fs_entry!(RAMFS, VirtualFsEntry { name: "ramfs", create: ramfs_create });
initramfs_entry!(CPIO, InitramFsEntry { name: "cpio", unpack: cpio_unpack });
```

### Section Discovery

At boot, `drivers/registry.rs` reads each section by casting the linker-defined boundary symbols into typed slices:

```rust
pub fn pci_driver_entries() -> &'static [PciDriverEntry] {
    let start = addr_of!(__hadron_pci_drivers_start).cast::<PciDriverEntry>();
    let end = addr_of!(__hadron_pci_drivers_end).cast::<PciDriverEntry>();
    let count = end.offset_from(start) as usize;
    core::slice::from_raw_parts(start, count)
}
```

## Driver Matching and Probing

### PCI Driver Matching

`drivers/registry.rs::match_pci_drivers()` iterates all `PciDriverEntry` entries and, for each, tests every discovered `PciDeviceInfo` against the entry's `id_table`. `PciDeviceId::matches()` supports:

- Exact vendor/device ID matching
- Wildcard matching (`PCI_ANY_ID = 0xFFFF`)
- Subsystem vendor/device matching
- Class/subclass/prog-if matching with configurable mask

On a match, the kernel calls `probe_context::pci_probe_context()` to mint a fresh `PciProbeContext`, invokes the entry's `probe` function, and on success registers the returned devices in the device registry.

### Platform Driver Matching

`match_platform_drivers()` takes a list of `(name, compatible)` pairs and matches them against `PlatformDriverEntry::compatible` strings. On a match, it creates a `PlatformProbeContext` and calls the entry's `init` function.

## PCI Enumeration

PCI bus management is kernel infrastructure, located in `kernel/hadron-kernel/src/pci/`.

### Configuration Access Mechanism (`pci/cam.rs`)

The `PciCam` struct provides legacy I/O port access (ports `0xCF8` / `0xCFC`) to the 256-byte PCI configuration space. It exposes `read_u8`, `read_u16`, `read_u32`, and `write_u32` methods, all `unsafe` since they perform raw port I/O. Named register offsets are defined in the `regs` submodule.

### Bus Enumeration (`pci/enumerate.rs`)

`enumerate()` walks the PCI bus hierarchy:

1. Checks if the root host controller (0:0.0) is multi-function. If so, each function represents a separate bus domain.
2. For each bus, scans all 32 device slots.
3. For each device, reads the full `PciDeviceInfo` including BARs.
4. If a device is a PCI-to-PCI bridge (class 0x06, subclass 0x04), recursively enumerates the secondary bus.
5. If a device is multi-function (header type bit 7), scans functions 1-7.

### BAR Decoding

`decode_bars()` implements the standard PCI BAR sizing algorithm:

1. Save the original BAR value.
2. Write `0xFFFFFFFF` to determine the address mask.
3. Restore the original value.
4. Compute size from the inverted mask.

BARs are decoded into `PciBar::Memory { base, size, prefetchable, is_64bit }`, `PciBar::Io { base, size }`, or `PciBar::Unused`. 64-bit memory BARs consume two BAR slots.

### Capability Parsing (`pci/caps.rs`)

`walk_capabilities()` returns a `CapabilityIter` that walks the PCI capability linked list starting from the Capabilities Pointer register (offset 0x34). Each iteration yields a `RawCapability` with the capability `id` and config-space `offset`.

Specialized parsers exist for:
- **VirtIO PCI capabilities** (`read_virtio_pci_cap`) -- Parses `VirtioPciCap` with fields for `cfg_type` (CommonCfg, NotifyCfg, IsrCfg, DeviceCfg, PciCfg), BAR index, offset, and length.
- **MSI-X capabilities** (`read_msix_cap`) -- Parses `MsixCapability` with table size, table BAR/offset, and PBA BAR/offset.

## Device Registry

Defined in `drivers/device_registry.rs`, the `DeviceRegistry` is the kernel's central hub for decoupling driver discovery from device consumption.

### Storage

The registry stores devices in `BTreeMap`s keyed by the leaf segment of the `DevicePath`:
- **Framebuffers**: `BTreeMap<String, Arc<dyn Framebuffer>>` -- shared ownership via `Arc`, multiple consumers can hold references
- **Block devices**: `BTreeMap<String, Box<dyn DynBlockDevice>>` -- take-once ownership, removed from the registry on retrieval via `take_block_device()`

### Driver Tracking

Each probed driver is tracked by a `DriverEntry` containing its name, current `DriverState`, optional `Arc<dyn ManagedDriver>` lifecycle handle, and the list of `DevicePath`s it registered.

### Registration Flow

After a successful probe, the kernel calls `register_driver(name, devices, lifecycle)`, which:
1. Iterates the `DeviceSet`'s framebuffers and block devices
2. Inserts each into the appropriate `BTreeMap`, keyed by the `DevicePath` leaf
3. Records a `DriverEntry` with state `Active`

### Lifecycle Management

`shutdown_all()` iterates all driver entries in reverse registration order. For each active or suspended driver with a lifecycle handle, it calls `ManagedDriver::shutdown()` and transitions the state to `Shutdown`.

### Global Access

The registry is stored as a `SpinLock<Option<DeviceRegistry>>` global static, accessed via `with_device_registry()` (shared) and `with_device_registry_mut()` (exclusive). Both panic with descriptive messages if the registry has not been initialized.

## Device Paths

Defined in `driver_api/device_path.rs`, `DevicePath` provides hierarchical device naming that encodes a device's position in the hardware topology:

- PCI devices: `pci/0000:00:1f.2/ahci/ahci-0`
- Platform devices: `platform/uart16550`

The `leaf()` method returns the last segment (e.g., `"ahci-0"`), which is used as the key in the device registry for backward compatibility.

## Dynamic Dispatch for Async Traits

Because `BlockDevice` uses `async fn` methods (which are not dyn-compatible), `driver_api/dyn_dispatch.rs` provides a `DynBlockDevice` trait that wraps the returned futures in `Pin<Box<dyn Future>>`. The `DynBlockDeviceWrapper<D>` adapter converts any concrete `BlockDevice` into a `Box<dyn DynBlockDevice>`. A blanket `BlockDevice` impl on `Box<dyn DynBlockDevice>` closes the round-trip, allowing type-erased block devices to be passed to functions expecting `impl BlockDevice`.

## Error Handling

`DriverError` (`driver_api/error.rs`) is a typed enum with six variants: `DeviceNotFound`, `InitFailed`, `Timeout`, `Unsupported`, `IoError`, and `InvalidState`. Block I/O uses a separate `IoError` enum with variants: `OutOfRange`, `DeviceError`, `InvalidBuffer`, `Timeout`, `DmaError`, and `NotReady`.

## Key Types Reference

| Type | Location | Purpose |
|------|----------|---------|
| `Driver` | `driver_api/driver.rs` | Base trait for identity/metadata |
| `DriverInfo` | `driver_api/driver.rs` | Static metadata (name, type, description) |
| `DriverState` | `driver_api/driver.rs` | Lifecycle state enum |
| `PlatformDriver` | `driver_api/category.rs` | Category trait with async `probe()` |
| `ManagedDriver` | `driver_api/lifecycle.rs` | Lifecycle hooks (suspend/resume/shutdown) |
| `PciProbeContext` | `driver_api/probe_context.rs` | PCI probe bundle with capabilities |
| `PlatformProbeContext` | `driver_api/probe_context.rs` | Platform probe bundle with capabilities |
| `PciDriverEntry` | `driver_api/registration.rs` | Linker-section entry for PCI drivers |
| `PlatformDriverEntry` | `driver_api/registration.rs` | Linker-section entry for platform drivers |
| `PciDriverRegistration` | `driver_api/registration.rs` | Probe result: devices + lifecycle handle |
| `DeviceSet` | `driver_api/registration.rs` | Collection of devices from a probe |
| `DevicePath` | `driver_api/device_path.rs` | Hierarchical device naming |
| `DeviceRegistry` | `drivers/device_registry.rs` | Central device storage and tracking |
| `IoPortRange` | `driver_api/resource.rs` | Exclusive I/O port claim |
| `MmioRegion` | `driver_api/resource.rs` | Exclusive MMIO region claim |
| `IrqLine` | `driver_api/resource.rs` | Exclusive IRQ claim |
| `PciDeviceId` | `driver_api/pci.rs` | Device ID for matching |
| `PciDeviceInfo` | `driver_api/pci.rs` | Full enumerated device info |
| `PciBar` | `driver_api/pci.rs` | Decoded Base Address Register |
| `PciCam` | `pci/cam.rs` | Legacy config space access |
| `CapabilityIter` | `pci/caps.rs` | PCI capability linked-list walker |
| `BlockDevice` | `driver_api/block.rs` | Async sector I/O interface |
| `SerialPort` | `driver_api/serial.rs` | Async byte I/O interface |
| `Framebuffer` | `driver_api/framebuffer.rs` | Display output interface |
| `DynBlockDevice` | `driver_api/dyn_dispatch.rs` | Dyn-compatible block device wrapper |
| `DriverError` | `driver_api/error.rs` | Driver operation errors |
| `CapabilityFlags` | `driver_api/capability.rs` | Runtime capability bitmap |
