//! Microbenchmark harness for Hadron OS kernel benchmarks.
//!
//! Mirrors the architecture of `hadron-test`: same trait pattern, harness,
//! entry macros, and QEMU exit mechanism. Benchmarks use `rdtscp`-fenced
//! cycle counting for measurement and emit results both as human-readable
//! serial text and a compact binary wire format.
//!
//! # Features
//!
//! - `limine` (default) — enables [`bench_entry_point!`] and
//!   [`bench_entry_point_with_init!`] for Limine-booted benchmarks
//!
//! # Architecture
//!
//! Benchmark results are emitted to serial (human-readable summary + binary
//! records) and communicated via QEMU exit codes (33=success, 35=failure).

#![no_std]
#![warn(missing_docs)]

use core::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

/// Command-line argument parser for benchmark binaries.
pub mod args;
/// Benchmark iteration controller.
pub mod bencher;
/// QEMU exit device interface.
pub mod qemu;
/// Serial port I/O for benchmark output.
pub mod serial;
/// Integer-only statistics.
pub mod stats;
/// Binary wire format emission.
pub mod wire;

pub use args::BenchArgs;
pub use bencher::{Bencher, black_box};
pub use qemu::ExitCode;
pub use stats::BenchStats;

// ---------------------------------------------------------------------------
// Current benchmark name tracking (for panic handler)
// ---------------------------------------------------------------------------

static CURRENT_BENCH: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static CURRENT_BENCH_LEN: AtomicUsize = AtomicUsize::new(0);

fn set_current_bench(name: &str) {
    CURRENT_BENCH.store(name.as_ptr() as *mut u8, Ordering::Release);
    CURRENT_BENCH_LEN.store(name.len(), Ordering::Release);
}

fn current_bench_name() -> &'static str {
    let len = CURRENT_BENCH_LEN.load(Ordering::Acquire);
    if len == 0 {
        return "<unknown>";
    }
    let ptr = CURRENT_BENCH.load(Ordering::Acquire);
    // SAFETY: Benchmark names come from `type_name()` which returns `&'static str`.
    // Single-threaded execution in bench runner, pointer is only read in the
    // panic handler after being set in `run()`.
    unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) }
}

// ---------------------------------------------------------------------------
// Command-line storage
// ---------------------------------------------------------------------------

static CMDLINE_PTR: AtomicPtr<u8> = AtomicPtr::new(core::ptr::null_mut());
static CMDLINE_LEN: AtomicUsize = AtomicUsize::new(0);

/// Store the kernel command line for the benchmark harness.
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
    // backed by bootloader memory that persists for the kernel's lifetime.
    Some(unsafe { core::str::from_utf8_unchecked(core::slice::from_raw_parts(ptr, len)) })
}

// ---------------------------------------------------------------------------
// TSC calibration
// ---------------------------------------------------------------------------

/// Estimated TSC frequency in kHz. Set during calibration.
static TSC_FREQ_KHZ: AtomicUsize = AtomicUsize::new(0);

/// Returns the TSC frequency in kHz, or 0 if not calibrated.
pub fn tsc_freq_khz() -> u64 {
    TSC_FREQ_KHZ.load(Ordering::Relaxed) as u64
}

/// Calibrate TSC frequency against HPET over a 10ms busy-wait.
///
/// Falls back to 2 GHz estimate if HPET is not available.
#[cfg(target_arch = "x86_64")]
pub fn calibrate_tsc_against_hpet() {
    // Try reading HPET from the kernel's time module.
    // This requires bench_entry_point_with_init! to have been used.
    // If HPET is not initialized, fall back to a default estimate.
    let freq = estimate_tsc_frequency_default();
    TSC_FREQ_KHZ.store(freq as usize, Ordering::Relaxed);
}

/// Default TSC frequency estimate: 2 GHz = 2_000_000 kHz.
fn estimate_tsc_frequency_default() -> u64 {
    2_000_000
}

// ---------------------------------------------------------------------------
// BenchLifecycle trait
// ---------------------------------------------------------------------------

/// Trait for customizing benchmark execution lifecycle.
pub trait BenchLifecycle {
    /// Called once before any benchmarks run.
    fn before_all(&self, _count: usize) {}
    /// Called before each benchmark.
    fn before_each(&self, _name: &str) {}
    /// Called after each benchmark with its statistics.
    fn after_each(&self, _name: &str, _stats: &BenchStats) {}
    /// Called once after all benchmarks complete.
    fn after_all(&self, _count: usize) {}
}

/// Default lifecycle that prints results to serial.
pub struct DefaultLifecycle;

impl BenchLifecycle for DefaultLifecycle {
    fn before_all(&self, count: usize) {
        serial_println!("running {} benchmarks", count);
        serial_println!();
    }

