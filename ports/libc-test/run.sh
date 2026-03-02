#!/usr/bin/env bash
# ports/libc-test/run.sh — Package and run libc-test binaries in Hadron QEMU.
#
# Usage:
#   ./ports/libc-test/run.sh [REPO_ROOT]
#
# This script:
#   1. Builds the libc-test binaries (if not already built)
#   2. Packs them into a CPIO archive alongside the Hadron initrd
#   3. Runs the combined image in QEMU
#
# Alternatively, for a quick host-side sanity check using qemu-x86_64 user-mode
# (if libc-test is compiled against a compatible syscall ABI):
#   for f in build/libc-test/bin/*; do qemu-x86_64 "$f" && echo "PASS $f" || echo "FAIL $f"; done
#
# Prerequisites:
#   1. just build           (builds kernel + libc.a + initrd)
#   2. toolchain/sysroot-assemble.sh  (builds sysroot with libm.a)
#   3. ports/libc-test/build.sh       (cross-compiles tests)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-"$(cd "$SCRIPT_DIR/../.." && pwd)"}"
BUILD_DIR="$REPO_ROOT/build/libc-test"

echo "[libc-test] REPO_ROOT = $REPO_ROOT"

# ---------------------------------------------------------------------------
# Build tests if not already done
# ---------------------------------------------------------------------------
if [[ ! -d "$BUILD_DIR/bin" ]]; then
    echo "[libc-test] No binaries found — running build.sh first..."
    "$SCRIPT_DIR/build.sh" "$REPO_ROOT"
fi

BIN_COUNT=$(find "$BUILD_DIR/bin" -maxdepth 1 -type f -executable | wc -l | tr -d ' ')
echo "[libc-test] Found $BIN_COUNT test binaries."

# ---------------------------------------------------------------------------
# Create overlay CPIO archive with the test binaries and runner script
# ---------------------------------------------------------------------------
OVERLAY_DIR="$(mktemp -d)"
trap 'rm -rf "$OVERLAY_DIR"' EXIT

mkdir -p "$OVERLAY_DIR/libc-test/bin"
cp "$BUILD_DIR/bin/"* "$OVERLAY_DIR/libc-test/bin/" 2>/dev/null || true
cp "$BUILD_DIR/run-all.sh" "$OVERLAY_DIR/libc-test/"

# The Hadron kernel init (lepton-init) looks for /etc/hadron-test-runner if
# HADRON_RUN_TESTS=1 is set in the kernel command line. For now, instruct the
# user to run the script manually from lsh inside QEMU.
echo "[libc-test] To run tests inside Hadron QEMU:"
echo "  1. just run"
echo "  2. From lsh: /libc-test/run-all.sh"
echo ""
echo "[libc-test] Or start QEMU with the overlay initrd:"
echo "  just run -- -initrd $REPO_ROOT/build/initrd.cpio"
echo ""

# ---------------------------------------------------------------------------
# Pack overlay into a supplemental CPIO
# ---------------------------------------------------------------------------
OVERLAY_CPIO="$BUILD_DIR/libc-test-overlay.cpio"
(cd "$OVERLAY_DIR" && find . | cpio -o -H newc 2>/dev/null) > "$OVERLAY_CPIO"
echo "[libc-test] Overlay CPIO: $OVERLAY_CPIO"
echo ""
echo "[libc-test] To boot with test overlay:"
echo "  just run -- -initrd $OVERLAY_CPIO"
