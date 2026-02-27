#!/usr/bin/env bash
# build-mesa.sh — Fetch, patch, configure, and cross-compile Mesa for Hadron.
#
# Usage:
#   ./ports/mesa/build-mesa.sh [REPO_ROOT] [BUILD_DIR]
#
# Defaults:
#   REPO_ROOT = project root (two directories above this script)
#   BUILD_DIR = $REPO_ROOT/build/mesa-build
#
# Prerequisites:
#   1. Run ports/mesa/sysroot-setup.sh first (assembles hadron-libc sysroot)
#   2. meson, ninja, git, and a cross-capable clang in $PATH
#   3. Python 3 in $PATH (required by Meson)
#
# Outputs:
#   $BUILD_DIR/src/vulkan/icd/libvulkan_lvp.so  — lavapipe Vulkan ICD
#   $BUILD_DIR/src/intel/vulkan/anv_icd.json    — (not built, lavapipe only)
#   $BUILD_DIR/hadron_lvp_icd.json              — ICD manifest (generated below)
#
# To use on Hadron:
#   export VK_ICD_FILENAMES=/path/to/hadron_lvp_icd.json
#   vkcube   # or any Vulkan application

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-"$(cd "$SCRIPT_DIR/../.." && pwd)"}"
BUILD_DIR="${2:-"$REPO_ROOT/build/mesa-build"}"
SYSROOT_DIR="$REPO_ROOT/build/mesa-sysroot"
MESA_SRC_DIR="$BUILD_DIR/mesa-src"
MESA_BUILD_DIR="$BUILD_DIR/mesa-obj"

MESA_VERSION="24.3.4"
MESA_TAG="mesa-$MESA_VERSION"
MESA_REPO="https://gitlab.freedesktop.org/mesa/mesa.git"

CROSS_FILE="$SCRIPT_DIR/hadron.cross"

echo "[build-mesa] REPO_ROOT    = $REPO_ROOT"
echo "[build-mesa] BUILD_DIR    = $BUILD_DIR"
echo "[build-mesa] SYSROOT_DIR  = $SYSROOT_DIR"
echo "[build-mesa] MESA_VERSION = $MESA_VERSION"

# ── Verify sysroot ────────────────────────────────────────────────────────────

if [[ ! -f "$SYSROOT_DIR/usr/lib/libhadron_libc.a" ]]; then
    echo "[build-mesa] Sysroot not found — running sysroot-setup.sh first..."
    "$SCRIPT_DIR/sysroot-setup.sh" "$REPO_ROOT" "$SYSROOT_DIR"
fi

# ── Fetch Mesa source ─────────────────────────────────────────────────────────

mkdir -p "$BUILD_DIR"

if [[ ! -d "$MESA_SRC_DIR/.git" ]]; then
    echo "[build-mesa] Cloning Mesa $MESA_VERSION..."
    git clone --depth 1 --branch "$MESA_TAG" "$MESA_REPO" "$MESA_SRC_DIR"
else
    echo "[build-mesa] Mesa source already present at $MESA_SRC_DIR"
fi

# ── Apply patches ─────────────────────────────────────────────────────────────

PATCH_DIR="$SCRIPT_DIR/patches"
APPLIED_MARK="$MESA_SRC_DIR/.hadron-patches-applied"

if [[ ! -f "$APPLIED_MARK" ]]; then
    echo "[build-mesa] Applying Hadron patches..."
    for patch in "$PATCH_DIR"/00*.patch; do
        echo "  applying: $(basename "$patch")"
        git -C "$MESA_SRC_DIR" apply "$patch"
    done
    touch "$APPLIED_MARK"
    echo "[build-mesa] All patches applied."
else
    echo "[build-mesa] Patches already applied (delete $APPLIED_MARK to re-apply)."
fi

# ── Configure with Meson ──────────────────────────────────────────────────────

if [[ ! -f "$MESA_BUILD_DIR/build.ninja" ]]; then
    echo "[build-mesa] Running Meson setup..."

    # PKG_CONFIG_PATH must point at our sysroot stubs so Mesa doesn't pick up
    # host wayland headers.
    PKG_CONFIG_PATH="$SYSROOT_DIR/usr/lib/pkgconfig" \
    meson setup \
        --cross-file "$CROSS_FILE"                   \
        --buildtype release                           \
        --prefix /usr                                 \
        -Dvulkan-drivers=swrast                       \
        -Dgallium-drivers=                            \
        -Dplatforms=wayland                           \
        -Dshared-glapi=disabled                       \
        -Dllvm=disabled                               \
        -Dgles1=disabled                              \
        -Dgles2=disabled                              \
        -Dopengl=false                                \
        -Degl=disabled                                \
        -Dglx=disabled                                \
        -Ddri3=disabled                               \
        -Dglvnd=false                                 \
        -Dzstd=disabled                               \
        -Dxmlconfig=disabled                          \
        -Dintel-clc=disabled                          \
        -Dvideo-codecs=                               \
        -Db_staticpic=false                           \
        "$MESA_SRC_DIR" "$MESA_BUILD_DIR"
else
    echo "[build-mesa] Meson build directory already configured."
fi

# ── Build ─────────────────────────────────────────────────────────────────────

echo "[build-mesa] Building Mesa (this may take several minutes)..."
ninja -C "$MESA_BUILD_DIR" -j"$(nproc)" src/vulkan/icd/libvulkan_lvp.so

# ── Generate ICD manifest ─────────────────────────────────────────────────────

LVP_SO="$MESA_BUILD_DIR/src/vulkan/icd/libvulkan_lvp.so"
ICD_JSON="$MESA_BUILD_DIR/hadron_lvp_icd.json"

if [[ -f "$LVP_SO" ]]; then
    cat > "$ICD_JSON" << EOF
{
    "file_format_version": "1.0.0",
    "ICD": {
        "library_path": "$LVP_SO",
        "api_version": "1.3.0"
    }
}
EOF
    echo ""
    echo "[build-mesa] ── Build complete ──────────────────────────────────────────────────"
    echo "[build-mesa] ICD library : $LVP_SO"
    echo "[build-mesa] ICD manifest: $ICD_JSON"
    echo ""
    echo "[build-mesa] To run Vulkan applications on Hadron:"
    echo "  export VK_ICD_FILENAMES=$ICD_JSON"
    echo "  vulkaninfo --summary"
    echo "  vkcube"
else
    echo "[build-mesa] ERROR: libvulkan_lvp.so not found after build."
    exit 1
fi
