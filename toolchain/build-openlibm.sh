#!/usr/bin/env bash
# build-openlibm.sh — Cross-compile OpenLibm for the Hadron sysroot.
#
# Auto-fetches OpenLibm from GitHub if vendor/openlibm/ does not exist.
#
# Prerequisites:
#   - git, clang, llvm-ar on PATH
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
# Use the Hadron user ELF target; close enough to none-elf for a C-only build.
TARGET="x86_64-unknown-none-elf"
# Use the libc include directory directly — no dependency on the assembled sysroot.
LIBC_INCLUDE="$REPO_ROOT/userspace/hadron-libc/include"

CFLAGS=(
    "--target=$TARGET"
    "-nostdinc"
    "-fno-exceptions"
    "-msse4.2"
    "-O2"
    "-ffunction-sections"
    "-fdata-sections"
    # OpenLibm internal include paths
    "-I$OPENLIBM_SRC/include"
    "-I$OPENLIBM_SRC/src"
    "-I$OPENLIBM_SRC/ld80"
    "-I$OPENLIBM_SRC"
    # Hadron libc headers (for stdint.h, float.h, etc.)
    "-isystem" "$LIBC_INCLUDE"
    # Mark this as a standalone openlibm build (not wrapping system libm)
    "-DOPENLIBM"
    "-D__ELF__"
    "-D__x86_64__"
    # Enable BSD/XSI extensions in openlibm_math.h (needed for M_PI_4, signgam, etc.)
    "-D__BSD_VISIBLE=1"
)

ASFLAGS=(
    "--target=$TARGET"
    "-msse4.2"
    "-I$OPENLIBM_SRC/include"
    "-I$OPENLIBM_SRC/src"
    "-I$OPENLIBM_SRC/amd64"
    "-D__ELF__"
    "-D__x86_64__"
)

# ---------------------------------------------------------------------------
# Auto-fetch OpenLibm if missing
# ---------------------------------------------------------------------------
if [[ ! -d "$OPENLIBM_SRC/src" ]]; then
    echo "==> OpenLibm source not found — fetching from GitHub..."
    mkdir -p "$(dirname "$OPENLIBM_SRC")"
    git clone --depth 1 https://github.com/JuliaMath/openlibm.git "$OPENLIBM_SRC"
fi

# ---------------------------------------------------------------------------
# Collect source files
# ---------------------------------------------------------------------------
echo "==> Building OpenLibm..."
rm -rf "$BUILD_DIR"
mkdir -p "$BUILD_DIR/obj"

# Collect .c files from src/ (portable) and ld80/ (80-bit long double).
SOURCES=()
for dir in "$OPENLIBM_SRC/src" "$OPENLIBM_SRC/ld80"; do
    if [[ -d "$dir" ]]; then
        while IFS= read -r -d '' f; do
            SOURCES+=("$f")
        done < <(find "$dir" -maxdepth 1 -name '*.c' -print0 2>/dev/null)
    fi
done

# Collect x86_64 assembly optimisations from amd64/ (.S files).
ASM_SOURCES=()
if [[ -d "$OPENLIBM_SRC/amd64" ]]; then
    while IFS= read -r -d '' f; do
        ASM_SOURCES+=("$f")
    done < <(find "$OPENLIBM_SRC/amd64" -maxdepth 1 -name '*.S' -print0 2>/dev/null)
fi

if [[ ${#SOURCES[@]} -eq 0 ]]; then
    echo "ERROR: No source files found in $OPENLIBM_SRC" >&2
    exit 1
fi

echo "    Found ${#SOURCES[@]} C files, ${#ASM_SOURCES[@]} assembly files."

# ---------------------------------------------------------------------------
# Compile C files
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
        # Some files may not compile freestanding — skip them with a note.
        ((FAILED++)) || true
    fi
done

# Compile assembly files (override C function with SSE-optimized version).
for src in "${ASM_SOURCES[@]}"; do
    base="$(basename "$src" .S)"
    obj="$BUILD_DIR/obj/amd64_${base}.o"
    if "$CLANG" "${ASFLAGS[@]}" -c "$src" -o "$obj" 2>/dev/null; then
        OBJECTS+=("$obj")
    else
        ((FAILED++)) || true
    fi
done

echo "    Compiled ${#OBJECTS[@]} objects (${FAILED} skipped)."

# ---------------------------------------------------------------------------
# Create archive
# ---------------------------------------------------------------------------
"$AR" rcs "$BUILD_DIR/libm.a" "${OBJECTS[@]}"
echo "==> Built $BUILD_DIR/libm.a ($(ls -lh "$BUILD_DIR/libm.a" | awk '{print $5}'))"
