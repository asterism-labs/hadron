#!/usr/bin/env bash
# ports/libc-test/build.sh — Cross-compile musl libc-test for Hadron.
#
# Usage:
#   ./ports/libc-test/build.sh [REPO_ROOT] [BUILD_DIR]
#
# Defaults:
#   REPO_ROOT = directory two levels above this script (project root)
#   BUILD_DIR = $REPO_ROOT/build/libc-test
#
# Prerequisites:
#   1. Run toolchain/sysroot-assemble.sh first (assembles hadron sysroot with libm.a)
#   2. clang on PATH
#
# Produces:
#   $BUILD_DIR/bin/<test>   — individual statically linked test binaries
#   $BUILD_DIR/run-all.sh   — script that runs all binaries in QEMU user mode

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="${1:-"$(cd "$SCRIPT_DIR/../.." && pwd)"}"
BUILD_DIR="${2:-"$REPO_ROOT/build/libc-test"}"
SRC_DIR="$BUILD_DIR/src"
SYSROOT="$REPO_ROOT/build/mesa-sysroot"

LIBC_TEST_REPO="git://repo.or.cz/libc-test"
LIBC_TEST_COMMIT="HEAD"

CLANG="${CLANG:-clang}"
TARGET="x86_64-unknown-none-elf"

echo "[libc-test] REPO_ROOT = $REPO_ROOT"
echo "[libc-test] BUILD_DIR = $BUILD_DIR"
echo "[libc-test] SYSROOT   = $SYSROOT"

# ---------------------------------------------------------------------------
# Validate sysroot
# ---------------------------------------------------------------------------
if [[ ! -f "$SYSROOT/usr/lib/libc.a" ]]; then
    echo "[libc-test] Sysroot not found — running sysroot-assemble.sh first..."
    "$REPO_ROOT/toolchain/sysroot-assemble.sh" --skip-build
fi

# ---------------------------------------------------------------------------
# Fetch libc-test source
# ---------------------------------------------------------------------------
if [[ ! -d "$SRC_DIR/.git" ]]; then
    echo "[libc-test] Fetching libc-test source..."
    mkdir -p "$(dirname "$SRC_DIR")"
    git clone --depth 1 "$LIBC_TEST_REPO" "$SRC_DIR"
fi

# ---------------------------------------------------------------------------
# Compile individual test files
# ---------------------------------------------------------------------------
mkdir -p "$BUILD_DIR/bin"

# Test suites to cross-compile (ordered by dependency complexity):
#   string    — strlen, strcpy, etc.         (no syscalls beyond write)
#   ctype     — isalpha, isdigit, etc.       (pure table lookups)
#   math      — sqrt, sin, pow, etc.         (needs libm)
#   regression — miscellaneous regression    (may use fork/exec — skip if fail)
TEST_DIRS=(
    "src/string"
    "src/regression"
    "src/math"
)

CFLAGS=(
    "--target=$TARGET"
    "-nostdinc"
    "--sysroot=$SYSROOT"
    "-isystem" "$SYSROOT/usr/include"
    "-I$SRC_DIR/src"
    "-O1"
    "-fno-stack-protector"
    # libc-test uses T() macro for test registration — needs main() from libc-test
    "-DTEST_INCLUDE_MAIN"
)

LDFLAGS=(
    "--target=$TARGET"
    "-nostdlib"
    "-static"
    "-L$SYSROOT/usr/lib"
    "$SYSROOT/usr/lib/crt1.o"
    "$SYSROOT/usr/lib/crti.o"
    "-lc"
    "-lm"
    "$SYSROOT/usr/lib/crtn.o"
)

PASS=0
FAIL=0
SKIP=0

for dir in "${TEST_DIRS[@]}"; do
    test_src_dir="$SRC_DIR/$dir"
    [[ -d "$test_src_dir" ]] || { echo "[libc-test] Skipping missing $dir"; ((SKIP++)) || true; continue; }

    # Each .c file in the directory is a standalone test.
    while IFS= read -r -d '' src; do
        name="$(basename "$src" .c)"
        out="$BUILD_DIR/bin/${dir//\//_}_${name}"

        # Compile and link in one step.
        if "$CLANG" "${CFLAGS[@]}" "${LDFLAGS[@]}" "$src" -o "$out" 2>/dev/null; then
            ((PASS++)) || true
        else
            ((FAIL++)) || true
        fi
    done < <(find "$test_src_dir" -maxdepth 1 -name '*.c' -print0 2>/dev/null)
done

echo "[libc-test] Built: $PASS binaries, $FAIL failed to compile, $SKIP dirs skipped."

# ---------------------------------------------------------------------------
# Generate run-all.sh (for use inside QEMU via initrd)
# ---------------------------------------------------------------------------
cat > "$BUILD_DIR/run-all.sh" << 'RUNNER'
#!/bin/sh
# Run all libc-test binaries and report pass/fail counts.
PASS=0; FAIL=0
for t in /libc-test/bin/*; do
    [ -x "$t" ] || continue
    name="$(basename "$t")"
    if "$t" >/dev/null 2>&1; then
        echo "  PASS $name"
        PASS=$((PASS + 1))
    else
        echo "  FAIL $name (exit $?)"
        FAIL=$((FAIL + 1))
    fi
done
echo ""
echo "libc-test: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ] && exit 0 || exit 1
RUNNER
chmod +x "$BUILD_DIR/run-all.sh"

echo "[libc-test] Build complete. Binaries in $BUILD_DIR/bin/"
echo "[libc-test] See ports/libc-test/run.sh to run in QEMU."
