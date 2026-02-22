# hadron-drivers

Pluggable hardware drivers for the Hadron kernel. This crate contains all concrete device driver implementations that run in ring 0 alongside the kernel. Drivers register themselves at link time via linker-section entries (`#[hadron_driver]` macro) and are matched against discovered PCI and ACPI platform devices during boot. The crate depends on the driver API traits defined in `hadron-kernel` and the MMIO register abstraction from `hadron-mmio`.

## Features

- **AHCI SATA driver** -- drives Intel ICH9 and any AHCI-compatible controller (class 01:06:01); enumerates ports, identifies attached SATA disks, and provides async block I/O with DMA bounce buffers and IRQ-driven completion
- **VirtIO block driver** -- VirtIO 1.0 PCI modern transport with split virtqueue management, feature negotiation (`VIRTIO_F_VERSION_1`), MSI-X interrupt support, and async sector read/write
- **UART 16550 serial** -- COM1-COM4 serial port driver with configurable baud rate, FIFO control, and interrupt-driven async receive; used as the early boot console before the heap is available
- **Bochs VGA display** -- PCI framebuffer driver for the Bochs/QEMU VBE adapter; configures resolution via VBE dispi registers and provides a `Framebuffer` device for the kernel's fbcon console
- **i8042 keyboard and mouse** -- PS/2 controller driver with scancode-to-keycode translation, async keyboard input via IRQ-driven ring buffer, and PS/2 mouse protocol support with async event delivery
- **APIC interrupt controller** -- Local APIC and I/O APIC drivers for x86_64 interrupt routing, including ISA IRQ remapping via MADT override entries and EOI signaling
- **Legacy 8259 PIC** -- PIC initialization and masking for the boot-time transition before APIC takeover
- **HPET timer** -- High Precision Event Timer driver providing monotonic nanosecond timestamps and periodic tick generation for the scheduler
- **PIT timer** -- Intel 8254 Programmable Interval Timer for HPET-less systems and LAPIC calibration
- **TSC clock source** -- Time Stamp Counter calibration (against HPET) and nanosecond conversion for high-resolution timing
- **RAM filesystem (ramfs)** -- in-memory filesystem implementing the `Inode` and `FileSystem` traits with directories, regular files, and symlinks; serves as the root filesystem
- **CPIO initramfs unpacker** -- extracts CPIO newc archives into the mounted root filesystem at boot
- **FAT filesystem** -- read/write FAT12/16/32 filesystem driver backed by block devices, with long filename (LFN) support via `hadris-fat`
- **ISO 9660 filesystem** -- read-only ISO 9660 / CD-ROM filesystem driver backed by block devices via `hadris-iso`
- **Ramdisk block device** -- wraps an in-memory byte buffer as a `BlockDevice` for testing and initrd-backed filesystems
- **Linker-section registration** -- all drivers use the `#[hadron_driver]` proc macro to emit `PciDriverEntry` or `PlatformDriverEntry` structs into dedicated ELF sections, discovered by the kernel's registry scanner at boot
- **Anchor symbol** -- `__HADRON_DRIVERS_ANCHOR` ensures the linker includes this crate's sections even when no direct symbol references exist

## Architecture

The crate is organized by hardware subsystem, each in its own module:

- **`ahci/`** -- AHCI HBA register definitions (`regs`), HBA controller abstraction (`hba`), per-port state machine (`port`), command table and FIS construction (`command`), and the `AhciDisk` block device wrapper with PCI driver registration.
- **`virtio/`** -- VirtIO PCI modern transport (`pci`), split virtqueue descriptor/available/used ring management (`queue`), and the virtio-blk device driver (`block`) with PCI registration.
- **`serial/`** -- UART 16550 register-level driver (`uart16550`) and async serial receive future (`serial_async`).
- **`display/`** -- Bochs VGA PCI framebuffer driver (`bochs_vga`) with VBE dispi register programming.
- **`input/`** -- i8042 PS/2 controller driver (`i8042`), async keyboard input with scancode translation (`keyboard_async`), and async mouse event stream (`mouse_async`).
- **`interrupt/`** -- APIC subsystem (`apic/local_apic`, `apic/io_apic`) and legacy 8259 PIC driver (`pic`).
- **`timer/`** -- HPET (`hpet`), PIT (`pit`), and TSC (`tsc`) timer drivers.
- **`block/`** -- ramdisk block device (`ramdisk`).
- **`fs/`** -- ramfs (`ramfs`), CPIO initramfs unpacker (`initramfs`), FAT driver (`fat`), and ISO 9660 driver (`iso9660`), all registered via linker-section entries.
- **`pci/`** -- PCI capability parsing (`caps`), MSI-X setup (`msix`), CAM config access (`cam`), bus enumeration (`enumerate`), and stub driver for unmatched devices (`stub`).
- **`registry.rs`** -- reads PCI and platform driver entries from linker sections and matches them against discovered devices.
- **`irq.rs`** -- IRQ line binding and ISA-to-vector translation helpers.
- **`bus.rs`** -- bus-level driver probe orchestration.
