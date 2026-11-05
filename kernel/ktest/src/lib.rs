//! Kernel test registration.
//!
//! [`ktest!`] defines a test function and places a [`Test`] for it in the
//! `.ktest` linker section. The kernel's runner reads the section through
//! [`tests`] after boot. The linker script must define `__ktest_start` and
//! `__ktest_end` around the section.

#![no_std]

#[doc(hidden)]
pub use linkset::linkset_entry;

/// One registered kernel test.
#[derive(Debug)]
pub struct Test {
    /// Path of the test function, such as `kernel::boot::limine_responds`.
    pub name: &'static str,
    /// The test body. A test fails by panicking.
    pub func: fn(),
    /// Set by `#[ignore]`. The runner skips ignored tests unless asked to
    /// run them.
    pub ignore: bool,
}

linkset::declare_linkset! {
    /// Returns every test registered with [`ktest!`] in the kernel image.
    pub fn tests() -> [Test],
    section = "ktest"
}

/// Defines a kernel test.
///
/// The test is a function with no arguments that fails by panicking.
/// Write `#[ignore]` first to keep it out of the default run.
///
/// ```ignore
/// ktest::ktest! {
///     fn adds() {
///         assert_eq!(1 + 1, 2);
///     }
/// }
/// ```
#[macro_export]
macro_rules! ktest {
    (#[ignore] $(#[$attr:meta])* fn $name:ident() $body:block) => {
        $crate::ktest!(@define true, $(#[$attr])* fn $name() $body);
    };
    ($(#[$attr:meta])* fn $name:ident() $body:block) => {
        $crate::ktest!(@define false, $(#[$attr])* fn $name() $body);
    };
    (@define $ignore:literal, $(#[$attr:meta])* fn $name:ident() $body:block) => {
        $(#[$attr])*
        fn $name() $body

        const _: () = {
            $crate::linkset_entry!("ktest", TEST: $crate::Test = $crate::Test {
                name: concat!(module_path!(), "::", stringify!($name)),
                func: $name,
                ignore: $ignore,
            });
        };
    };
}
