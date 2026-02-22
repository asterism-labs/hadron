# Hadron

Hadron is an x86_64 kernel written in Rust. It uses an async-native, cooperative execution model built around a two-crate architecture: a monolithic **hadron-kernel** providing arch primitives, memory management, scheduling, VFS, and PCI core, paired with a pluggable **hadron-drivers** crate where hardware drivers are registered via linker sections. Both run in ring 0. The project follows a particle physics naming theme — **Gluon** for the build system, **Lepton** for userspace.

## Features

### Boot & Architecture

| Feature | Description |
|---------|-------------|
| Limine boot protocol | Boots via the Limine protocol with a dedicated boot stub |
| x86_64 support | Full x86_64 with GDT, IDT, TSS, and MSR management |
| ACPI parsing | In-house ACPI table parser (RSDP, MADT, HPET, FADT, MCFG) |
| Custom target spec | Bare-metal `x86_64-unknown-hadron` target with red-zone and SSE disabled |

### Memory Management

| Feature | Description |
|---------|-------------|
| Physical memory manager | Bitmap-based PMM with region tracking |
| Virtual memory manager | 4-level paging with per-process address spaces |
| Kernel heap | Slab allocator with `GlobalAlloc` integration |
| HHDM | Higher-half direct mapping for physical memory access |

### Scheduling & Async

| Feature | Description |
|---------|-------------|
| Async cooperative executor | Core executor driving all kernel subsystems |
| Per-CPU executors | Each CPU runs its own executor instance |
| Budget preemption | Tasks yield after exhausting their time budget |
| Timer queue | Async timers and sleeps backed by HPET/TSC |
| Work stealing | IPI-based cross-CPU task migration |

### Virtual Filesystem

| Feature | Description |
|---------|-------------|
| Async VFS traits | Async inode, directory, and filesystem interfaces |
| ramfs | In-memory filesystem for tmpfs-style mounts |
| devfs | Device file interface (`/dev/*`) |
| ext2 | Read/write ext2 filesystem driver |
| FAT | FAT12/16/32 filesystem support |
| ISO 9660 | CD-ROM filesystem for bootable media |
| initramfs | CPIO-based initial ramdisk, unpacked at boot |
| Mount table | Multi-filesystem mount management |

### Networking

| Feature | Description |
|---------|-------------|
| TCP/UDP/ICMP/ARP | Full network stack via smoltcp |
| VirtIO-net | Virtio network device driver |
| Socket syscalls | POSIX-style socket API for userspace |

### Drivers & Hardware

| Feature | Description |
|---------|-------------|
| AHCI | SATA controller for disk I/O |
| VirtIO-blk | Virtio block device driver |
| UART 16550 | Serial console for debug output |
| PS/2 keyboard & mouse | Legacy input device support |
| Bochs VGA | Display driver with framebuffer console |
| HPET / PIT / TSC | Timer sources with calibration |
| LAPIC / IOAPIC | Interrupt routing and per-CPU local APICs |

### Process Management & Syscalls

| Feature | Description |
|---------|-------------|
| ELF64 loader | Loads and maps userspace ELF binaries |
| Ring 3 execution | Full user-mode process support |
| ~50 syscalls | File, process, memory, socket, and signal syscalls |
| vDSO | Virtual dynamic shared object for fast clock reads |
| Futex | Fast userspace locking primitive |

### IPC & Signals

| Feature | Description |
|---------|-------------|
| Pipes | Byte-stream IPC between processes |
| Async channels | Kernel-internal mpsc and oneshot channels |
| POSIX signals | Signal delivery and handler registration |

### SMP

| Feature | Description |
|---------|-------------|
| AP bootstrap | Multi-core startup via ACPI MADT |
| Per-CPU GDT/TSS | Isolated descriptor tables per core |
| IPI work stealing | Cross-CPU task migration via inter-processor interrupts |
| GS-base per-CPU data | Per-CPU storage using `GS` segment base |

### Userspace

| Feature | Description |
|---------|-------------|
| lepton-syslib | Userspace syscall wrapper library |
| lepton-init | Init process (PID 1) |
| lsh | Interactive shell with pipes, redirection, and job control |
| lepton-coreutils | 11 core utilities (ls, cat, echo, mkdir, etc.) |

### Developer Tools & Testing

| Feature | Description |
|---------|-------------|
| Gluon build system | Custom Rhai-scripted build system invoking rustc directly |
| hadron-test | Kernel-mode test harness with serial output |
| hadron-bench | Micro-benchmark framework |
| HKIF backtraces | Symbolic kernel backtraces via DWARF |
| Lockdep | Lock-order dependency checking |
| Profiling | Built-in kernel profiling support |

## Project Structure

```
hadron/
├── kernel/
│   ├── hadron-kernel/       # Monolithic kernel core
│   ├── hadron-drivers/      # Pluggable hardware drivers
│   └── boot/limine/         # Limine boot stub
├── crates/
│   ├── parse/               # ACPI, ELF, DWARF, FDT, binary parsers
│   ├── boot/                # Limine & UEFI protocol bindings
│   ├── core/                # Core abstractions & linkset
│   ├── driver/              # Driver framework & MMIO macros
│   ├── syscall/             # Syscall definitions & macros
│   ├── test/                # Test & benchmark harnesses
│   └── tools/               # Host-side codegen & profiling
├── userspace/               # Init, shell, syslib, coreutils
├── tools/gluon/             # Build system
├── targets/                 # Custom target specs
└── docs/                    # mdbook documentation
```

## Getting Started

Requires a nightly Rust toolchain, QEMU, and NASM. See `rust-toolchain.toml` for the pinned nightly version.

```sh
just vendor      # Fetch vendored dependencies
just configure   # Generate rust-project.json and resolve config
just build       # Build the kernel, drivers, userspace, and initrd
just run         # Launch in QEMU
```

## License

GPL-3.0-only
