#!/usr/bin/env bash
# ports/openlibm/sysroot-setup.sh — Install libm.a into the Hadron sysroot.
#
# Usage:
#   ./ports/openlibm/sysroot-setup.sh [REPO_ROOT] [SYSROOT_DIR]
#
# Defaults:
#   REPO_ROOT   = directory two levels above this script (project root)
#   SYSROOT_DIR = $REPO_ROOT/build/mesa-sysroot
#
# After this script:
#   <sysroot>/usr/lib/libm.a  — OpenLibm math library

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-"$(cd "$SCRIPT_DIR/../.." && pwd)"}"
SYSROOT_DIR="${2:-"$REPO_ROOT/build/mesa-sysroot"}"

echo "[openlibm] REPO_ROOT   = $REPO_ROOT"
echo "[openlibm] SYSROOT_DIR = $SYSROOT_DIR"

LIBM_A="$REPO_ROOT/build/openlibm/libm.a"

# Build if libm.a is not yet present.
if [[ ! -f "$LIBM_A" ]]; then
    echo "[openlibm] libm.a not found — building..."
    "$SCRIPT_DIR/build.sh" "$REPO_ROOT"
fi

# Install into sysroot.
mkdir -p "$SYSROOT_DIR/usr/lib"
cp "$LIBM_A" "$SYSROOT_DIR/usr/lib/libm.a"
echo "[openlibm] Installed $LIBM_A -> $SYSROOT_DIR/usr/lib/libm.a"

# Copy public headers (math.h is already in hadron-libc, but openlibm ships
# openlibm_math.h which exposes internal detail — skip it to avoid conflicts).
echo "[openlibm] Sysroot setup complete."
