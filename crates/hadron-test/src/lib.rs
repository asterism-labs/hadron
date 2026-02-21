//! Test harness for Hadron OS kernel integration tests.
//!
//! Provides serial output, QEMU exit, a custom test runner with argument
//! parsing, lifecycle hooks, and entry-point macros for both Limine-booted
//! and UEFI test binaries.
//!
//! # Features
//!
//! - `limine` (default) — enables [`test_entry_point!`] and [`test_entry_point_with_init!`] for Limine-booted tests
//! - `uefi` — enables [`uefi_test_entry_point!`] for standalone UEFI app tests
//!
//! # Architecture
//!
//! Test results are communicated purely via QEMU exit codes (33=success,
//! 35=failure). The [`TestHarness`] parses command-line arguments for
//! filtering and listing, runs tests through [`TestLifecycle`] hooks,
//! and exits QEMU with the appropriate code.

#![no_std]
#![warn(missing_docs)]

use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// Command-line argument parser for test binaries.
pub mod args;
/// QEMU exit device interface.
pub mod qemu;
/// Serial port I/O for test output.
pub mod serial;

pub use args::TestArgs;
pub use qemu::ExitCode;

// ---------------------------------------------------------------------------
// Current test name tracking (for panic handler)
// ---------------------------------------------------------------------------

static CURRENT_TEST: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static CURRENT_TEST_LEN: AtomicUsize = AtomicUsize::new(0);

fn set_current_test(name: &str) {
    CURRENT_TEST.store(name.as_ptr() as *mut u8, Ordering::Release);
    CURRENT_TEST_LEN.store(name.len(), Ordering::Release);
}

fn current_test_name() -> &'static str {
    let len = CURRENT_TEST_LEN.load(Ordering::Acquire);
    if len == 0 {
        return "<unknown>";
    }
    let ptr = CURRENT_TEST.load(Ordering::Acquire);
    // SAFETY: test names come from `type_name()` which returns `&'static str`.
    // Single-threaded execution in test runner, pointer is only read in the
    // panic handler after being set in `run()`.
    unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) }
}

// ---------------------------------------------------------------------------
// Command-line storage (set by entry point macros, read by test_runner)
// ---------------------------------------------------------------------------

static CMDLINE_PTR: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static CMDLINE_LEN: AtomicUsize = AtomicUsize::new(0);

/// Store the kernel command line for the test harness.
///
/// Called by entry point macros after reading the Limine response.
/// The string must live for the duration of the program (typically
/// backed by bootloader memory).
pub fn set_command_line(cmdline: &str) {
    CMDLINE_PTR.store(cmdline.as_ptr() as *mut u8, Ordering::Release);
    CMDLINE_LEN.store(cmdline.len(), Ordering::Release);
}

/// Retrieve the stored command line, if any.
pub fn command_line() -> Option<&'static str> {
    let len = CMDLINE_LEN.load(Ordering::Acquire);
    if len == 0 {
        return None;
    }
    let ptr = CMDLINE_PTR.load(Ordering::Acquire);
    if ptr.is_null() {
        return None;
    }
    // SAFETY: the pointer comes from a `&str` stored by `set_command_line`,
    // which is backed by bootloader memory that persists for the kernel's lifetime.
    Some(unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) })
}

// ---------------------------------------------------------------------------
// TestLifecycle trait
// ---------------------------------------------------------------------------

/// Trait for customizing test execution lifecycle.
///
/// Implement this to add custom setup/teardown around tests.
/// Used by both integration test macros (with [`DefaultLifecycle`]) and
/// kernel test mode (with a custom lifecycle).
pub trait TestLifecycle {
    /// Called once before any tests run.
    fn before_all(&self, _count: usize) {}
    /// Called before each test.
    fn before_each(&self, _name: &str) {}
    /// Called after each test passes.
    fn after_each_pass(&self, _name: &str) {}
    /// Called after each test fails (from panic handler, before QEMU exit).
    fn after_each_fail(&self, _name: &str) {}
    /// Called once after all tests complete successfully.
    fn after_all(&self, _passed: usize, _failed: usize) {}
}

/// Default lifecycle that prints results to serial in a format
/// similar to Rust's built-in test output.
pub struct DefaultLifecycle;

impl TestLifecycle for DefaultLifecycle {
    fn before_all(&self, count: usize) {
        serial_println!("running {} tests", count);
    }

    fn before_each(&self, name: &str) {
        serial_print!("test {} ... ", name);
    }

    fn after_each_pass(&self, _name: &str) {
        serial_println!("ok");
    }

