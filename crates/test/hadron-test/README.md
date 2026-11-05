# hadron-test

A `no_std` test harness for Hadron kernel integration tests, designed to run test binaries inside QEMU and report results via serial output and QEMU exit codes. The harness integrates with Rust's `custom_test_frameworks` feature, providing a drop-in test runner that handles argument parsing, test filtering, lifecycle hooks, and pass/fail reporting. Test results are communicated through the QEMU ISA debug-exit device (exit code 33 for success, 35 for failure).

## Features

- Custom test runner compatible with `#![test_runner(hadron_test::test_runner)]` and `#[test_case]` attributes
- `test_entry_point!` macro for minimal Limine-booted test binaries with serial and command-line setup
- `test_entry_point_with_init!` macro for tests requiring full kernel initialization (PMM, VMM, heap allocator)
- `uefi_test_entry_point!` macro for standalone UEFI application test binaries
- libtest-compatible argument parsing from the kernel command line (`--exact`, `--list`, `--quiet`, positional filter)
- Pluggable `TestLifecycle` trait for customizing setup/teardown around individual tests
- Serial port output (COM1 on x86_64) for human-readable test reporting
- Panic handler that identifies the failing test by name and exits QEMU with the failure code
