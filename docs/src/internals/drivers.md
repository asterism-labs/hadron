# Hardware Drivers

The `hadron-drivers` crate contains all hardware driver implementations for the
Hadron kernel. It is separate from `hadron-kernel` by design: the kernel defines
driver API traits and registration infrastructure, while `hadron-drivers`
provides the concrete implementations. Both crates run in ring 0 and are linked
into the same kernel binary.

## Organization

Drivers are organized by subsystem under `kernel/hadron-drivers/src/`:

| Module        | Purpose                                        |
|---------------|------------------------------------------------|
| `ahci/`       | AHCI SATA disk controller                      |
| `virtio/`     | VirtIO PCI modern transport and block device   |
| `serial/`     | UART 16550 serial port (sync and async)        |
| `input/`      | i8042 PS/2 controller, keyboard, mouse         |
| `display/`    | Bochs VGA framebuffer                          |
| `block/`      | In-memory block devices (ramdisk)              |
| `fs/`         | Filesystem implementations (FAT, ISO 9660, ramfs, initramfs) |
| `pci/`        | PCI bus enumeration, CAM access, capabilities, MSI-X |
| `interrupt/`  | APIC (Local + I/O) and legacy 8259 PIC         |
| `timer/`      | HPET, PIT, TSC clock sources                   |

The crate root (`lib.rs`) re-exports key types and defines the
`__HADRON_DRIVERS_ANCHOR` symbol, which the linker script references via
`EXTERN()` to force inclusion of all driver registration entries.

## Driver Registration Model

Hadron uses a **linker-section-based** registration model. Drivers never call a
runtime `register()` function. Instead, the `#[hadron_driver]` proc macro emits
a static struct into a dedicated linker section, and the kernel reads those
sections at boot to discover available drivers.

### PCI Drivers

PCI drivers use the `#[hadron_driver]` attribute macro on an impl block:

```rust
#[hadron_driver_macros::hadron_driver(
    name = "ahci",
    kind = pci,
    capabilities = [Irq, Mmio, Dma, PciConfig],
    pci_ids = &ID_TABLE,
)]
impl AhciDriver {
    fn probe(ctx: DriverContext) -> Result<PciDriverRegistration, DriverError> {
        // ...
    }
}
```

The macro places a `PciDriverEntry` into the `.hadron_pci_drivers` linker
section. Each entry contains a name, a PCI device ID table, a capability
descriptor, and a `probe` function pointer. At boot, `registry::match_pci_drivers()`
iterates the section, matches entries against enumerated PCI devices by
vendor/device ID or class/subclass/progif, and calls `probe` on matches. The
probe function receives a `DriverContext` providing access to requested
capabilities (MMIO mapping, IRQ binding, DMA allocation, PCI config space).

### Platform Drivers

Platform drivers target fixed-address legacy hardware (serial ports, PS/2
controller, HPET). They use the same `#[hadron_driver]` macro with
`kind = platform` and a `compatible` string instead of PCI IDs:

```rust
#[hadron_driver_macros::hadron_driver(
    name = "uart16550",
    kind = platform,
    capabilities = [Irq, Spawner],
    compatible = "ns16550",
)]
```

Platform entries go into the `.hadron_platform_drivers` section. The kernel
maintains a hardcoded table of known platform devices in `bus.rs`:

```rust
const PLATFORM_DEVICES: &[(&str, &str)] = &[
    ("com1", "ns16550"),
    ("com2", "ns16550"),
    ("i8042", "i8042"),
    ("hpet0", "hpet"),
];
```

`registry::match_platform_drivers()` matches these against registered entries
by compatible string and calls `init`.

### Filesystem Drivers

Filesystem drivers use dedicated macros that emit entries into their own linker
sections:

- `block_fs_entry!` -- places a `BlockFsEntry` into `.hadron_block_fs` (for
  disk-backed filesystems like FAT and ISO 9660)
- `virtual_fs_entry!` -- places a `VirtualFsEntry` into `.hadron_virtual_fs`
  (for virtual filesystems like ramfs)
- `initramfs_entry!` -- places an `InitramFsEntry` into `.hadron_initramfs`
  (for initramfs unpackers like the CPIO handler)

### Device Tree

During boot, `bus::DeviceTree` builds a hierarchical tree from enumerated PCI
devices and hardcoded platform devices. The tree is printed to the kernel log
using box-drawing characters and used for driver matching. Key types:

- `DeviceTree` -- the full tree, with PCI bus nodes, platform bus, and USB bus
  placeholder
- `DeviceNode` -- a node with a name, `DeviceInfo` variant, optional driver
  name, and children
- `DeviceInfo` -- enum: `Root`, `PciBus`, `PciDevice`, `PlatformDevice`,
  `PlatformBus`, `UsbBus`