    fn after_all(&self, passed: usize, failed: usize) {
        if failed == 0 {
            serial_println!("\ntest result: ok. {} passed; 0 failed", passed);
        } else {
            serial_println!(
                "\ntest result: FAILED. {} passed; {} failed",
                passed,
                failed
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Testable trait
// ---------------------------------------------------------------------------

/// A test that can be named and run.
pub trait Testable {
    /// Run the test function.
    fn run(&self);
    /// Return the fully-qualified test name.
    fn name(&self) -> &'static str;
}

impl<T: Fn()> Testable for T {
    fn run(&self) {
        set_current_test(self.name());
        self();
        set_current_test("");
    }

    fn name(&self) -> &'static str {
        core::any::type_name::<T>()
    }
}

// ---------------------------------------------------------------------------
// TestHarness
// ---------------------------------------------------------------------------

/// Test harness that runs tests with filtering, listing, and lifecycle hooks.
///
/// The harness parses command-line arguments, filters tests, and runs them
/// through the provided [`TestLifecycle`]. On completion (or first failure),
/// it exits QEMU with the appropriate exit code.
pub struct TestHarness<'a, L: TestLifecycle = DefaultLifecycle> {
    args: TestArgs<'a>,
    lifecycle: L,
}

impl<'a, L: TestLifecycle> TestHarness<'a, L> {
    /// Create a new test harness with the given arguments and lifecycle.
    pub fn new(args: TestArgs<'a>, lifecycle: L) -> Self {
        Self { args, lifecycle }
    }

    /// Run all tests, applying filters and lifecycle hooks.
    ///
    /// This function never returns — it exits QEMU with [`ExitCode::Success`]
    /// if all tests pass, or the panic handler exits with [`ExitCode::Failure`]
    /// on the first test failure.
    pub fn run(&self, tests: &[&dyn Testable]) -> ! {
        // --list mode: print test names and exit
        if self.args.list {
            for test in tests {
                if self.args.matches(test.name()) {
                    serial_println!("{}: test", test.name());
                }
            }
            qemu::exit(ExitCode::Success);
        }

        // Count matching tests
        let total = tests.iter().filter(|t| self.args.matches(t.name())).count();

        if !self.args.quiet {
            self.lifecycle.before_all(total);
        }

        let mut passed = 0usize;
        for test in tests {
            if !self.args.matches(test.name()) {
                continue;
            }

            if !self.args.quiet {
                self.lifecycle.before_each(test.name());
            }

            test.run(); // Panics on failure → panic handler exits QEMU

            if !self.args.quiet {
                self.lifecycle.after_each_pass(test.name());
            }
            passed += 1;
        }

        // All tests passed
        self.lifecycle.after_all(passed, 0);
        qemu::exit(ExitCode::Success);
    }
}

// ---------------------------------------------------------------------------
// test_runner (entry point for custom_test_frameworks)
// ---------------------------------------------------------------------------

/// Custom test runner. Pass to `#![test_runner(hadron_test::test_runner)]`.
///
/// Reads the command line stored by the entry point macro, parses test
/// arguments, and runs tests through [`TestHarness`] with [`DefaultLifecycle`].
pub fn test_runner(tests: &[&dyn Testable]) {
    let cmdline = command_line();
    let args = TestArgs::parse(cmdline);
    let harness = TestHarness::new(args, DefaultLifecycle);
    harness.run(tests);
}

/// Handle a panic in a test binary: print failure info and exit QEMU.
pub fn test_panic_handler(info: &core::panic::PanicInfo) -> ! {
    serial_println!("FAILED");
    serial_println!();
    serial_println!("---- {} ----", current_test_name());
    serial_println!("{}", info);
    serial_println!();
    qemu::exit(ExitCode::Failure);
}

// ---------------------------------------------------------------------------
// Entry point macros
// ---------------------------------------------------------------------------

/// Generate the Limine entry point, request markers, and panic handler
/// for an integration test binary.
///
/// Each integration test file should invoke this once at crate root:
///
/// ```ignore
/// #![no_std]
/// #![no_main]
/// #![feature(custom_test_frameworks)]
/// #![test_runner(hadron_test::test_runner)]
/// #![reexport_test_harness_main = "test_main"]
///
/// hadron_test::test_entry_point!();
///
/// #[test_case]
/// fn it_works() { assert_eq!(2 + 2, 4); }
/// ```
#[cfg(feature = "limine")]
#[macro_export]
macro_rules! test_entry_point {
    () => {
        #[used]
        #[unsafe(link_section = ".requests_start")]
        static _REQUESTS_START_MARKER: ::limine::RequestsStartMarker =
            ::limine::RequestsStartMarker::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _BASE_REVISION: ::limine::BaseRevision = ::limine::BaseRevision::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _CMDLINE_REQUEST: ::limine::ExecutableCmdlineRequest =
            ::limine::ExecutableCmdlineRequest::new();

        #[used]
        #[unsafe(link_section = ".requests_end")]
        static _REQUESTS_END_MARKER: ::limine::RequestsEndMarker =
            ::limine::RequestsEndMarker::new();

        #[unsafe(no_mangle)]
        extern "C" fn _start() -> ! {
            $crate::serial::init();
            if let Some(resp) = _CMDLINE_REQUEST.response() {
                $crate::set_command_line(resp.cmdline());
            }
            test_main();
            $crate::qemu::exit($crate::ExitCode::Success);
        }

        #[panic_handler]
        fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::test_panic_handler(info)
        }
    };
}

/// Generate the Limine entry point with full kernel initialization for
/// integration tests that need PMM, VMM, and the heap allocator.
///
/// This sets up Limine requests for HHDM, memory map, executable address,
/// and paging mode, then calls [`hadron_kernel::test_init`] before running
/// the test harness.
///
/// ```ignore
/// #![no_std]
/// #![no_main]
/// #![feature(custom_test_frameworks)]
/// #![test_runner(hadron_test::test_runner)]
/// #![reexport_test_harness_main = "test_main"]
///
/// extern crate alloc;
///
/// hadron_test::test_entry_point_with_init!();
///
/// #[test_case]
/// fn heap_works() {
///     let v = alloc::vec![1, 2, 3];
///     assert_eq!(v.len(), 3);
/// }
/// ```
#[cfg(feature = "limine")]
#[macro_export]
macro_rules! test_entry_point_with_init {
    () => {
        #[used]
        #[unsafe(link_section = ".requests_start")]
        static _REQUESTS_START_MARKER: ::limine::RequestsStartMarker =
            ::limine::RequestsStartMarker::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _BASE_REVISION: ::limine::BaseRevision = ::limine::BaseRevision::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _HHDM_REQUEST: ::limine::HhdmRequest = ::limine::HhdmRequest::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _MEMMAP_REQUEST: ::limine::MemMapRequest = ::limine::MemMapRequest::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _EXEC_ADDR_REQUEST: ::limine::ExecutableAddressRequest =
            ::limine::ExecutableAddressRequest::new();

        #[used]
        #[unsafe(link_section = ".requests")]
        static _PAGING_MODE_REQUEST: ::limine::PagingModeRequest =
            ::limine::PagingModeRequest::new(
                ::limine::paging::PagingMode::Paging4Level,
                ::limine::paging::PagingMode::Paging4Level,
                ::limine::paging::PagingMode::Paging5Level,
            );

        #[used]
        #[unsafe(link_section = ".requests")]
        static _CMDLINE_REQUEST: ::limine::ExecutableCmdlineRequest =
            ::limine::ExecutableCmdlineRequest::new();

        #[used]
        #[unsafe(link_section = ".requests_end")]
        static _REQUESTS_END_MARKER: ::limine::RequestsEndMarker =
            ::limine::RequestsEndMarker::new();

        #[unsafe(no_mangle)]
        extern "C" fn _start() -> ! {
            $crate::serial::init();

            // Store command line for test harness args parsing.
            if let Some(resp) = _CMDLINE_REQUEST.response() {
                $crate::set_command_line(resp.cmdline());
            }

            // Read Limine responses.
            let hhdm_offset = _HHDM_REQUEST
                .response()
                .expect("HHDM response not available")
                .hhdm_base;

            let memmap = _MEMMAP_REQUEST
                .response()
                .expect("Memory map response not available");

            let exec_addr = _EXEC_ADDR_REQUEST
                .response()
                .expect("Executable address response not available");

            // Read current page table root.
            let page_table_root: u64;
            #[cfg(target_arch = "x86_64")]
            {
                unsafe {
                    core::arch::asm!(
                        "mov {}, cr3",
                        out(reg) page_table_root,
                        options(nomem, preserves_flags)
                    );
                }
            }
            #[cfg(target_arch = "aarch64")]
            {
                unsafe {
                    core::arch::asm!(
                        "mrs {}, TTBR1_EL1",
                        out(reg) page_table_root,
                        options(nomem, preserves_flags)
                    );
                }
            }

            // Build memory map.
            let mut memory_map = ::noalloc::vec::ArrayVec::new();
            for entry in memmap.entries() {
                use ::limine::memmap::MemMapEntryType;
                let kind = match entry.type_ {
                    MemMapEntryType::Usable => {
                        ::hadron_kernel::boot::MemoryRegionKind::Usable
                    }
                    MemMapEntryType::Reserved => {
                        ::hadron_kernel::boot::MemoryRegionKind::Reserved
                    }
                    MemMapEntryType::AcpiReclaimable | MemMapEntryType::AcpiTables => {
                        ::hadron_kernel::boot::MemoryRegionKind::AcpiReclaimable
                    }
                    MemMapEntryType::AcpiNvs => {
                        ::hadron_kernel::boot::MemoryRegionKind::AcpiNvs
                    }
                    MemMapEntryType::BadMemory => {
                        ::hadron_kernel::boot::MemoryRegionKind::BadMemory
                    }
                    MemMapEntryType::BootloaderReclaimable => {
                        ::hadron_kernel::boot::MemoryRegionKind::BootloaderReclaimable
                    }
                    MemMapEntryType::KernelAndModules => {
                        ::hadron_kernel::boot::MemoryRegionKind::KernelAndModules
                    }
                    MemMapEntryType::Framebuffer => {
                        ::hadron_kernel::boot::MemoryRegionKind::Framebuffer
                    }
                };
                memory_map.push(::hadron_kernel::boot::MemoryRegion {
                    start: ::hadron_kernel::addr::PhysAddr::new(entry.base),
                    size: entry.length,
                    kind,
                });
            }

            // Build BootInfoData.
            let boot_info = ::hadron_kernel::boot::BootInfoData {
                memory_map,
                hhdm_offset,
                kernel_address: ::hadron_kernel::boot::KernelAddressInfo {
                    physical_base: ::hadron_kernel::addr::PhysAddr::new(exec_addr.phys_base),
                    virtual_base: ::hadron_kernel::addr::VirtAddr::new(exec_addr.virt_base),
                },
                paging_mode: ::hadron_kernel::boot::PagingMode::Level4,
                framebuffers: ::noalloc::vec::ArrayVec::new(),
                rsdp_address: None,
                dtb_address: None,
                command_line: _CMDLINE_REQUEST.response().map(|r| r.cmdline()),
                smbios_32: None,
                smbios_64: None,
                page_table_root: ::hadron_kernel::addr::PhysAddr::new(
                    page_table_root & 0x000F_FFFF_FFFF_F000,
                ),
                initrd: None,
                smp_cpus: ::noalloc::vec::ArrayVec::new(),
                bsp_lapic_id: 0,
            };

            ::hadron_kernel::test_init(&boot_info);
            test_main();
            $crate::qemu::exit($crate::ExitCode::Success);
        }

        #[panic_handler]
        fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::test_panic_handler(info)
        }
    };
}