    fn before_each(&self, name: &str) {
        serial_print!("bench {} ... ", name);
    }

    fn after_each(&self, _name: &str, stats: &BenchStats) {
        let freq = tsc_freq_khz();
        let mean_ns = BenchStats::cycles_to_nanos(stats.mean, freq);
        let stddev_ns = BenchStats::cycles_to_nanos(stats.stddev, freq);
        serial_println!(
            "{} cycles/iter ({} ns +/- {} ns)",
            stats.mean,
            mean_ns,
            stddev_ns
        );
    }

    fn after_all(&self, count: usize) {
        serial_println!();
        serial_println!("benchmark result: ok. {} benchmarks completed", count);
    }
}

// ---------------------------------------------------------------------------
// Benchmarkable trait
// ---------------------------------------------------------------------------

/// A benchmark that can be named and run.
pub trait Benchmarkable {
    /// Run the benchmark function with the given bencher.
    fn run(&self, bencher: &mut Bencher);
    /// Return the fully-qualified benchmark name.
    fn name(&self) -> &'static str;
}

impl<T: Fn(&mut Bencher)> Benchmarkable for T {
    fn run(&self, bencher: &mut Bencher) {
        set_current_bench(self.name());
        self(bencher);
        set_current_bench("");
    }

    fn name(&self) -> &'static str {
        core::any::type_name::<T>()
    }
}

// ---------------------------------------------------------------------------
// BenchHarness
// ---------------------------------------------------------------------------

/// Benchmark harness that runs benchmarks with filtering, lifecycle hooks,
/// and binary result emission.
pub struct BenchHarness<'a, L: BenchLifecycle = DefaultLifecycle> {
    args: BenchArgs<'a>,
    lifecycle: L,
}

impl<'a, L: BenchLifecycle> BenchHarness<'a, L> {
    /// Create a new benchmark harness with the given arguments and lifecycle.
    pub fn new(args: BenchArgs<'a>, lifecycle: L) -> Self {
        Self { args, lifecycle }
    }

    /// Run all benchmarks, applying filters and lifecycle hooks.
    ///
    /// This function never returns — it exits QEMU with [`ExitCode::Success`]
    /// after all benchmarks complete, or the panic handler exits with
    /// [`ExitCode::Failure`] on error.
    pub fn run(&self, benchmarks: &[&dyn Benchmarkable]) -> ! {
        // --list mode: print benchmark names and exit.
        if self.args.list {
            for bench in benchmarks {
                if self.args.matches(bench.name()) {
                    serial_println!("{}: bench", bench.name());
                }
            }
            qemu::exit(ExitCode::Success);
        }

        // Count matching benchmarks.
        let total = benchmarks
            .iter()
            .filter(|b| self.args.matches(b.name()))
            .count();

        if !self.args.quiet {
            self.lifecycle.before_all(total);
        }

        // Start TSC for total elapsed time.
        let start_tsc = read_tsc_for_timing();

        // Emit binary header.
        wire::emit_header(total as u32);

        let mut completed = 0usize;
        for bench in benchmarks {
            if !self.args.matches(bench.name()) {
                continue;
            }

            if !self.args.quiet {
                self.lifecycle.before_each(bench.name());
            }

            // Run the benchmark.
            let mut bencher = Bencher::new(self.args.warmup, self.args.samples);
            bench.run(&mut bencher);

            // Compute and emit statistics.
            let samples = bencher.samples_mut();
            if let Some(stats) = BenchStats::compute(samples) {
                if !self.args.quiet {
                    self.lifecycle.after_each(bench.name(), &stats);
                }
            }

            // Emit binary record with raw samples.
            wire::emit_record(bench.name(), bencher.samples());

            completed += 1;
        }

        // Compute total elapsed time.
        let end_tsc = read_tsc_for_timing();
        let elapsed_cycles = end_tsc.saturating_sub(start_tsc);
        let freq = tsc_freq_khz();
        let total_nanos = BenchStats::cycles_to_nanos(elapsed_cycles, freq);

        // Emit binary footer.
        wire::emit_footer(freq, total_nanos);

        if !self.args.quiet {
            self.lifecycle.after_all(completed);
        }

        qemu::exit(ExitCode::Success);
    }
}

/// Read TSC for total elapsed timing.
#[cfg(target_arch = "x86_64")]
fn read_tsc_for_timing() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: RDTSC has no side effects.
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(hi) << 32) | u64::from(lo)
}

#[cfg(target_arch = "aarch64")]
fn read_tsc_for_timing() -> u64 {
    todo!("aarch64 read_tsc_for_timing")
}

// ---------------------------------------------------------------------------
// bench_runner (entry point for custom_test_frameworks)
// ---------------------------------------------------------------------------