### IRQ-to-Async Bridge

The `irq` module provides `IrqLine`, which bridges hardware interrupts into the
async executor. Each `IrqLine` binds an interrupt vector to a `WaitQueue`
(array of 224 entries covering vectors 32--255). When a hardware interrupt fires,
`irq_wakeup_handler` calls `wake_all()` on the corresponding wait queue, waking
any async task that called `irq.wait().await`.

```rust
let irq = IrqLine::bind_isa(4, irq_cap)?;  // Bind ISA IRQ 4
irq.wait().await;                            // Sleep until interrupt fires
```

## Storage Drivers

### AHCI (SATA)

**Source:** `ahci/mod.rs`, `ahci/hba.rs`, `ahci/port.rs`, `ahci/command.rs`, `ahci/regs.rs`

The AHCI driver supports Intel ICH9 controllers (vendor `0x8086`, device
`0x2922`) and any AHCI-compliant controller (class `0x01`, subclass `0x06`,
prog-if `0x01`).

**Key types:**

- `AhciDisk` -- implements the `BlockDevice` trait; wraps an `AhciPort` with an
  `IrqLine` and `DmaCapability`
- `AhciHba` -- manages the HBA's MMIO register space (ABAR), enables AHCI mode
  and global interrupts, queries capabilities (command slot count, 64-bit support)
- `AhciPort` -- per-port state: command list base, FIS buffer, per-slot command
  tables, atomic slot allocation bitmask, and parsed `DeviceIdentity`
- `DeviceIdentity` -- parsed ATA IDENTIFY DEVICE data (sector count, sector
  size, model string, serial number)
- `FisRegH2d`, `CommandHeader`, `PrdtEntry` -- packed `#[repr(C)]` structs
  matching the AHCI hardware command format

**Initialization flow:**

1. The PCI probe reads BAR5 (ABAR) and maps it via MMIO
2. Bus mastering is enabled in PCI config space
3. The HBA is created from the ABAR base; AHCI mode and global interrupts are
   enabled
4. An `IrqLine` is bound to the device's ISA IRQ and unmasked
5. Each implemented port is checked for device presence via SStatus
6. For present ports: command list and FIS buffers are allocated via DMA, command
   tables are set up per slot, SERR is cleared, interrupts are enabled, and
   IDENTIFY DEVICE runs to discover disk geometry
7. Each identified disk is wrapped in `AhciDisk` and registered via `DeviceSet`

**I/O model:** Sector reads use DMA bounce buffers. The `read_sector` method
allocates a DMA page, programs a READ DMA EXT command via the FIS and PRDT, and
awaits completion through `issue_command_async`, which loops on `irq.wait().await`
until the port's interrupt status indicates completion. Write support is
stubbed for a future phase.

### Ramdisk

**Source:** `block/ramdisk.rs`

`RamDisk` is a heap-backed `BlockDevice` used for testing. It allocates a
`Vec<u8>` of `sector_count * sector_size` bytes. Reads are simple `copy_from_slice`
operations. Write through the `BlockDevice` trait is not supported (requires
interior mutability); direct mutation is available via `as_bytes_mut()`.

## VirtIO Drivers

**Source:** `virtio/mod.rs`, `virtio/pci.rs`, `virtio/queue.rs`, `virtio/block.rs`

### PCI Modern Transport

`VirtioPciTransport` discovers VirtIO configuration structures by walking PCI
vendor-specific capabilities. It maps BARs for the four required regions:

- **Common config** -- device status, features, queue configuration
- **Notify** -- queue doorbell writes (with per-queue offset multiplier)
- **ISR** -- interrupt status (clears on read)
- **Device config** -- device-specific parameters (capacity, sector size)

Optional MSI-X support is detected from the PCI capability list.

### Split Virtqueues

`Virtqueue` implements the VirtIO split-queue model with three DMA-allocated
regions:

- **Descriptor table** -- array of `VirtqDesc` entries (address, length, flags,
  next pointer)
- **Available ring** -- guest-to-device: descriptor head indices for pending
  requests
- **Used ring** -- device-to-guest: completed descriptor IDs and byte counts

The free list threads through descriptor `next` fields. `add_buf()` chains
descriptors and publishes them to the available ring. `poll_used()` checks the
used ring for completions and frees consumed descriptor chains.

### VirtIO Block Device

`VirtioBlkDisk` implements `BlockDevice` for VirtIO block devices (vendor
`0x1AF4`, device `0x1042` modern or `0x1001` transitional).

**Initialization follows the VirtIO 1.0 spec steps 1-7:**

1. Reset the device (status = 0)
2. Set ACKNOWLEDGE
3. Set DRIVER
4. Negotiate features (require `VIRTIO_F_VERSION_1`)
5. Set FEATURES_OK, verify readback
6. Set up the request queue (queue 0) via `VirtioDevice::setup_queue()`
7. Set DRIVER_OK

