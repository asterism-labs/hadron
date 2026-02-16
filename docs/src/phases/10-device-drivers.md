# Phase 10: Device Drivers

## Goal

Enumerate PCI devices, implement an async block device abstraction, and add VirtIO drivers for QEMU virtual hardware. After this phase, the kernel can discover hardware and perform async block device I/O, with IRQ-to-async bridging using the same WaitQueue pattern established by existing serial and keyboard drivers.

## PCI Enumeration

PCI configuration space is accessed via I/O ports `0xCF8` (address) and `0xCFC` (data). The kernel performs brute-force bus enumeration, scanning all bus/device/function combinations:

```rust
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub bars: [Option<Bar>; 6],
}

pub enum Bar {
    Memory { base: u64, size: u64, prefetchable: bool },
    Io { base: u16, size: u16 },
}
```

BAR decoding determines whether each Base Address Register points to memory-mapped I/O or port I/O space, along with the region size (determined by writing all-ones and reading back).

## Async BlockDevice Trait

```rust
pub trait BlockDevice: Send + Sync {
    async fn read_sector(&self, sector: u64, buf: &mut [u8]) -> Result<(), IoError>;
    async fn write_sector(&self, sector: u64, buf: &[u8]) -> Result<(), IoError>;
    fn sector_size(&self) -> usize;
    fn sector_count(&self) -> u64;

    /// Read multiple contiguous sectors.
    async fn read_sectors(&self, start: u64, buf: &mut [u8]) -> Result<(), IoError> {
        let sector_size = self.sector_size();
        for (i, chunk) in buf.chunks_mut(sector_size).enumerate() {
            self.read_sector(start + i as u64, chunk).await?;
        }
        Ok(())
    }
}
```

For a ram-backed block device (used in testing), the futures resolve immediately. For VirtIO-blk, the futures await IRQ completion via WaitQueue.

## VirtIO Transport

VirtIO devices communicate through virtqueues -- shared memory structures consisting of:

- **Descriptor table**: an array of `VirtqDesc` entries, each pointing to a buffer with address, length, flags, and a chain pointer.
- **Available ring**: the driver writes descriptor indices here to submit requests to the device.
- **Used ring**: the device writes completed descriptor indices here to notify the driver.

```rust
pub struct Virtqueue {
    descriptors: &'static mut [VirtqDesc],
    available: &'static mut VirtqAvail,
    used: &'static mut VirtqUsed,
    queue_size: u16,
    free_head: u16,
    num_free: u16,
    last_used_idx: u16,
}

#[repr(C)]
pub struct VirtqDesc {
    pub addr: u64,
    pub len: u32,
    pub flags: u16,
    pub next: u16,
}
```

## VirtIO-blk Driver

The VirtIO-blk driver implements `BlockDevice` using WaitQueue for IRQ-to-async bridging:

1. The driver submits a request (read or write) by populating descriptor chain entries in the virtqueue and writing to the available ring.
2. The driver notifies the device by writing to the doorbell register.
3. The async task awaits a WaitQueue associated with the virtqueue.
4. When the device completes the request, it fires an IRQ. The IRQ handler pushes the completion to `Priority::Critical` on the executor and wakes the WaitQueue.
5. The blocked task resumes, reads the result from the used ring, and returns.

This is the same IRQ-to-async pattern used by the existing serial and keyboard drivers.

## Driver Task Integration

Driver tasks are spawned on the executor:

- Initialization and background processing run as `Priority::Normal` tasks.
- IRQ handlers enqueue wakeups at `Priority::Critical` to ensure prompt completion notification.

## Ram-Backed Block Device

A ram-backed block device (`RamDisk`) backed by a `Vec<u8>` is provided for testing. It implements the async `BlockDevice` trait with futures that resolve immediately, enabling filesystem testing without hardware.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/drivers/pci/mod.rs` | PCI subsystem, bus enumeration |
| `hadron-kernel/src/drivers/pci/config.rs` | Configuration space read/write |
| `hadron-kernel/src/drivers/pci/device.rs` | `PciDevice` struct, BAR decoding |
| `hadron-kernel/src/drivers/block/mod.rs` | Async `BlockDevice` trait |
| `hadron-kernel/src/drivers/block/ramdisk.rs` | RAM-backed block device (testing) |
| `hadron-kernel/src/drivers/virtio/mod.rs` | VirtIO transport, virtqueue |
| `hadron-kernel/src/drivers/virtio/block.rs` | VirtIO-blk driver |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| PCI config space I/O | Frame (uses existing I/O port wrappers) | Accesses I/O ports 0xCF8/0xCFC |
| PCI enumeration logic | Service | Uses safe I/O port wrappers |
| `BlockDevice` trait | Service | Abstract async interface |
| VirtIO virtqueue setup | Service | MMIO through safe abstractions |
| VirtIO-blk driver | Service | Protocol implementation over virtqueue |
| RamDisk | Service | Pure memory operations |

## Dependencies

- **Phase 4**: Virtual memory (for MMIO mappings of device BARs).
- **Phase 5**: Interrupt handling (for device IRQs, WaitQueue wakeups).
- **Phase 8**: VFS (for `/dev` integration of block devices).

## Milestone

```
PCI: Found 5 devices
  00:00.0 Host Bridge (8086:1237)
  00:01.0 ISA Bridge (8086:7000)
  00:01.1 IDE Controller (8086:7010)
  00:02.0 VGA (1234:1111)
  00:03.0 VirtIO Block (1AF4:1001)
VirtIO-blk: 1 GiB block device, 512-byte sectors
Block read test: sector 0 = [0xEB, 0x63, 0x90, ...]
```