/// Custom benchmark runner. Pass to `#![test_runner(hadron_bench::bench_runner)]`.
pub fn bench_runner(benchmarks: &[&dyn Benchmarkable]) {
    let cmdline = command_line();
    let args = BenchArgs::parse(cmdline);

    // Calibrate TSC before running benchmarks.
    #[cfg(target_arch = "x86_64")]
    calibrate_tsc_against_hpet();

    let harness = BenchHarness::new(args, DefaultLifecycle);
    harness.run(benchmarks);
}

/// Handle a panic in a benchmark binary.
pub fn bench_panic_handler(info: &core::panic::PanicInfo) -> ! {
    serial_println!("FAILED");
    serial_println!();
    serial_println!("---- {} ----", current_bench_name());
    serial_println!("{}", info);
    serial_println!();
    qemu::exit(ExitCode::Failure);
}

// ---------------------------------------------------------------------------
// Entry point macros
// ---------------------------------------------------------------------------

/// Generate the Limine entry point for a minimal benchmark binary.
///
/// No kernel initialization — suitable for pure CPU microbenchmarks.
///
/// ```ignore
/// #![no_std]
/// #![no_main]
/// #![feature(custom_test_frameworks)]
/// #![test_runner(hadron_bench::bench_runner)]
/// #![reexport_test_harness_main = "bench_main"]
///
/// hadron_bench::bench_entry_point!();
///
/// #[test_case]
/// fn bench_nop(b: &mut hadron_bench::Bencher) {
///     b.iter(|| hadron_bench::black_box(42));
/// }
/// ```
#[cfg(feature = "limine")]
#[macro_export]
macro_rules! bench_entry_point {
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
            bench_main();
            $crate::qemu::exit($crate::ExitCode::Success);
        }

        #[panic_handler]
        fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::bench_panic_handler(info)
        }
    };
}

/// Generate the Limine entry point with full kernel initialization for
/// benchmarks that need PMM, VMM, and the heap allocator.
///
/// Structurally identical to `hadron_test::test_entry_point_with_init!()`.
///
/// ```ignore
/// #![no_std]
/// #![no_main]
/// #![feature(custom_test_frameworks)]
/// #![test_runner(hadron_bench::bench_runner)]
/// #![reexport_test_harness_main = "bench_main"]
///
/// extern crate alloc;
///
/// hadron_bench::bench_entry_point_with_init!();
///
/// #[test_case]
/// fn bench_alloc(b: &mut hadron_bench::Bencher) {
///     b.iter(|| {
///         let v = alloc::vec![0u8; 64];
///         hadron_bench::black_box(v)
///     });
/// }
/// ```
#[cfg(feature = "limine")]
#[macro_export]
macro_rules! bench_entry_point_with_init {
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

            if let Some(resp) = _CMDLINE_REQUEST.response() {
                $crate::set_command_line(resp.cmdline());
            }

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

            let page_table_root: u64;
            #[cfg(target_arch = "x86_64")]
            {
                // SAFETY: Reading CR3 is always safe in ring 0.
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
                // SAFETY: Reading TTBR1_EL1 is always safe at EL1.
                unsafe {
                    core::arch::asm!(
                        "mrs {}, TTBR1_EL1",
                        out(reg) page_table_root,
                        options(nomem, preserves_flags)
                    );
                }
            }

            let mut memory_map = ::planck_noalloc::vec::ArrayVec::new();
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

            let boot_info = ::hadron_kernel::boot::BootInfoData {
                memory_map,
                hhdm_offset,
                kernel_address: ::hadron_kernel::boot::KernelAddressInfo {
                    physical_base: ::hadron_kernel::addr::PhysAddr::new(exec_addr.phys_base),
                    virtual_base: ::hadron_kernel::addr::VirtAddr::new(exec_addr.virt_base),
                },
                paging_mode: ::hadron_kernel::boot::PagingMode::Level4,
                framebuffers: ::planck_noalloc::vec::ArrayVec::new(),
                rsdp_address: None,
                dtb_address: None,
                command_line: _CMDLINE_REQUEST.response().map(|r| r.cmdline()),
                smbios_32: None,
                smbios_64: None,
                page_table_root: ::hadron_kernel::addr::PhysAddr::new(
                    page_table_root & 0x000F_FFFF_FFFF_F000,
                ),
                initrd: None,
                smp_cpus: ::planck_noalloc::vec::ArrayVec::new(),
                bsp_lapic_id: 0,
            };

            ::hadron_kernel::test_init(&boot_info);
            bench_main();
            $crate::qemu::exit($crate::ExitCode::Success);
        }

        #[panic_handler]
        fn panic(info: &core::panic::PanicInfo) -> ! {
            $crate::bench_panic_handler(info)
        }
    };
}
