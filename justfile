# Hadron kernel development recipes

# Format all source files
fmt:
    cargo fmt --all
    taplo fmt
    @echo "Formatted all files."

# Check formatting without modifying files
fmt-check:
    cargo fmt --all -- --check
    taplo fmt --check
    @echo "All formatting checks passed."

# Run all lints
lint:
    cargo xtask clippy
    taplo check
    typos
    @echo "All lint checks passed."

# Format then lint
check: fmt lint

# Build the kernel
build:
    cargo xtask build

# Build and run in QEMU
run:
    cargo xtask run

# Build and run tests in QEMU
test:
    cargo xtask test
