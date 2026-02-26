# Mesa & Vulkan

## Goal

Port Mesa to Hadron to provide Vulkan API support. Phase 1 targets lavapipe
(software Vulkan) to validate the entire stack. Phase 2 enables VirtIO GPU 3D
via virgl or venus for hardware-accelerated Vulkan in QEMU. See
[Graphics Stack Design](../design/graphics-stack.md) for the architectural
rationale.

## Background

Mesa is the open-source userspace graphics library that implements Vulkan (and
OpenGL) for Linux. It contains:

- **Vulkan drivers**: lavapipe (CPU), radv (AMD), ANV (Intel), venus/virgl
  (VirtIO GPU passthrough).
- **DRM loader** (`src/loader/`): discovers GPU devices via sysfs and opens
  `/dev/dri/` nodes.
- **WSI layer** (`src/vulkan/wsi/`): window system integration — creates
  presentable surfaces via Wayland or X11.
- **OS abstractions** (`src/util/os_*`): platform-specific wrappers for mmap,
  threads, files, sockets.

Porting Mesa means: cross-compiling against hadron-libc, implementing OS
abstractions for Hadron, patching the DRM loader for Hadron's sysfs, and
replacing procfs reads with `sys_query` calls.

## Key Design

### Phase 1: Lavapipe (Software Vulkan)

Lavapipe runs entirely in userspace — the CPU executes all shading and
rasterization. No kernel GPU driver required. It validates:

- Mesa compiles against hadron-libc sysroot.
- Vulkan API calls (`vkCreateInstance`, `vkCreateDevice`, `vkCreateSwapchain`)
  work end-to-end.
- Wayland WSI presents frames to the compositor.
- Shader JIT works via `mprotect` (RW → RX page transitions).

#### Kernel Prerequisites

| Prerequisite | Status | Notes |
|--------------|--------|-------|
| `mprotect` syscall | **New** | Shader JIT: mmap RW, write code, mprotect RX |
| sysfs | **New** | Device discovery (`/sys/bus/pci/`, `/sys/class/drm/`) |
| Dynamic devfs | **New** | Runtime `/dev/dri/renderD128` creation |
| Unix domain sockets | **New** | Wayland transport |
| `sys_query` extensions | **New** | Replace procfs reads (VMAPS, CPU_INFO) |
| mmap (anonymous + device) | Done | `sys_mem_map` |
| ioctl | Done | `Inode::ioctl` |
| futex | Done | FUTEX_WAIT / FUTEX_WAKE |
| Threads + TLS | Done | `task_clone` + CLONE_SETTLS |

#### libc Additions

Mesa depends on several libc functions not yet in hadron-libc:

| Function | Implementation |
|----------|---------------|
| `poll()` | Wrapper around `event_wait_many` syscall |
| `mprotect()` | Wrapper around new `sys_mem_protect` syscall |
| `pthread_create/join/mutex/cond` | Wrap `task_clone` + `futex` |
| `socket/connect/bind/listen/accept` | AF_UNIX syscall wrappers |
| `sendmsg/recvmsg` | For fd-passing over UDS |
| `getenv/setenv` | Mesa reads `MESA_*`, `XDG_RUNTIME_DIR` env vars |

**Dynamic loading** (`dlopen`/`dlsym`): Mesa uses these to load driver shared
objects at runtime. For Phase 1, statically link the lavapipe driver into the
Mesa build to avoid implementing a dynamic linker. Phase 2 can revisit if needed.

#### Mesa Build Integration

1. Add Hadron cross-compilation target to Mesa's Meson build system.
2. Define `__hadron__` platform macro for `#ifdef` blocks.
3. Point at hadron-libc sysroot for headers and libraries.
4. Disable drivers other than lavapipe (Phase 1) to minimize build surface.

#### OS Abstraction Layer

Mesa's `src/util/os_*` files provide platform abstractions. Hadron
implementations for each:

| File | Hadron approach |
|------|-----------------|
| `os_mman.h` | Hadron's mmap is POSIX-compatible — works as-is with correct flags |
| `os_file.c` | Hadron's VFS is POSIX-compatible — open/read/write/close map directly |
| `os_thread.h` | Wrap hadron-libc pthreads (task_clone + futex) |
| `os_misc.c` | Implement `sysconf`, `getenv`; stub `sysctl` |
| `os_socket.c` | Standard UDS once AF_UNIX is implemented |

#### DRM Loader Patching

Mesa's `src/loader/` discovers GPUs by reading sysfs and opening `/dev/dri/`
nodes. With sysfs implemented, most of the loader works. Patches needed:

- Replace any `/proc/self/maps` reads with `sys_query(QUERY_VMAPS)` wrapper.
- Replace `/proc/cpuinfo` reads with `sys_query(QUERY_CPU_INFO)` wrapper.
- Ensure the PCI sysfs `resource` file format matches what Mesa's parser expects.

These are approximately 5-10 callsites across the Mesa codebase.

### Phase 2: VirtIO GPU 3D (virgl/venus)

GPU-accelerated Vulkan via VirtIO GPU 3D passthrough to the QEMU host.

#### VirtIO GPU 3D Commands

Extend the existing VirtIO GPU driver (`kernel/drivers/src/virtio/gpu.rs`) with
3D command types. The VirtIO transport (virtqueues, DMA, fences) already exists;
this adds the 3D-specific commands on top:

