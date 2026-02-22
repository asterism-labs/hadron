//! Kernel test runner for staged boot testing.
//!
//! When compiled with `--cfg ktest`, this module drives test execution at
//! checkpoint stages during [`kernel_init`](crate::boot::kernel_init).
//! Tests are collected from the `.hadron_kernel_tests` linker section via
//! [`hadron_ktest::kernel_test_entries()`].

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicPtr, AtomicU32, Ordering};

use hadron_ktest::{
    AsyncBarrier, KernelTestDescriptor, TestContext, TestKind, TestStage, kernel_test_entries,
};

use crate::driver_api::hw::Watchdog;
use crate::sync::SpinLock;

/// Tracks the currently-running test name for the panic handler.
static CURRENT_TEST: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static CURRENT_TEST_LEN: AtomicU32 = AtomicU32::new(0);
/// Total number of failures across all stages.
static FAILURES: AtomicU32 = AtomicU32::new(0);

/// Cached watchdog device for arming/disarming around tests.
static WATCHDOG: SpinLock<Option<Arc<dyn Watchdog>>> = SpinLock::leveled("KTEST_WD", 4, None);

/// Default per-test timeout in seconds (if not specified by the test).
const DEFAULT_TIMEOUT_SECS: u32 = 10;

/// Initializes the ktest watchdog by fetching the first registered watchdog
/// from the device registry. No-op if no watchdog is available.
pub fn init_watchdog() {
    let wd = crate::drivers::device_registry::with_device_registry(|reg| reg.first_watchdog());
    if let Some(wd) = wd {
        *WATCHDOG.lock() = Some(wd);
        hadron_ktest::serial_println!("ktest: watchdog armed for hang detection");
    }
}

/// Arms the watchdog with the given timeout. No-op if no watchdog is available.
fn arm_watchdog(timeout_secs: u32) {
    let guard = WATCHDOG.lock();
    if let Some(ref wd) = *guard {
        wd.arm(timeout_secs);
    }
}

/// Disarms the watchdog. No-op if no watchdog is available.
fn disarm_watchdog() {
    let guard = WATCHDOG.lock();
    if let Some(ref wd) = *guard {
        wd.disarm();
    }
}

/// Sets the currently-running test name (for panic reporting).
fn set_current_test(name: &'static str) {
    CURRENT_TEST.store(name.as_ptr() as *mut u8, Ordering::Release);
    CURRENT_TEST_LEN.store(name.len() as u32, Ordering::Release);
}

/// Clears the currently-running test name.
fn clear_current_test() {
    CURRENT_TEST.store(core::ptr::null_mut(), Ordering::Release);
    CURRENT_TEST_LEN.store(0, Ordering::Release);
}

/// Returns the currently-running test name, if any.
fn current_test_name() -> Option<&'static str> {
    let ptr = CURRENT_TEST.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    let len = CURRENT_TEST_LEN.load(Ordering::Acquire) as usize;
    // SAFETY: The pointer comes from a &'static str via set_current_test.
    Some(unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) })
}

/// Panic handler for ktest mode.
///
/// Prints the failing test name and panic info to serial, then exits QEMU
/// with a failure code. This function diverges.
pub fn on_panic(info: &core::panic::PanicInfo) -> ! {
    if let Some(name) = current_test_name() {
        hadron_ktest::serial_println!("FAILED");
        hadron_ktest::serial_println!("  test '{}' panicked: {}", name, info);
    } else {
        hadron_ktest::serial_println!("\n!!! KERNEL PANIC (ktest) !!!");
        hadron_ktest::serial_println!("{}", info);
    }
    hadron_ktest::qemu::exit_qemu(hadron_ktest::qemu::FAILURE);
}

/// Runs all synchronous tests for a given stage.
///
/// Called from `kernel_init()` at the appropriate checkpoint.
/// Returns the number of tests that ran.
pub fn run_sync_stage(stage: TestStage) -> usize {
    let entries = kernel_test_entries();
    let count = entries.iter().filter(|t| t.stage == stage).count();

    hadron_ktest::serial_println!("\nrunning {} {} test(s)", count, stage.as_str());

    let mut passed = 0;
    for test in entries.iter().filter(|t| t.stage == stage) {
        run_sync_test(test);
        passed += 1;
    }

    hadron_ktest::serial_println!(
        "test result: ok. {} passed; 0 failed ({} stage)\n",
        passed,
        stage.as_str()
    );
    passed
}

/// Runs a single synchronous test.
fn run_sync_test(test: &KernelTestDescriptor) {
    hadron_ktest::serial_print!("test {}::{} ... ", test.module_path, test.name);
    set_current_test(test.name);

    let timeout = if test.timeout_secs > 0 {
        test.timeout_secs
    } else {
        DEFAULT_TIMEOUT_SECS
    };
    arm_watchdog(timeout);

    // SAFETY: The proc macro guarantees this is a `fn()` for Sync tests.
    let f: fn() = unsafe { core::mem::transmute(test.test_fn) };
    f();

    disarm_watchdog();
    clear_current_test();
    hadron_ktest::serial_println!("ok");
}

