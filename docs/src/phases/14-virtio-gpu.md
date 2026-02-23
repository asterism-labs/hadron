# Phase 14: VirtIO GPU 2D Driver

## Goal

Implement the VirtIO GPU 2D protocol over virtqueues, providing structured resource management for display output. Replace or supplement the Bochs VGA dumb framebuffer with a proper display protocol that supports resource creation, host-side transfer, scanout configuration, and hardware cursor. After this phase, the kernel has a modern display path that works with QEMU's `virtio-gpu` device.

## Background

The Bochs VGA driver from Phase 10 provides a simple linear framebuffer where the CPU does all rendering via direct memory writes. VirtIO GPU 2D adds a structured command protocol over virtqueues:

- **Resources** are GPU-side memory objects (2D images) with defined formats and dimensions.
- The guest creates resources, attaches backing memory, renders into them on the CPU side, then **transfers** the updated regions to the host.
- The host **scans out** a resource to the display, handling the actual screen update.

This is still software-rendered on the guest side, but the display protocol is proper and efficient. The existing virtqueue infrastructure from Phase 10 (VirtIO block/net) makes this straightforward.

## Key Design

### VirtIO GPU Command Protocol

The driver communicates with the device via a control virtqueue. Each command is a request/response pair:

| Command | Purpose |
|---------|---------|
| `VIRTIO_GPU_CMD_GET_DISPLAY_INFO` | Query available displays (resolution, enabled) |
| `VIRTIO_GPU_CMD_RESOURCE_CREATE_2D` | Create a 2D resource with format and dimensions |
| `VIRTIO_GPU_CMD_RESOURCE_UNREF` | Destroy a resource |
| `VIRTIO_GPU_CMD_SET_SCANOUT` | Assign a resource to a display scanout |
| `VIRTIO_GPU_CMD_RESOURCE_FLUSH` | Flush a region of a resource to the display |
| `VIRTIO_GPU_CMD_TRANSFER_TO_HOST_2D` | Transfer guest memory to host resource |
| `VIRTIO_GPU_CMD_RESOURCE_ATTACH_BACKING` | Attach guest memory pages to a resource |
| `VIRTIO_GPU_CMD_RESOURCE_DETACH_BACKING` | Detach guest memory from a resource |
| `VIRTIO_GPU_CMD_UPDATE_CURSOR` | Set or update the hardware cursor |
| `VIRTIO_GPU_CMD_MOVE_CURSOR` | Move the hardware cursor position |

### Driver Structure

```rust
pub struct VirtioGpu {
    control_queue: Virtqueue,
    cursor_queue: Virtqueue,
    displays: Vec<DisplayInfo>,
    next_resource_id: u32,
}

#[repr(C)]
pub struct DisplayInfo {
    pub rect: Rect,       // x, y, width, height
    pub enabled: bool,
    pub flags: u32,
}

impl VirtioGpu {
    /// Query the device for available displays.
    pub async fn get_display_info(&mut self) -> Result<Vec<DisplayInfo>, GpuError> {
        let req = VirtioGpuGetDisplayInfo {};
        let resp = self.control_queue.send_recv(&req).await?;
        // Parse display info from response
    }

    /// Create a 2D resource on the host.
    pub async fn create_resource_2d(
        &mut self,
        width: u32,
        height: u32,
        format: VirtioGpuFormat,
    ) -> Result<u32, GpuError> {
        let resource_id = self.next_resource_id;
        self.next_resource_id += 1;
        let req = VirtioGpuResourceCreate2d {
            resource_id,
            format,
            width,
            height,
        };
        self.control_queue.send_recv(&req).await?;
        Ok(resource_id)
    }

    /// Attach guest memory as backing storage for a resource.
    pub async fn attach_backing(
        &mut self,
        resource_id: u32,
        pages: &[PhysAddr],
    ) -> Result<(), GpuError> {
        let req = VirtioGpuResourceAttachBacking {
            resource_id,
            entries: pages.iter().map(|p| VirtioGpuMemEntry {
                addr: p.as_u64(),
                length: PAGE_SIZE as u32,
            }).collect(),
        };
        self.control_queue.send_recv(&req).await
    }

    /// Transfer a region from guest backing memory to host resource.
    pub async fn transfer_to_host_2d(
        &mut self,
        resource_id: u32,
        rect: Rect,
    ) -> Result<(), GpuError> {
        let req = VirtioGpuTransferToHost2d {
            resource_id,
            rect,
            offset: 0,
        };
        self.control_queue.send_recv(&req).await
    }

    /// Set a resource as the scanout source for a display.
    pub async fn set_scanout(
        &mut self,
        scanout_id: u32,
        resource_id: u32,
        rect: Rect,
    ) -> Result<(), GpuError> {
        let req = VirtioGpuSetScanout {
            scanout_id,
            resource_id,
            rect,
        };
        self.control_queue.send_recv(&req).await
    }

    /// Flush a region of the scanout resource to the display.
    pub async fn resource_flush(
        &mut self,
        resource_id: u32,
        rect: Rect,
    ) -> Result<(), GpuError> {
        let req = VirtioGpuResourceFlush {
            resource_id,
            rect,
        };
        self.control_queue.send_recv(&req).await
    }

    /// Update the hardware cursor image and position.
    pub async fn update_cursor(
        &mut self,
        resource_id: u32,
        x: u32,
        y: u32,
        hot_x: u32,
        hot_y: u32,
    ) -> Result<(), GpuError> {
        let req = VirtioGpuUpdateCursor {
            scanout_id: 0,
            x, y,
            resource_id,
            hot_x, hot_y,
        };
        self.cursor_queue.send_recv(&req).await
    }
}
```

