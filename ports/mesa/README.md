# Hadron Mesa Port — Phase 1 (lavapipe / software Vulkan)

This directory contains everything needed to cross-compile Mesa's lavapipe
(CPU software renderer) for Hadron and connect it to the Lepton Wayland
compositor.

## Quick Start

```sh
# 1. Build hadron-libc first (needed for the sysroot)
just build

# 2. Assemble the cross-compilation sysroot
./ports/mesa/sysroot-setup.sh

# 3. Fetch Mesa, apply patches, and build
./ports/mesa/build-mesa.sh

# 4. Copy ICD library and manifest to the initrd
#    (see build/mesa-build/hadron_lvp_icd.json for the manifest path)
```

## Directory Layout

```
ports/mesa/
├── README.md              — this file
├── hadron.cross           — Meson cross-compilation file (patched by sysroot-setup.sh)
├── sysroot-setup.sh       — assembles hadron-libc sysroot under build/mesa-sysroot/
├── build-mesa.sh          — fetches Mesa 24.x, applies patches, runs Meson + ninja
└── patches/
    ├── 0001-detect-os-hadron.patch      — add DETECT_OS_HADRON macro
    ├── 0002-os-misc-hadron.patch        — sysconf → hadron_query_cpu_info
    ├── 0003-replace-procfs-reads.patch  — /proc/cpuinfo + /proc/self/maps → sys_query
    ├── 0004-wayland-wsi-hadron.patch    — XDG_RUNTIME_DIR default /run
    ├── 0005-drm-loader-no-llvm-jit.patch — disable LLVM JIT (Phase 1)
    └── 0006-drm-loader-hadron-fallback.patch — empty DRM sysfs scan on Hadron
```

## Architecture

```
vkcube / Vulkan app
    │  VK_ICD_FILENAMES=hadron_lvp_icd.json
    ▼
libvulkan_lvp.so (lavapipe — CPU software renderer)
    │  Wayland WSI (wsi_common_wayland.c)
    │  connects to /run/wayland-0
    ▼
lepton-compositor (Wayland server)
    │  wl_shm shared memory blits
    ▼
/dev/fb0 (kernel framebuffer)
    ▼
VirtIO GPU 2D / Bochs VGA / VBoxVGA
```

## Patches Summary

| Patch | Description |
|-------|-------------|
| `0001` | Adds `DETECT_OS_HADRON` to `src/util/detect_os.h` |
| `0002` | `os_get_num_cpu()` uses `hadron_query_cpu_info()` instead of `sysconf` |
| `0003` | Replaces `/proc/self/maps` and `/proc/cpuinfo` with `sys_query` calls |
| `0004` | Wayland WSI sets `XDG_RUNTIME_DIR=/run` if unset |
| `0005` | Forces lavapipe non-JIT path (LLVM cross-compilation deferred to Phase 2) |
| `0006` | DRM loader returns empty list on Hadron (lavapipe uses `VK_ICD_FILENAMES`) |

## Phase 2 TODOs

- Enable LLVM JIT once `libLLVM.a` is available in the Hadron sysroot (remove patch 0005)
- Add VirtIO GPU 3D / venus acceleration driver
- Support DMA-buf buffer sharing between compositor and GPU driver
