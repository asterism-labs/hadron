#!/usr/bin/env bash
# build-mesa.sh — Cross-compile Mesa (lavapipe) for Hadron.
#
# Prerequisites:
#   1. Sysroot assembled: ./toolchain/sysroot-assemble.sh
#   2. Mesa source at vendor/mesa/ (or MESA_SRC env var)
#   3. Meson + Ninja installed on host
#
# Usage:
#   ./toolchain/build-mesa.sh [--reconfigure]
#
# Produces:
#   build/mesa/src/gallium/targets/lavapipe/libvulkan_lvp.a
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SYSROOT="$REPO_ROOT/build/mesa-sysroot"
MESA_SRC="${MESA_SRC:-$REPO_ROOT/vendor/mesa}"
MESA_BUILD="$REPO_ROOT/build/mesa"
CROSS_FILE="$REPO_ROOT/toolchain/hadron-x86_64.meson-cross"
PATCH_FILE="$REPO_ROOT/toolchain/mesa-hadron.patch"

# ---------------------------------------------------------------------------
# Validate prerequisites
# ---------------------------------------------------------------------------
if [[ ! -d "$SYSROOT/usr/include" ]]; then
    echo "ERROR: Sysroot not found at $SYSROOT" >&2
    echo "       Run ./toolchain/sysroot-assemble.sh first." >&2
    exit 1
fi

if [[ ! -d "$MESA_SRC" ]]; then
    echo "ERROR: Mesa source not found at $MESA_SRC" >&2
    echo "       Clone Mesa into vendor/mesa/ or set MESA_SRC." >&2
    exit 1
fi

if ! command -v meson &>/dev/null; then
    echo "ERROR: meson not found in PATH." >&2
    exit 1
fi

if ! command -v ninja &>/dev/null; then
    echo "ERROR: ninja not found in PATH." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Apply Hadron patches if not already applied
# ---------------------------------------------------------------------------
if [[ -f "$PATCH_FILE" ]]; then
    PATCH_MARKER="$MESA_SRC/.hadron-patched"
    if [[ ! -f "$PATCH_MARKER" ]]; then
        echo "==> Applying Hadron patches to Mesa..."
        (cd "$MESA_SRC" && git apply "$PATCH_FILE" 2>/dev/null || patch -p1 < "$PATCH_FILE")
        touch "$PATCH_MARKER"
    else
        echo "==> Mesa patches already applied."
    fi
fi

# ---------------------------------------------------------------------------
# Generate resolved cross file (substitute placeholders)
# ---------------------------------------------------------------------------
RESOLVED_CROSS="$REPO_ROOT/build/hadron-x86_64.meson-cross.resolved"
COMPAT_HEADER="$REPO_ROOT/toolchain/hadron_compat.h"
mkdir -p "$REPO_ROOT/build"
sed -e "s|@SYSROOT@|$SYSROOT|g" \
    -e "s|@COMPAT_HEADER@|$COMPAT_HEADER|g" \
    "$CROSS_FILE" > "$RESOLVED_CROSS"

# ---------------------------------------------------------------------------
# Configure Mesa with Meson
# ---------------------------------------------------------------------------
MESON_ARGS=(
    --cross-file "$RESOLVED_CROSS"
    --buildtype release

    # Software rasterizer only — softpipe (no LLVM dependency).
    # Lavapipe (software Vulkan) requires LLVM for JIT; softpipe does not.
    -Dvulkan-drivers=
    -Dgallium-drivers=softpipe

    # LLVM disabled — softpipe uses Mesa's built-in TGSI interpreter.
    -Dllvm=disabled

    # Disable all display/windowing backends
    -Dglx=disabled
    -Degl=disabled
    -Dopengl=false
    -Dgles1=disabled
    -Dgles2=disabled
    -Dplatforms=

    # No shared libraries or dynamic loading
    -Dshared-glapi=disabled
    -Dshared-llvm=disabled

    # No filesystem-backed shader cache (requires zlib/zstd compression)
    -Dshader-cache=disabled

    # Disable optional dependencies Mesa would probe for
    -Dzlib=disabled
    -Dzstd=disabled
    -Dlibunwind=disabled
    -Dvalgrind=disabled
    -Dselinux=false
    -Dlmsensors=disabled
    -Dxlib-lease=disabled

    # Static build
    -Ddefault_library=static

    # Disable tests and tools (we can't run them during cross-compile)
    -Dbuild-tests=false
)

if [[ "${1:-}" == "--reconfigure" ]] && [[ -d "$MESA_BUILD" ]]; then
    echo "==> Reconfiguring Mesa build..."
    meson setup "$MESA_BUILD" "$MESA_SRC" "${MESON_ARGS[@]}" --reconfigure
elif [[ ! -d "$MESA_BUILD" ]]; then
    echo "==> Configuring Mesa build..."
    meson setup "$MESA_BUILD" "$MESA_SRC" "${MESON_ARGS[@]}"
else
    echo "==> Mesa build already configured (use --reconfigure to reset)."
fi

# ---------------------------------------------------------------------------
# Build
# ---------------------------------------------------------------------------
echo "==> Building Mesa (lavapipe)..."
ninja -C "$MESA_BUILD" -j"$(nproc)"

# ---------------------------------------------------------------------------
# Report
# ---------------------------------------------------------------------------
echo ""
echo "==> Mesa build complete."
echo "    Build artifacts:"
find "$MESA_BUILD" -name "*.a" -newer "$MESA_BUILD/build.ninja" -print 2>/dev/null | head -20