### Display Update Flow

1. Userspace renders into a buffer (mapped via `/dev/fb0` or a new `/dev/gpu0`).
2. Kernel (or userspace via ioctl) calls `transfer_to_host_2d` to push dirty regions to the host.
3. Kernel calls `resource_flush` to update the physical display.

For simple use cases, the driver can expose a framebuffer-compatible interface where writes to the backing memory are periodically flushed. For more advanced use, ioctls allow explicit transfer and flush control.

### Hardware Cursor

VirtIO GPU supports a hardware cursor via the cursor virtqueue. This offloads cursor rendering from the CPU and provides tear-free, low-latency cursor movement — essential for the compositor in Phase 15.

```rust
// Create a 64x64 RGBA cursor resource
let cursor_id = gpu.create_resource_2d(64, 64, FORMAT_BGRA8888).await?;
gpu.attach_backing(cursor_id, &cursor_pages).await?;
// Upload cursor image
gpu.transfer_to_host_2d(cursor_id, Rect::new(0, 0, 64, 64)).await?;
// Set cursor position and hotspot
gpu.update_cursor(cursor_id, mouse_x, mouse_y, 0, 0).await?;
```

### Integration with Phase 13

The VirtIO GPU driver can serve as an alternative backend for `/dev/fb0`. When available, the devfs framebuffer node uses VirtIO GPU resources instead of raw Bochs VGA BAR memory. The `sys_mmap` interface remains the same — userspace maps the resource's backing memory and draws as before.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-drivers/src/virtio/gpu.rs` | **New:** VirtIO GPU 2D driver |
| `hadron-drivers/src/virtio/mod.rs` | Register GPU device type in VirtIO probe |
| `hadron-kernel/src/driver_api/gpu.rs` | **New:** GPU device trait (create resource, transfer, flush) |
| `hadron-kernel/src/drivers/display.rs` | Update to support VirtIO GPU as framebuffer backend |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| VirtIO GPU command encoding | Service | Byte-level protocol over safe virtqueue API |
| Virtqueue send/recv | Service | Uses existing VirtIO transport from Phase 10 |
| Resource management | Service | Bookkeeping of resource IDs and backing memory |
| Hardware cursor | Service | Commands over cursor virtqueue |
| Physical page allocation for backing | Frame | Physical memory allocation |

## Dependencies

- **Phase 10**: VirtIO transport (virtqueue infrastructure, device discovery).
- **Phase 13**: Input & display infrastructure (devfs framebuffer node, sys_mmap).

## Milestone

```
virtio-gpu: device found, 1 display (1280x720)
virtio-gpu: created resource 1 (1280x720 BGRX8888)
virtio-gpu: attached 900 backing pages
virtio-gpu: scanout 0 -> resource 1
virtio-gpu: hardware cursor enabled (64x64)

[userspace] drawing via virtio-gpu... flush
[userspace] cursor moving at (640, 360)
```
