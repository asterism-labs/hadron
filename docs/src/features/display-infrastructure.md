# Display Infrastructure

## Status

**Completed** — Framebuffer device node, mmap support, and ioctl queries are implemented.

## Overview

Display Infrastructure exposes the physical framebuffer to userspace programs via `/dev/fb0`. Userspace can memory-map the framebuffer and draw pixels directly. The framebuffer is backed by the Bochs VGA MMIO region (linear framebuffer, typical resolution 1280x720x32bpp).

## Key Types

| Type | Module | Purpose |
|------|--------|---------|
| `Framebuffer` | `driver_api::framebuffer` | Owns the MMIO framebuffer physical address range and resolution |
| `FbDeviceInode` | `fs::devfs` | VFS inode for `/dev/fb0`; implements mmap and ioctl |
| `FbInfo` | `syscall::ioctl` | User-visible framebuffer metadata (width, height, pitch, bpp, format) |

## Design Decisions

### Why mmap Instead of Direct Read/Write?

- **Performance**: Drawing pixels directly via mmap avoids syscall overhead on every pixel write.
- **Atomicity**: Userspace can produce frame buffers atomically (entire frame or nothing).
- **Compatibility**: Standard POSIX mmap semantics; familiar to graphics programmers.

### Why Bochs VGA as the Display Backend?

- **Simplicity**: Single-buffered linear framebuffer; no complex FIFO or DMA setup.
- **QEMU support**: Builtin; no additional device configuration needed.
- **Future extensibility**: Drivers can be swapped; the VFS interface remains stable.

### Write-Combining for Performance

The mmap for `/dev/fb0` uses **write-combining** cache attributes. This allows CPU stores to be coalesced before hitting the MMIO bus, dramatically improving throughput for graphics workloads.

## Implementation Status

### Completed

- `/dev/fb0` device node registration in devfs
- Framebuffer mmap support in `Inode::mmap()`
- FBIOGET_INFO ioctl returning `FbInfo`
- Write-combining cache policy on x86_64

### Design Pattern: Stateless Inode

The `FbDeviceInode` holds only an immutable reference to the physical framebuffer. Multiple processes can mmap `/dev/fb0` simultaneously; each gets its own VMA covering the same physical memory. The kernel VFS handles the multi-reader safety (device memory is inherently atomic at the hardware level).

## Files to Reference

- **Framebuffer driver**: [`kernel/hadron-kernel/src/drivers/bochs_vga.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/drivers/bochs_vga.rs) — MMIO framebuffer setup and resolution queries
- **devfs registration**: [`kernel/hadron-kernel/src/fs/devfs.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/fs/devfs.rs) — `/dev/fb0` device node
- **VFS inode trait**: [`kernel/hadron-kernel/src/fs/inode.rs`](https://github.com/anomalyco/hadron/blob/main/kernel/hadron-kernel/src/fs/inode.rs) — `mmap()` and `ioctl()` method signatures

## Example Usage

```c
// Open framebuffer device
int fd = open("/dev/fb0", O_RDWR);

// Query framebuffer info
struct fb_info info;
ioctl(fd, FBIOGET_INFO, &info);
printf("Resolution: %dx%d, Pitch: %d bytes\n",
       info.width, info.height, info.pitch);

// Map framebuffer into address space
uint32_t *fb = mmap(NULL, info.size, PROT_READ | PROT_WRITE,
                    MAP_SHARED, fd, 0);

// Draw a red rectangle at (100, 100), 50x50 pixels
uint32_t red = 0xFF0000FF;  // BGRX8888 format
for (int y = 0; y < 50; y++) {
    for (int x = 0; x < 50; x++) {
        fb[(100 + y) * (info.pitch / 4) + (100 + x)] = red;
    }
}

munmap(fb, info.size);
close(fd);
```

## Future Enhancements

- **Double buffering**: Kernel-managed front/back buffers with atomic swap
- **Hardware acceleration**: GPU driver support via ioctls
- **DRM integration**: Full DRM/KMS subsystem (complex, deferred to Network Phase 2+)

## References

- [Bochs VGA specification](https://wiki.osdev.org/Bochs_VGA)
- [Memory-Mapped I/O guide](https://wiki.osdev.org/Memory_Mapped_IO)
- POSIX mmap semantics
