//! Runner for the tests registered with [`ktest::ktest!`].
//!
//! Gluon's `test` profile sets the `KTEST` config option, and `kmain`
//! then calls [`run`] after boot. A failing test panics. The kernel has no
//! unwinding, so the panic handler reports the test through
//! [`report_panic`] and exits QEMU, and the run stops at that test.

use core::fmt::Write;
use core::sync::atomic::{AtomicBool, Ordering};

use crate::boot::{EXIT_SUCCESS, Serial, exit_qemu};

/// Set while a test runs, so the panic handler knows a test failed.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Runs every registered test, prints one line per test on COM1 and exits
/// QEMU with success. Ignored tests run only when the `KTEST_IGNORED`
/// config option is set.
pub(crate) fn run() -> ! {
    let tests = ktest::tests();
    let _ = writeln!(Serial, "running {} tests", tests.len());
    let (mut passed, mut ignored) = (0, 0);
    for test in tests {
        let _ = write!(Serial, "test {} ... ", test.name);
        if test.ignore && !hadron_config::KTEST_IGNORED {
            let _ = writeln!(Serial, "ignored");
            ignored += 1;
            continue;
        }
        RUNNING.store(true, Ordering::Relaxed);
        (test.func)();
        RUNNING.store(false, Ordering::Relaxed);
        let _ = writeln!(Serial, "ok");
        passed += 1;
    }
    let _ = writeln!(
        Serial,
        "test result: ok. {passed} passed; 0 failed; {ignored} ignored"
    );
    exit_qemu(EXIT_SUCCESS)
}

/// Called by the panic handler. If a test is running, ends its line with
/// `FAILED`. The panic handler then prints the panic message.
pub(crate) fn report_panic() {
    if RUNNING.load(Ordering::Relaxed) {
        let _ = writeln!(Serial, "FAILED");
    }
}

// Shows that a failing test fails the run. Run it with
// `GLUON_KTEST_IGNORED=true gluon run -p test`.
ktest::ktest! {
    #[ignore]
    fn fails_on_purpose() {
        assert_eq!(1 + 1, 3, "this test fails on purpose");
    }
}