| Command | Purpose |
|---------|---------|
| `VIRTIO_GPU_CMD_CTX_CREATE` | Create GPU rendering context |
| `VIRTIO_GPU_CMD_CTX_DESTROY` | Destroy rendering context |
| `VIRTIO_GPU_CMD_SUBMIT_3D` | Submit command buffer for execution |
| `VIRTIO_GPU_CMD_RESOURCE_CREATE_3D` | Create 3D-capable resource |
| `VIRTIO_GPU_CMD_TRANSFER_TO_HOST_3D` | Transfer data to host resource |
| `VIRTIO_GPU_CMD_TRANSFER_FROM_HOST_3D` | Transfer data from host resource |
| `VIRTIO_GPU_CMD_CTX_ATTACH_RESOURCE` | Attach resource to context |
| `VIRTIO_GPU_CMD_CTX_DETACH_RESOURCE` | Detach resource from context |

#### DRM Device Node

Create `/dev/dri/renderD128` via dynamic devfs. The device inode implements
`Inode::ioctl` for DRM ioctls using Linux ioctl numbers (see
[Graphics Stack Design](../design/graphics-stack.md) for rationale):

```rust
pub struct DrmRenderNode {
    gpu: Arc<VirtioGpu3d>,
    /// Per-open context tracking.
    contexts: SpinLock<BTreeMap<u32, GpuContext>>,
}

impl Inode for DrmRenderNode {
    fn ioctl(&self, cmd: u32, arg: usize) -> Result<usize, FsError> {
        match cmd {
            DRM_IOCTL_VERSION => { /* return driver name/version */ }
            DRM_IOCTL_GET_CAP => { /* return capability value */ }
            VIRTGPU_EXECBUFFER => { /* submit command buffer */ }
            VIRTGPU_RESOURCE_CREATE => { /* allocate GPU resource */ }
            VIRTGPU_MAP => { /* map resource to userspace */ }
            VIRTGPU_WAIT => { /* wait for GPU fence */ }
            _ => Err(FsError::InvalidArgument),
        }
    }

    fn mmap_phys(&self) -> Option<(PhysAddr, usize)> {
        // Map GPU resource backing memory to userspace
    }
}
```

#### Buffer Sharing (Minimal DMA-buf)

For zero-copy frame presentation from GPU to compositor:

- Kernel-managed buffer objects with reference counting.
- fd-based export/import via `PRIME_HANDLE_TO_FD` / `FD_TO_HANDLE` ioctls.
- mmap support for CPU readback.
- Compositor imports buffer fds and scanouts directly.

This starts minimal — no full Linux DMA-buf framework. Expand as needed.

#### venus vs virgl

- **virgl**: Encodes Gallium3D commands, host decodes and executes via OpenGL.
  Well-tested, works with any host GPU.
- **venus**: Native Vulkan command stream passthrough. Thinner layer, better
  performance, but requires Vulkan-capable host GPU.

**Recommendation**: Target venus for native Vulkan performance. Fall back to
virgl if host Vulkan support is unavailable.

### Phase 3: Real GPU (Design Considerations)

Not implementing now. Design decisions in Phases 1-2 that matter for the future:

- **DRM ioctl trait**: Keep the `Inode::ioctl` dispatch generic so AMD/Intel
  drivers can implement the same interface.
- **Buffer objects**: The minimal DMA-buf model should be extensible to GEM/TTM.
- **KMS**: VirtIO GPU scanout is a simplified KMS path. A real KMS would add
  CRTCs, encoders, connectors, and planes.
- **IOMMU**: AMD IVRS parsing already exists in `crates/parse/acpi/src/ivrs.rs`.
  An IOMMU kernel driver is needed for device memory isolation.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `kernel/kernel/src/syscall/memory.rs` | Add `sys_mem_protect` (mprotect) |
| `kernel/kernel/src/gpu/mod.rs` | **New:** DRM device abstraction |
| `kernel/kernel/src/gpu/virtio.rs` | **New:** VirtIO GPU 3D DRM node |
| `kernel/drivers/src/virtio/gpu.rs` | Extend with 3D command types |
| `kernel/syscall/src/lib.rs` | Add `sys_mem_protect` syscall number |
| `hadron-libc/src/mman.c` | Add `mprotect()` wrapper |
| `hadron-libc/src/pthread.c` | **New:** pthreads implementation |
| `hadron-libc/src/poll.c` | **New:** `poll()` wrapper |
| Mesa source tree (vendored) | OS abstractions, DRM loader, procfs patches |

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| `mprotect` page table update | Frame | Direct page table manipulation |
| DRM ioctl dispatch | Service | Pattern matching on ioctl number |
| VirtIO GPU 3D commands | Service | Command encoding over safe virtqueue API |
| GPU resource management | Service | Bookkeeping of resource IDs and backing memory |
| Buffer object refcounting | Service | Arc-based lifecycle management |
| Mesa OS abstraction layer | Userspace | Pure C/C++ userspace code |

## Dependencies

- **sysfs**: Mesa device discovery (new).
- **Unix Domain Sockets**: Wayland transport for WSI (new).
- **Dynamic devfs**: `/dev/dri/renderD128` creation (new).
- **Wayland Minimal Subset**: WSI presentation surface (new).
- **VirtIO GPU 2D**: Existing driver provides transport infrastructure (in progress).

## Milestone

### Phase 1 (Lavapipe)

```
$ vulkaninfo --summary
Vulkan Instance Version: 1.3
GPU id  : 0 (llvmpipe (LLVM 17, 256 bits))
  driverVersion  : 0.0.1
  apiVersion     : 1.3.x

$ vkcube
[wayland] connected to compositor
[vulkan] created swapchain 800x600
[vulkan] rendering...
```

### Phase 2 (VirtIO GPU venus)

```
$ vulkaninfo --summary
GPU id  : 0 (Venus (virtio-gpu venus))
  driverVersion  : 24.x.x
  apiVersion     : 1.3.x

virtio-gpu: ctx_create (id=1, nlen=5, "venus")
virtio-gpu: submit_3d (ctx=1, size=4096)
virtio-gpu: resource_create_3d (id=2, 800x600 R8G8B8A8)
```