IRQ delivery prefers MSI-X when available, falling back to legacy INTx.
MSI-X setup allocates a vector, binds an `IrqLine`, and configures MSI-X table
entry 0.

**I/O model:** Each block request uses a 3-descriptor chain in a single DMA
page:

| Offset     | Content              | Direction       |
|------------|----------------------|-----------------|
| 0..16      | `VirtioBlkReqHeader` | Device-readable |
| 16..16+ss  | Data buffer          | Device-writable (read) or device-readable (write) |
| 16+ss      | Status byte          | Device-writable |

After submitting the chain and notifying the device, the driver loops on
`irq.wait().await`, acknowledges the ISR, and polls the used ring for
completion. Both read and write are fully implemented.

## Serial Drivers

### UART 16550

**Source:** `serial/uart16550.rs`

`Uart16550` is a stateless `Copy` type identified by a base I/O port address.
Standard port constants are defined: `COM1` (`0x3F8`), `COM2` (`0x2F8`),
`COM3` (`0x3E8`), `COM4` (`0x2E8`).

**Initialization** programs the UART via I/O ports:

1. Disable interrupts
2. Set DLAB, write baud rate divisor (supports 9600 to 115200 baud)
3. Configure 8N1 line settings
4. Enable and clear FIFOs with 14-byte trigger level
5. Set DTR, RTS, OUT2 in the modem control register
6. Run a loopback self-test (send `0xAE`, verify echo)
7. Restore normal operation

The type implements `core::fmt::Write` for formatted output (with `\n` to
`\r\n` translation). Blocking `read_byte()` and `write_byte()` spin on the
Line Status Register.

Register access uses `bitflags` types: `Ier`, `Fcr`, `Lcr`, `Mcr`, `Lsr` for
type-safe register manipulation.

### Async Serial

**Source:** `serial/serial_async.rs`

`AsyncSerial` wraps a `Uart16550` with an `IrqLine` to implement the
`SerialPort` trait. It binds the ISA IRQ (typically IRQ 4 for COM1), unmasks the
I/O APIC entry, and enables the UART RX interrupt.

`read_byte()` first checks if data is already in the FIFO, then falls back to
`irq.wait().await` to sleep until the next RX interrupt. TX remains synchronous
since it completes effectively instantly at typical baud rates.

The platform driver registration spawns a "serial-echo" async task that reads
bytes from the serial port and echoes them back, providing an interactive serial
console for debugging.

## Input Drivers

### i8042 PS/2 Controller

**Source:** `input/i8042.rs`

`I8042` drives the Intel 8042 PS/2 controller via I/O ports `0x60` (data) and
`0x64` (status/command). Like `Uart16550`, it is a stateless `Copy` type.

**Initialization sequence:**

1. Disable both ports (keyboard and mouse)
2. Flush the output buffer
3. Read and modify the config byte (disable IRQs during setup)
4. Run controller self-test (expect `0x55`)
5. Enable both ports
6. Re-enable IRQ bits (IRQ 1 for keyboard, IRQ 12 for mouse)
7. Reset the keyboard device (send `0xFF`, expect ACK + self-test pass)

**Scancode translation** maps Set 1 scancodes to `KeyCode` values. The
`scancode_to_keycode()` function handles standard make/break codes (bit 7 =
release). Extended scancodes (prefixed by `0xE0`) are translated by
`extended_scancode_to_keycode()` covering arrow keys, Home, End, Page Up/Down,
Insert, Delete, and right-side modifiers.

**Mouse packet parsing** is handled by `MousePacket::parse()`, which decodes
a 3-byte PS/2 packet into relative X/Y movement and button state (left, right,
middle). Sign extension uses bits 4-5 of the status byte.

### Async Keyboard

**Source:** `input/keyboard_async.rs`

`AsyncKeyboard` implements the `KeyboardDevice` trait by combining an `I8042`
with an `IrqLine` bound to ISA IRQ 1. It tracks extended scancode state via an
`AtomicBool`. The `read_event()` method attempts to decode a `KeyEvent` (key +
pressed/released) from the scancode stream, sleeping on the IRQ when no data is
available.

### Async Mouse

**Source:** `input/mouse_async.rs`

`AsyncMouse` implements the `MouseDevice` trait, binding to ISA IRQ 12. The
`read_event()` method collects a full 3-byte packet by awaiting
`read_mouse_byte()` three times (each waiting on the IRQ if needed), then parses
it into a `MouseEvent` with dx, dy, and button fields.

## Display Drivers

### Bochs VGA

**Source:** `display/bochs_vga.rs`

`BochsVga` drives the Bochs/QEMU VGA adapter (vendor `0x1234`, device `0x1111`)
using the VBE DISPI interface for mode setting and BAR0 for the linear
framebuffer.

