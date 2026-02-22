//! Sampling profiler: periodic timer-interrupt-driven stack capture.
//!
//! Hooks into the existing LAPIC timer interrupt (vector 254). When active,
//! captures the interrupted RIP + frame pointer stack walk into a per-CPU
//! ring buffer at a configurable sample rate (default 100 Hz).
//!
//! Start/stop is controlled by [`start`] and [`stop`]. Stopping drains
//! all per-CPU buffers to serial in the HPRF binary format.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::buffer::{MAX_SAMPLE_DEPTH, Sample, SampleBufCell, SampleRingBuf};
use super::format;
use crate::arch::x86_64::hw::tsc;
use crate::id::CpuId;
use crate::percpu::{CpuLocal, cpu_count, current_cpu};

/// Whether sampling is currently active.
static SAMPLING_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Software divider: sample every N-th timer tick (1000 Hz / N = sample rate).
static SAMPLE_DIVIDER: AtomicU32 = AtomicU32::new(10);

/// Per-CPU sample ring buffers.
///
/// Accessed only from the timer ISR on the owning CPU. Each CPU has its
/// own buffer indexed by CPU ID.
static SAMPLE_BUFFERS: CpuLocal<SampleBufCell> = {
    const CELL: SampleBufCell = SampleBufCell::new();
    CpuLocal::new([CELL; crate::config::MAX_CPUS])
};

/// Per-CPU tick counters for the software divider.
static TICK_COUNTERS: CpuLocal<core::cell::UnsafeCell<u32>> = {
    const CELL: core::cell::UnsafeCell<u32> = core::cell::UnsafeCell::new(0);
    CpuLocal::new([CELL; crate::config::MAX_CPUS])
};

// SAFETY: tick counters are only accessed from the timer ISR on the owning CPU.
unsafe impl Send for core::cell::UnsafeCell<u32> {}
unsafe impl Sync for core::cell::UnsafeCell<u32> {}

/// Start the sampling profiler.
///
/// `rate_hz` is the desired sample rate (1-1000 Hz). The timer fires at
/// 1000 Hz, so the actual rate is `1000 / divider` where divider = 1000 / rate_hz.
pub fn start(rate_hz: u32) {
    let rate = rate_hz.clamp(1, 1000);
    let divider = 1000 / rate;
    SAMPLE_DIVIDER.store(divider.max(1), Ordering::Relaxed);
    SAMPLING_ACTIVE.store(true, Ordering::Release);
    crate::kinfo!("Sampling profiler started (rate={rate} Hz, divider={divider})");
}

/// Stop the sampling profiler and drain all buffers to serial.
///
/// Emits the HPRF binary format header, all sample records, and the
/// end-of-stream marker.
pub fn stop() {
    SAMPLING_ACTIVE.store(false, Ordering::Release);

    // Brief fence to let any in-flight ISRs complete.
    core::sync::atomic::fence(Ordering::SeqCst);

    crate::kinfo!("Sampling profiler stopped, draining buffers...");

    let cpus = cpu_count();

    // Emit HPRF header.
    format::emit_header(format::FLAG_SAMPLES, 0, 0, cpus);

    // Drain each CPU's buffer.
    for cpu in 0..cpus {
        let cpu_id = CpuId::new(cpu);
        let buf_cell = SAMPLE_BUFFERS.get_for(cpu_id);
        // SAFETY: Sampling is stopped, no ISR is writing. We're the only reader.
        let buf: &mut SampleRingBuf = unsafe { &mut *buf_cell.0.get() };

        let count = buf.len();
        if count > 0 {
            crate::kinfo!("  CPU {cpu_id}: draining {count} samples");
        }

        buf.drain(|sample| {
            format::emit_sample_record(
                sample.cpu_id as u8,
                sample.tsc,
                &sample.stack[..sample.depth as usize],
                sample.depth as u16,
            );
        });
    }

    format::emit_end_of_stream();
    crate::kinfo!("Profiling data emission complete");
}

/// Called from the timer ISR to potentially capture a sample.
///
/// This is the hot path — when sampling is inactive, costs only a single
/// atomic load (~1 cycle).
///
/// # Arguments
///
/// - `interrupted_rip`: the RIP from the interrupt frame
/// - `interrupted_rsp`: the RSP from the interrupt frame
/// - `interrupted_rbp`: the RBP at the time of interrupt (frame pointer)
///
/// # Safety
///
/// Must only be called from the timer ISR (ring 0 path) with a valid
/// interrupt frame on the stack.
#[inline]
pub unsafe fn sample_capture(interrupted_rip: u64, _interrupted_rsp: u64, interrupted_rbp: u64) {
    // Fast path: check if sampling is active.
    if !SAMPLING_ACTIVE.load(Ordering::Relaxed) {
        return;
    }

    // Software divider: only sample every N-th tick.
    let counter_cell = TICK_COUNTERS.get();
    // SAFETY: Only accessed from the timer ISR on the owning CPU.
    let counter = unsafe { &mut *counter_cell.get() };
    *counter += 1;
    let divider = SAMPLE_DIVIDER.load(Ordering::Relaxed);
    if *counter < divider {
        return;
    }
    *counter = 0;

    // Capture the sample.
    let tsc_val = tsc::read_tsc();
    let percpu = current_cpu();
    let cpu_id = percpu.get_cpu_id();

    let mut sample = Sample {
        tsc: tsc_val,
        cpu_id: cpu_id.as_u32(),
        depth: 0,
        stack: [0; MAX_SAMPLE_DEPTH],
    };

    // First entry: the interrupted instruction pointer.
    sample.stack[0] = interrupted_rip;
    sample.depth = 1;

    // Walk the frame pointer chain from the interrupted context.
    let mut rbp = interrupted_rbp;
    while (sample.depth as usize) < MAX_SAMPLE_DEPTH {
        // Terminate on null or non-canonical RBP.
        if rbp == 0 || rbp < 0xFFFF_8000_0000_0000 || rbp & 0x7 != 0 {
            break;
        }

        // SAFETY: We validated that RBP is a kernel-space, aligned address.
        // The frame pointer chain was set up by -Cforce-frame-pointers=yes.
        let (next_rbp, ret_addr) = unsafe {
            let rbp_ptr = rbp as *const u64;
            (*rbp_ptr, *rbp_ptr.add(1))
        };

        if ret_addr >= 0xFFFF_8000_0000_0000 {
            sample.stack[sample.depth as usize] = ret_addr;
            sample.depth += 1;
        }

        // Detect loops.
        if next_rbp == rbp {
            break;
        }
        rbp = next_rbp;
    }

    // Push into the per-CPU ring buffer.
    let buf_cell = SAMPLE_BUFFERS.get_for(cpu_id);
    // SAFETY: Only the owning CPU's timer ISR writes to this buffer.
    let buf: &mut SampleRingBuf = unsafe { &mut *buf_cell.0.get() };
    buf.push(sample);
}

/// Returns whether the sampling profiler is currently active.
pub fn is_active() -> bool {
    SAMPLING_ACTIVE.load(Ordering::Relaxed)
}
