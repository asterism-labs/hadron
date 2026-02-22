# Hadron kernel development recipes

hb := "./target/release/gluon"

# Build the build tool itself
bootstrap:
    cargo build --package gluon --release --quiet

# Resolve config + generate rust-project.json
configure *args: bootstrap
    {{hb}} configure {{args}}

# Configure build options using a TUI Menu
menuconfig *args: bootstrap
    {{hb}} menuconfig {{args}}

# Build the kernel
build *args: bootstrap
    {{hb}} build {{args}}

vendor *args: bootstrap
    {{hb}} vendor {{args}}

# Build and run in QEMU
run *args: bootstrap
    {{hb}} run {{args}}

# Run tests
test *args: bootstrap
    {{hb}} test {{args}}

# Run kernel benchmarks
bench *args: bootstrap
    {{hb}} bench {{args}}

# Type-check without linking
check *args: bootstrap
    {{hb}} check {{args}}

# Run clippy lints
clippy *args: bootstrap
    {{hb}} clippy {{args}}

# Format source files
fmt *args: bootstrap
    {{hb}} fmt {{args}}

# Run miri on hadron-core sync primitives
miri *args:
    cargo +nightly miri test -p hadron-core -- sync:: {{args}}

# Run loom concurrency tests on hadron-core
loom *args:
    RUSTFLAGS="--cfg loom" cargo test -p hadron-core {{args}}

# Analyze profiling data (perf report, perf record)
perf *args: bootstrap
    {{hb}} perf {{args}}

# Remove build artifacts
clean *args: bootstrap
    {{hb}} clean {{args}}