**Initialization:**

1. Validate the BGA version (minimum `0xB0C0`)
2. Map BAR0 (framebuffer memory) via MMIO
3. Program DISPI registers: disable display, set resolution and BPP, enable
   with linear framebuffer mode
4. Zero the framebuffer to clear stale pixel data

The default mode is 1024x768 at 32bpp with `Bgr32` pixel format. The
`Framebuffer` trait implementation provides:

- `put_pixel(x, y, color)` -- bounds-checked volatile write to the framebuffer
- `fill_rect(x, y, w, h, color)` -- row-by-row volatile fill
- `copy_within(src, dst, count)` -- overlapping memory copy for scrolling
- `fill_zero(offset, count)` -- fast zeroing (for clearing scroll regions)

The VBE DISPI interface uses a pair of I/O ports (`0x01CE` index, `0x01CF`
data) wrapped in `DispiPorts` for register-indexed access.

## Filesystem Implementations

### FAT (FAT12/16/32)

**Source:** `fs/fat.rs`

`FatFileSystem` mounts FAT volumes using the `hadris-fat` crate. It auto-detects
the FAT variant from the boot sector. The filesystem wraps `FatFs` in an
`Arc<SharedFatFs>` for shared access (with an explicit `Sync` impl, since
FSInfo cache cells are only modified during serialized write operations).

Directory navigation uses `FatDirInode` (root vs. subdirectory variants) and
file reads use `FatFileInode`. File reads skip to the requested offset by
reading and discarding bytes in 512-byte chunks (seek is not yet supported in
the underlying crate). Write and unlink operations return `FsError::NotSupported`.

Registered via `block_fs_entry!` into the `.hadron_block_fs` linker section.

### ISO 9660

**Source:** `fs/iso9660.rs`

`Iso9660Fs` mounts ISO 9660 images (read-only) using the `hadris-iso` crate.
Directory inodes (`Iso9660DirInode`) store a `DirectoryRef` pointing to the
directory's on-disk location. File inodes (`Iso9660FileInode`) store the extent
LBA and file size; reads compute the byte offset as
`extent_lba * 2048 + offset` and delegate to `IsoImage::read_bytes_at()`.

Registered via `block_fs_entry!` into the `.hadron_block_fs` linker section.

### RamFS

**Source:** `fs/ramfs.rs`

`RamFs` is a heap-backed in-memory filesystem used as the root filesystem. All
data lives in `SpinLock`-protected structures: `Vec<u8>` for file data and
`BTreeMap<String, Arc<RamInode>>` for directory children.

`RamInode` supports files, directories, and symlinks. It implements the full
`Inode` trait including `create`, `unlink`, `read`, `write`, `read_link`, and
`create_symlink`. All futures resolve in a single poll (synchronous completion).

Registered via `virtual_fs_entry!` into the `.hadron_virtual_fs` linker section.

### Initramfs (CPIO)

**Source:** `fs/initramfs.rs`

`unpack_cpio()` extracts a CPIO newc archive into the VFS. It iterates entries
using `hadris_cpio::CpioReader` and:

- Creates directories recursively via `ensure_directory()`
- Creates files and writes their contents into the root filesystem
- Creates symlinks via `create_symlink()`
- Skips unsupported entry types (devices, etc.)

Registered via `initramfs_entry!` into the `.hadron_initramfs` linker section.

## Linker Section Summary

All driver registration relies on dedicated ELF sections that the kernel reads
at boot. No runtime registration calls are needed.

| Section                    | Entry Type            | Used By                  |
|----------------------------|-----------------------|--------------------------|
| `.hadron_pci_drivers`      | `PciDriverEntry`      | AHCI, VirtIO block, Bochs VGA |
| `.hadron_platform_drivers` | `PlatformDriverEntry` | UART 16550, i8042, HPET  |
| `.hadron_block_fs`         | `BlockFsEntry`        | FAT, ISO 9660            |
| `.hadron_virtual_fs`       | `VirtualFsEntry`      | ramfs                    |
| `.hadron_initramfs`        | `InitramFsEntry`      | CPIO unpacker            |

The linker script defines `__hadron_<section>_start` and `__hadron_<section>_end`
symbols for each section. At boot, `registry.rs` computes the entry count from
pointer arithmetic and constructs a slice over the section contents:

```rust
let start = addr_of!(__hadron_pci_drivers_start).cast::<PciDriverEntry>();
let end = addr_of!(__hadron_pci_drivers_end).cast::<PciDriverEntry>();
let count = end.offset_from(start) as usize;
core::slice::from_raw_parts(start, count)
```

This approach eliminates boot-time allocation for driver discovery and allows
new drivers to be added simply by linking them into the kernel image.
