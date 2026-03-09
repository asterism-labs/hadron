#!/usr/bin/env bash
# sysroot-assemble.sh — Build and assemble a C/C++ sysroot for cross-compiling
# Mesa (and other C libraries) against Hadron's libc.
#
# Usage: ./toolchain/sysroot-assemble.sh [--skip-build]
#
# Produces:
#   build/mesa-sysroot/usr/include/  — C headers from hadron-libc
#   build/mesa-sysroot/usr/lib/      — libc.a, libm.a, librt.a, libpthread.a, CRT objects
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SYSROOT="$REPO_ROOT/build/mesa-sysroot"
LIBC_A="$REPO_ROOT/build/kernel/x86_64-unknown-hadron-user/debug/libhadron_libc.a"
LIBC_INCLUDE="$REPO_ROOT/userspace/hadron-libc/include"
CXXABI_SRC="$REPO_ROOT/toolchain/cxxabi_stubs.cpp"

CLANG="${CLANG:-clang}"
AR="${AR:-llvm-ar}"
TARGET="x86_64-unknown-none-elf"
CFLAGS="--target=$TARGET -nostdinc -nostdlib -fno-exceptions -fno-rtti -msse2"

# ---------------------------------------------------------------------------
# Step 0: Optionally build libc.a via gluon
# ---------------------------------------------------------------------------
if [[ "${1:-}" != "--skip-build" ]]; then
    echo "==> Building libc.a via 'just build'..."
    (cd "$REPO_ROOT" && just build)
fi

if [[ ! -f "$LIBC_A" ]]; then
    echo "ERROR: libc.a not found at $LIBC_A" >&2
    echo "       Run 'just build' first or check the build output." >&2
    exit 1
fi

# ---------------------------------------------------------------------------
# Step 1: Create sysroot directory structure
# ---------------------------------------------------------------------------
echo "==> Assembling sysroot at $SYSROOT..."
rm -rf "$SYSROOT"
mkdir -p "$SYSROOT/usr/include" "$SYSROOT/usr/lib"

# ---------------------------------------------------------------------------
# Step 2: Copy headers
# ---------------------------------------------------------------------------
echo "    Copying headers from $LIBC_INCLUDE..."
cp -r "$LIBC_INCLUDE"/* "$SYSROOT/usr/include/"

# Copy clang builtin headers (stdalign.h, immintrin.h, float.h, etc.)
# that -nostdinc strips away. Use -n to not overwrite our libc headers.
CLANG_RESOURCE_DIR="$("$CLANG" -print-resource-dir)/include"
if [[ -d "$CLANG_RESOURCE_DIR" ]]; then
    echo "    Copying clang builtin headers from $CLANG_RESOURCE_DIR..."
    # macOS cp -rn exits 1 when any files are skipped due to -n; suppress that.
    cp -rn "$CLANG_RESOURCE_DIR"/* "$SYSROOT/usr/include/" || true
fi

# ---------------------------------------------------------------------------
# Step 3: Copy libc.a
# ---------------------------------------------------------------------------
echo "    Copying libc.a..."
cp "$LIBC_A" "$SYSROOT/usr/lib/libc.a"

# ---------------------------------------------------------------------------
# Step 4: Create stub archives (libm.a, librt.a, libpthread.a, libdl.a)
#
# These are either empty (for libs we don't need yet) or thin re-exports.
# Mesa links against -lm -lpthread -lrt -ldl; providing empty archives
# satisfies the linker when all needed symbols are actually in libc.a.
# ---------------------------------------------------------------------------
echo "    Creating stub archives..."
for lib in libm librt libpthread libdl; do
    "$AR" rcs "$SYSROOT/usr/lib/${lib}.a"
done

# ---------------------------------------------------------------------------
# Step 5: Create minimal CRT objects
#
# Mesa is a library, not an executable, so these just need to exist to
# satisfy Meson's compiler detection. They contain minimal .init/.fini
# and _start symbols.
# ---------------------------------------------------------------------------
echo "    Creating CRT objects..."

CRT_TMPDIR="$(mktemp -d)"
trap 'rm -rf "$CRT_TMPDIR"' EXIT

# crt1.o — provides _start (Mesa won't actually use it for library builds)
cat > "$CRT_TMPDIR/crt1.S" << 'ASM'
.section .text
.global _start
.type _start, @function
_start:
    xorl %ebp, %ebp
    movq %rsp, %rdi
    andq $-16, %rsp
    call main
    movl %eax, %edi
    call _exit
.size _start, . - _start
ASM

# crti.o — function prologue for .init/.fini sections
cat > "$CRT_TMPDIR/crti.S" << 'ASM'
.section .init
.global _init
.type _init, @function
_init:
    pushq %rbp
    movq %rsp, %rbp

.section .fini
.global _fini
.type _fini, @function
_fini:
    pushq %rbp
    movq %rsp, %rbp
ASM

# crtn.o — function epilogue for .init/.fini sections
cat > "$CRT_TMPDIR/crtn.S" << 'ASM'
.section .init
    popq %rbp
    ret

.section .fini
    popq %rbp
    ret
ASM

for src in crt1 crti crtn; do
    "$CLANG" --target="$TARGET" -c "$CRT_TMPDIR/${src}.S" -o "$SYSROOT/usr/lib/${src}.o"
done

# ---------------------------------------------------------------------------
# Step 6: Build C++ ABI stubs
# ---------------------------------------------------------------------------
if [[ -f "$CXXABI_SRC" ]]; then
    echo "    Building C++ ABI stubs..."
    "$CLANG" $CFLAGS -isystem "$SYSROOT/usr/include" \
        -c "$CXXABI_SRC" -o "$SYSROOT/usr/lib/cxxabi_stubs.o"
fi

# ---------------------------------------------------------------------------
# Step 7: Build OpenLibm (libm.a)
#
# build-openlibm.sh auto-fetches from GitHub if vendor/openlibm/ is absent.
# ---------------------------------------------------------------------------
echo "    Building OpenLibm (auto-fetches if needed)..."
"$REPO_ROOT/toolchain/build-openlibm.sh"
if [[ -f "$REPO_ROOT/build/openlibm/libm.a" ]]; then
    # Overwrite the empty stub archive created in Step 4.
    cp "$REPO_ROOT/build/openlibm/libm.a" "$SYSROOT/usr/lib/libm.a"
else
    echo "WARNING: build-openlibm.sh did not produce libm.a — using empty stub." >&2
fi

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
echo ""
echo "==> Sysroot assembled at: $SYSROOT"
echo "    Headers:  $SYSROOT/usr/include/"
echo "    Libraries: $SYSROOT/usr/lib/"
ls -la "$SYSROOT/usr/lib/"
