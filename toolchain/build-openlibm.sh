#!/usr/bin/env bash
# build-openlibm.sh — Cross-compile OpenLibm for the Hadron sysroot.
#
# Prerequisites:
#   - OpenLibm source at vendor/openlibm/ (or OPENLIBM_SRC env var)
#   - Clang + llvm-ar on PATH
#
# Usage:
#   ./toolchain/build-openlibm.sh
#
# Produces:
#   build/openlibm/libm.a
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OPENLIBM_SRC="${OPENLIBM_SRC:-$REPO_ROOT/vendor/openlibm}"
BUILD_DIR="$REPO_ROOT/build/openlibm"

CLANG="${CLANG:-clang}"
AR="${AR:-llvm-ar}"
TARGET="x86_64-unknown-none-elf"
SYSROOT="$REPO_ROOT/build/mesa-sysroot"

CFLAGS=(
    "--target=$TARGET"
    "-nostdinc"
    "-fno-exceptions"
    "-msse2"
    "-O2"
    "-ffunction-sections"
    "-fdata-sections"
    # OpenLibm internal include path
    "-I$OPENLIBM_SRC/include"
    "-I$OPENLIBM_SRC/src"
    "-I$OPENLIBM_SRC/ld80"
    "-I$OPENLIBM_SRC"
    # Hadron sysroot headers (for stdint.h, etc.)
    "-isystem" "$SYSROOT/usr/include"
    # Tell OpenLibm we're freestanding
    "-D__ELF__"
    "-DASSEMBLER"
    "-D__x86_64__"
)

# ---------------------------------------------------------------------------
# Validate prerequisites
# ---------------------------------------------------------------------------
if [[ ! -d "$OPENLIBM_SRC/src" ]]; then
    echo "ERROR: OpenLibm source not found at $OPENLIBM_SRC" >&2
    echo "       Fetch it with:" >&2
    echo "         git clone --depth 1 https://github.com/JuliaMath/openlibm.git vendor/openlibm" >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Collect source files
# ---------------------------------------------------------------------------
echo "==> Building OpenLibm..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/obj"

# Collect all .c files from src/ (common) and amd64/ (x86_64-specific).
SOURCES=()
for dir in "$OPENLIBM_SRC/src" "$OPENLIBM_SRC/ld80"; do
    if [[ -d "$dir" ]]; then
        while IFS= read -r -d '' f; do
            SOURCES+=("$f")
        done < <(find "$dir" -maxdepth 1 -name '*.c' -print0 2>/dev/null)
    fi
done

if [[ ${#SOURCES[@]} -eq 0 ]]; then
    echo "ERROR: No source files found in $OPENLIBM_SRC" >&2
    exit 1
fi

echo "    Found ${#SOURCES[@]} source files."

# ---------------------------------------------------------------------------
# Compile each file
# ---------------------------------------------------------------------------
OBJECTS=()
FAILED=0
for src in "${SOURCES[@]}"; do
    # Use directory prefix to avoid name collisions (e.g. src/e_fmodl.c vs ld80/e_fmodl.c).
    dir_prefix="$(basename "$(dirname "$src")")"
    base="$(basename "$src" .c)"
    obj="$BUILD_DIR/obj/${dir_prefix}_${base}.o"
    if "$CLANG" "${CFLAGS[@]}" -c "$src" -o "$obj" 2>/dev/null; then
        OBJECTS+=("$obj")
    else
        # Some files may not compile freestanding — skip them.
        ((FAILED++)) || true
    fi
done

echo "    Compiled ${#OBJECTS[@]} objects (${FAILED} skipped)."

# ---------------------------------------------------------------------------
# Create archive
# ---------------------------------------------------------------------------
"$AR" rcs "$BUILD_DIR/libm.a" "${OBJECTS[@]}"
echo "==> Built $BUILD_DIR/libm.a ($(ls -lh "$BUILD_DIR/libm.a" | awk '{print $5}'))"
