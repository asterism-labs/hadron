# sysfs Virtual Filesystem

## Goal

Implement a read-only virtual filesystem at `/sys` that exposes hardware topology
and device metadata. Mesa and other userspace tools read sysfs to discover PCI
devices and DRM render nodes. This replaces the Linux sysfs subset that the
graphics stack depends on.

## Background

Linux exposes hardware information through two virtual filesystems: procfs
(`/proc`) for process state and sysfs (`/sys`) for device topology. Hadron
replaces procfs with structured `sys_query` syscalls (see
[Graphics Stack Design](../design/graphics-stack.md)) but implements sysfs
because the hierarchical file metaphor fits hardware device trees well.

Mesa reads the following sysfs paths for GPU device discovery:

```
/sys/bus/pci/devices/0000:00:02.0/vendor    → "0x1af4"
/sys/bus/pci/devices/0000:00:02.0/device    → "0x1050"
/sys/bus/pci/devices/0000:00:02.0/class     → "0x030000"
/sys/bus/pci/devices/0000:00:02.0/resource   → BAR addresses and sizes
/sys/class/drm/renderD128/device/           → symlink to PCI device
/sys/dev/char/226:128                       → symlink to DRM device
```

## Key Design

### Virtual Inode Tree

sysfs is a read-only virtual filesystem backed by kernel data structures — no
on-disk storage. Each file returns dynamically generated content from the kernel's
PCI enumeration data and device registry.

```rust
/// A sysfs directory or file backed by a kernel data source.
pub enum SysfsNode {
    /// Directory with child entries.
    Dir(BTreeMap<&'static str, Arc<dyn Inode>>),
    /// File that returns a formatted value on read.
    Attr(Box<dyn Fn() -> Vec<u8> + Send + Sync>),
    /// Symbolic link to another path.
    Symlink(String),
}
```

### Filesystem Layout

```
/sys/
├── bus/
│   └── pci/
│       └── devices/
│           └── 0000:BB:DD.F/     ← one per PCI device
│               ├── vendor         ← "0x1af4\n"
│               ├── device         ← "0x1050\n"
│               ├── class          ← "0x030000\n"
│               ├── subsystem_vendor
│               ├── subsystem_device
│               ├── resource       ← BAR info (one line per BAR)
│               ├── irq            ← IRQ number
│               └── enable         ← "1\n" or "0\n"
├── class/
│   └── drm/
│       ├── card0/
│       │   └── device -> ../../bus/pci/devices/0000:00:02.0
│       └── renderD128/
│           └── device -> ../../bus/pci/devices/0000:00:02.0
└── dev/
    └── char/
        └── 226:128 -> ../../class/drm/renderD128
```

### Data Sources

sysfs reads from existing kernel data structures — no new data collection is
needed:

| sysfs path | Kernel source |
|------------|---------------|
| `/sys/bus/pci/devices/` | PCI enumeration tree (`kernel/pci/src/enumerate.rs`) |
| PCI device attributes | `PciDevice` struct fields (vendor, device, class, BARs) |
| `/sys/class/drm/` | Device registry DRM entries (populated by GPU drivers) |
| `/sys/dev/char/` | devfs major:minor mapping |

### Mount and Registration

sysfs is mounted at `/sys` during VFS initialization, alongside the existing
`/dev` (devfs) and `/` (ramfs) mounts. GPU drivers register their DRM class
entries via a sysfs registration API:

```rust
/// Register a DRM device in sysfs, creating /sys/class/drm/{name}/
/// with a device symlink pointing to the PCI device directory.
pub fn sysfs_register_drm(name: &str, pci_addr: PciAddress);
```

### Read Semantics

All sysfs files are read-only. Reads return the full attribute value — sysfs
files are small (typically under 32 bytes). Writes return `EACCES`. The `stat`
call returns appropriate sizes and modes.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `kernel/fs/src/sysfs.rs` | **New:** sysfs filesystem implementation |
| `kernel/fs/src/lib.rs` | Register sysfs mount at `/sys` |
| `kernel/kernel/src/init.rs` | Populate sysfs PCI tree after enumeration |
| `kernel/driver-api/src/lib.rs` | Add `sysfs_register_drm` API for drivers |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| sysfs inode tree | Service | Pure Rust data structures, no hardware access |
| PCI attribute formatting | Service | String formatting from existing structs |
| VFS mount integration | Service | Uses existing mount table API |

## Dependencies

- **Async VFS & Ramfs**: VFS mount table, `Inode` trait (complete).
- **Device Drivers**: PCI enumeration data, device registry (complete).
- **Dynamic devfs**: DRM class registration happens after driver probe (new).

## Milestone

```
sysfs: mounted at /sys
sysfs: populated 8 PCI devices under /sys/bus/pci/devices/

$ cat /sys/bus/pci/devices/0000:00:02.0/vendor
0x1af4
$ cat /sys/bus/pci/devices/0000:00:02.0/device
0x1050
$ ls /sys/class/drm/
card0  renderD128
$ readlink /sys/class/drm/renderD128/device
../../bus/pci/devices/0000:00:02.0
```
