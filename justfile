# Hadron kernel development recipes

hb := "./tools/hadron-build/target/release/hadron-build"

# Build the build tool itself
bootstrap:
    cargo build --manifest-path tools/hadron-build/Cargo.toml --release --quiet

# Resolve config + generate rust-project.json
configure *args: bootstrap
    {{hb}} configure {{args}}

# Build the kernel
build *args: bootstrap
    {{hb}} build {{args}}

# Build and run in QEMU
run *args: bootstrap
    {{hb}} run {{args}}

# Run tests
test *args: bootstrap
    {{hb}} test {{args}}

# Type-check without linking
check: bootstrap
    {{hb}} check

# Run clippy lints
clippy: bootstrap
    {{hb}} clippy

# Format source files
fmt *args: bootstrap
    {{hb}} fmt {{args}}

# Remove build artifacts
clean:
    {{hb}} clean
