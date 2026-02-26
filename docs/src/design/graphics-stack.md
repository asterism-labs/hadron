# Graphics Stack Design

Hadron's graphics stack targets Vulkan via Mesa, with a custom Wayland compositor
and applications built from scratch. The stack is designed in three phases:
software rendering first (lavapipe), VirtIO GPU acceleration second
(virgl/venus), and real GPU support (AMD/Intel) as a long-term target.

## Architecture

```
┌─────────────────────────────────────┐
│  Applications (Vulkan clients)      │
├─────────────────────────────────────┤
│  Wayland (minimal subset for WSI)   │
├─────────────────────────────────────┤
│  Mesa (lavapipe / virgl / venus)    │
├─────────────────────────────────────┤
│  Kernel (DRM device, sysfs, VirtIO  │
│  GPU 3D, display output)            │
└─────────────────────────────────────┘
```

**What we port**: Mesa (Vulkan driver library), libwayland-client (wire protocol).

**What we build**: Wayland compositor, sysfs, DRM device abstraction, Unix domain
sockets, VirtIO GPU 3D extension, all applications.

## Phase Progression

### Phase 1: Lavapipe (Software Vulkan)

CPU-based Vulkan via Mesa's lavapipe driver. No GPU hardware required. This
validates the entire userspace stack — libc, Mesa compilation, Wayland WSI,
compositor integration — before introducing GPU complexity.

Kernel prerequisites: `mprotect` (shader JIT), sysfs (device discovery), Unix
domain sockets (Wayland transport), dynamic devfs.

### Phase 2: VirtIO GPU 3D (virgl/venus)

Hardware-accelerated Vulkan in QEMU. The host GPU does the real work; the kernel
shuttles command buffers via VirtIO. No IOMMU or complex memory management
required.

Kernel prerequisites: VirtIO GPU 3D command extension, DRM device node with
ioctl interface, minimal buffer sharing framework.

### Phase 3: Real GPU (AMD/Intel)

Bare-metal GPU drivers. Requires IOMMU, GEM/TTM memory management, KMS display
pipeline, firmware loading, GPU schedulers. This is the long-term aspiration;
design decisions in Phases 1-2 should not preclude it.

## Why Not procfs

Hadron uses sysfs and structured syscalls instead of procfs. The rationale:

1. **Text parsing is fragile and slow.** procfs encodes structured data as
   human-readable text that every consumer must parse. A process reading memory
   statistics parses the same formatted string that `cat /proc/meminfo` displays.
   Structured binary interfaces eliminate this overhead.

2. **The filesystem metaphor is wrong for process state.** sysfs describes
   hardware topology — buses, devices, classes — where the hierarchical file
   metaphor fits naturally. procfs describes per-process runtime state (memory
   maps, open fds, CPU affinity) where a purpose-built query interface is more
   appropriate.

3. **Information leakage.** procfs exposes system state broadly by default.
   Structured query syscalls allow fine-grained capability gating from the start.

### The Split

| Data type | Mechanism | Rationale |
|-----------|-----------|-----------|
| Hardware topology (PCI, DRM, class) | **sysfs** (`/sys/`) | Hierarchical device tree fits file metaphor |
| Per-process state (memory maps, fds) | **`sys_query`** syscall | Structured binary responses, capability-gated |
| System-wide stats (memory, uptime) | **`sys_query`** syscall | Already implemented (`QUERY_MEMORY`, `QUERY_UPTIME`) |
| Device access | **devfs** (`/dev/`) | Standard device node model |

### Mesa Impact

Mesa reads a small number of procfs paths. Each has a structured replacement:

| Linux procfs path | Hadron replacement |
|-------------------|--------------------|
| `/proc/self/maps` | `sys_query(QUERY_VMAPS)` returns `VmapEntry[]` |
| `/proc/cpuinfo` | `sys_query(QUERY_CPU_INFO)` returns `CpuInfo` struct |
| `/proc/sys/vm/mmap_min_addr` | Hardcoded in Mesa or `sys_query(QUERY_VM_CONFIG)` |

These are approximately 5-10 callsites in the Mesa codebase. The sysfs paths
Mesa reads (`/sys/bus/pci/devices/`, `/sys/class/drm/`) work as-is once sysfs
is implemented.

## DRM Ioctl Compatibility

The kernel exposes GPU devices at `/dev/dri/renderD128` (and `/dev/dri/card0`
for display) using **Linux DRM ioctl numbers with simplified semantics**. Mesa's
DRM loader has hard expectations about ioctl numbers — matching them minimizes
patching. The kernel need not implement the full Linux DRM subsystem; only the
ioctls that Mesa's specific drivers (lavapipe, virgl, venus) actually call.

Core DRM ioctls:

| Ioctl | Purpose |
|-------|---------|
| `DRM_IOCTL_VERSION` | Driver identification |
| `DRM_IOCTL_GET_CAP` / `SET_CAP` | Capability queries |
| `DRM_IOCTL_PRIME_HANDLE_TO_FD` / `FD_TO_HANDLE` | Buffer export/import |

Driver-specific ioctls (VirtIO GPU):

| Ioctl | Purpose |
|-------|---------|
| `VIRTGPU_EXECBUFFER` | Submit 3D command buffer |
| `VIRTGPU_GETPARAM` | Query GPU parameters |
| `VIRTGPU_RESOURCE_CREATE` | Allocate GPU resources |
| `VIRTGPU_MAP` | Map GPU resource to userspace |
| `VIRTGPU_WAIT` | Wait for GPU fence |

Each GPU driver implements the `Inode::ioctl` trait method. This dispatch model
scales to future AMD/Intel drivers implementing their own ioctl sets.

## Wayland Transport

Real Wayland uses Unix domain sockets with `sendmsg`/`recvmsg` for fd-passing.
Hadron implements AF_UNIX sockets rather than patching Wayland's transport layer,
because:

1. Unix domain sockets are broadly useful beyond Wayland (D-Bus, X11, many
   userspace tools).
2. It avoids forking libwayland-client with custom IPC, keeping the Mesa/Wayland
   port closer to upstream.
3. Hadron already has the fd infrastructure (`channel_send_fd`/`channel_recv_fd`)
   and the kernel VFS — UDS builds on both.

The alternative — patching libwayland to use Hadron's channel IPC — would reduce
kernel work but increase the userspace maintenance burden for every future
Wayland/Mesa update.

## Design Principles

1. **Phase 1 validates the stack.** Every component from libc to compositor is
   exercised by lavapipe before GPU hardware enters the picture.
2. **Match Linux where Mesa expects it.** DRM ioctl numbers, sysfs paths, and
   socket APIs match Linux conventions to minimize Mesa patching. Deviate only
   where Hadron has a principled alternative (procfs → sys_query).
3. **Design for real GPU from the start.** The DRM device trait, ioctl dispatch,
   and buffer sharing interfaces should be generic enough for amdgpu/i915 drivers
   to implement without redesigning the abstraction.
4. **No premature abstraction.** Build the minimal viable version of each
   component. Expand DMA-buf, KMS, and memory management only when real GPU
   support demands it.
