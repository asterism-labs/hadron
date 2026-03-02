//! Userspace test that always fails — verifies the failure path.
//!
//! Exercises the full failure path:
//! compile → CPIO → QEMU → `--utest` mode → QEMU exit 35.

#![no_std]
#![no_main]

use hadron_utest::utest_main;

utest_main!(it_fails);

/// Always fails via a panicking assertion.
fn it_fails() {
    assert_eq!(1 + 1, 3, "intentional failure");
}
