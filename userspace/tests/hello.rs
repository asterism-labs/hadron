//! Trivial userspace test — verifies that the utest pipeline boots and exits.
//!
//! This test always passes. It exercises the full path:
//! compile → CPIO → QEMU → `--utest` mode → QEMU exit 33.

#![no_std]
#![no_main]

use hadron_utest::utest_main;

utest_main!(it_boots);

/// Trivially passes — just verifies the test binary starts and returns.
fn it_boots() {
    // No assertion needed; returning from this function means success.
}