/// Runs all async tests (with_executor and userspace stages).
///
/// This is an async function that should be spawned on the executor.
/// After all tests complete, it exits QEMU with the appropriate code.
pub async fn run_async_stages() {
    run_async_stage(TestStage::WithExecutor).await;
    run_async_stage(TestStage::Userspace).await;

    // All stages done — report final result and exit.
    let total_failures = FAILURES.load(Ordering::Acquire);
    if total_failures > 0 {
        hadron_ktest::serial_println!("\nktest: {} total failure(s)", total_failures);
        hadron_ktest::qemu::exit_qemu(hadron_ktest::qemu::FAILURE);
    } else {
        hadron_ktest::serial_println!("\nktest: all tests passed");
        hadron_ktest::qemu::exit_qemu(hadron_ktest::qemu::SUCCESS);
    }
}

/// Runs all async tests for a given stage.
async fn run_async_stage(stage: TestStage) {
    let entries = kernel_test_entries();
    let count = entries.iter().filter(|t| t.stage == stage).count();

    if count == 0 {
        return;
    }

    hadron_ktest::serial_println!("\nrunning {} {} test(s)", count, stage.as_str());

    let mut passed = 0;
    for test in entries.iter().filter(|t| t.stage == stage) {
        match test.kind {
            TestKind::Async => {
                run_single_async_test(test).await;
                passed += 1;
            }
            TestKind::AsyncInstanced => {
                run_instanced_async_test(test).await;
                passed += 1;
            }
            TestKind::Sync => {
                // Sync tests shouldn't appear in async stages, but handle gracefully.
                run_sync_test(test);
                passed += 1;
            }
        }
    }

    hadron_ktest::serial_println!(
        "test result: ok. {} passed; 0 failed ({} stage)\n",
        passed,
        stage.as_str()
    );
}

/// Runs a single async test (no instances).
async fn run_single_async_test(test: &KernelTestDescriptor) {
    hadron_ktest::serial_print!("test {}::{} ... ", test.module_path, test.name);
    set_current_test(test.name);

    let timeout = if test.timeout_secs > 0 {
        test.timeout_secs
    } else {
        DEFAULT_TIMEOUT_SECS
    };
    arm_watchdog(timeout);

    // SAFETY: The proc macro guarantees this is a
    // `fn() -> Pin<Box<dyn Future<Output = ()> + Send>>` for Async tests.
    let f: fn() -> Pin<Box<dyn Future<Output = ()> + Send>> =
        unsafe { core::mem::transmute(test.test_fn) };
    f().await;

    disarm_watchdog();
    clear_current_test();
    hadron_ktest::serial_println!("ok");
}

/// Runs an instanced async test by spawning concurrent tasks.
async fn run_instanced_async_test(test: &KernelTestDescriptor) {
    let instance_count = test.instance_end_inclusive - test.instance_start + 1;
    hadron_ktest::serial_print!(
        "test {}::{} ({} instances) ... ",
        test.module_path,
        test.name,
        instance_count
    );
    set_current_test(test.name);

    let timeout = if test.timeout_secs > 0 {
        test.timeout_secs
    } else {
        DEFAULT_TIMEOUT_SECS
    };
    arm_watchdog(timeout);

    let barrier = alloc::sync::Arc::new(AsyncBarrier::new(instance_count));

    // SAFETY: The proc macro guarantees this is a
    // `fn(&'static TestContext) -> Pin<Box<dyn Future<Output = ()> + Send>>`.
    let f: fn(&'static TestContext) -> Pin<Box<dyn Future<Output = ()> + Send>> =
        unsafe { core::mem::transmute(test.test_fn) };

    // Spawn each instance as a separate task on the executor.
    let mut task_ids = alloc::vec::Vec::with_capacity(instance_count as usize);
    for id in test.instance_start..=test.instance_end_inclusive {
        // Leak the context to get a 'static lifetime. Acceptable memory cost
        // for a test binary that exits after completion.
        let ctx = Box::leak(Box::new(TestContext::new(
            id,
            instance_count,
            barrier.clone(),
        )));
        let fut = f(ctx);
        task_ids.push(crate::sched::spawn(fut));
    }

    // Wait for all instances to complete.
    // We yield in a loop, checking if all spawned tasks have finished.
    // TODO: Replace with proper JoinHandle when the executor supports it.
    for _ in 0..task_ids.len() {
        crate::sched::primitives::yield_now().await;
    }
    // Give instances time to complete by yielding several times.
    for _ in 0..1000 {
        crate::sched::primitives::yield_now().await;
        // Check if barrier has been fully released (all instances done).
        // For now we just yield enough times for cooperative tasks to complete.
    }

    disarm_watchdog();
    clear_current_test();
    hadron_ktest::serial_println!("ok");
}
