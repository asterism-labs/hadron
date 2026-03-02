#!/usr/bin/env bash
# ports/openlibm/build.sh — Build OpenLibm for the Hadron sysroot.
#
# Usage:
#   ./ports/openlibm/build.sh [REPO_ROOT]
#
# Defaults:
#   REPO_ROOT = directory two levels above this script (project root)
#
# This script delegates to toolchain/build-openlibm.sh which auto-fetches
# the OpenLibm source from GitHub if vendor/openlibm/ is absent.
#
# Produces:
#   build/openlibm/libm.a

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-"$(cd "$SCRIPT_DIR/../.." && pwd)"}"

echo "[openlibm] REPO_ROOT = $REPO_ROOT"

"$REPO_ROOT/toolchain/build-openlibm.sh"

LIBM_A="$REPO_ROOT/build/openlibm/libm.a"
if [[ ! -f "$LIBM_A" ]]; then
    echo "[openlibm] ERROR: build did not produce $LIBM_A" >&2
    exit 1
fi

echo "[openlibm] Build complete: $LIBM_A ($(du -h "$LIBM_A" | cut -f1))"