/// Generate the UEFI entry point, system table storage, and panic handler
/// for a standalone UEFI test binary.
///
/// This macro produces:
/// - A thread-safe `SYSTEM_TABLE` static (`AtomicPtr<SystemTable>`)
/// - A thread-safe `IMAGE_HANDLE` static (`AtomicPtr<c_void>`)
/// - An `unsafe fn system_table() -> &'static SystemTable` raw accessor
/// - An `unsafe fn image_handle() -> EfiHandle` accessor
/// - The `efi_main` UEFI entry point that stores the system table and handle,
///   inits serial, runs the test harness, and exits QEMU
/// - A panic handler
///
/// Each UEFI integration test file should invoke this once at crate root:
///
/// ```ignore
/// #![no_std]
/// #![no_main]
/// #![feature(custom_test_frameworks)]
/// #![test_runner(hadron_test::test_runner)]
/// #![reexport_test_harness_main = "test_main"]
///
/// hadron_test::uefi_test_entry_point!();
///
/// #[test_case]
/// fn trivial_assertion() { assert_eq!(1, 1); }
/// ```
#[cfg(feature = "uefi")]
#[macro_export]
macro_rules! uefi_test_entry_point {
    () => {
        use core::sync::atomic::{AtomicPtr, Ordering};

        static SYSTEM_TABLE: AtomicPtr<::uefi::table::SystemTable> =
            AtomicPtr::new(core::ptr::null_mut());

        static IMAGE_HANDLE: AtomicPtr<core::ffi::c_void> = AtomicPtr::new(core::ptr::null_mut());

        /// Retrieve the raw UEFI System Table stored at entry.
        ///
        /// # Safety
        /// Must only be called after `efi_main` has stored the pointer.
        unsafe fn system_table() -> &'static ::uefi::table::SystemTable {
            let ptr = SYSTEM_TABLE.load(Ordering::Acquire);
            unsafe { &*ptr }
        }

        /// Retrieve the EFI image handle stored at entry.
        ///
        /// # Safety
        /// Must only be called after `efi_main` has stored the handle.
        unsafe fn image_handle() -> ::uefi::EfiHandle {
            IMAGE_HANDLE.load(Ordering::Acquire)
        }

        #[unsafe(no_mangle)]
        extern "efiapi" fn efi_main(
            handle: ::uefi::EfiHandle,
            system_table: *mut ::uefi::table::SystemTable,
        ) -> ::uefi::EfiStatus {
            IMAGE_HANDLE.store(handle, Ordering::Release);
            SYSTEM_TABLE.store(system_table, Ordering::Release);
            $crate::serial::init();
            test_main();
            $crate::qemu::exit($crate::ExitCode::Success);
        }

        #[panic_handler]
        fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::test_panic_handler(info)
        }
    };
}
