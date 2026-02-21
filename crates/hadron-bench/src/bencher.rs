//! Benchmark iteration controller.
//!
//! The [`Bencher`] controls warmup and sampling phases, using fenced `rdtscp`
//! pairs for cycle-accurate measurement. Samples are stored in a stack-allocated
//! array (max 256 entries).

/// Maximum number of samples that can be stored per benchmark.
pub const MAX_SAMPLES: usize = 256;

/// Default number of warmup iterations.
pub const DEFAULT_WARMUP: u32 = 100;

/// Default number of sample iterations.
pub const DEFAULT_SAMPLES: u32 = 100;

/// Controls the measurement of a single benchmark function.
///
/// The benchmark function receives a `&mut Bencher` and calls [`Bencher::iter`]
/// with its workload closure. The bencher handles warmup iterations and then
/// collects cycle-accurate timing samples.
pub struct Bencher {
    warmup_count: u32,
    sample_count: u32,
    samples: [u64; MAX_SAMPLES],
    samples_collected: u32,
}

impl Bencher {
    /// Create a new bencher with the given warmup and sample counts.
    pub fn new(warmup: u32, samples: u32) -> Self {
        Self {
            warmup_count: warmup,
            sample_count: samples.min(MAX_SAMPLES as u32),
            samples: [0u64; MAX_SAMPLES],
            samples_collected: 0,
        }
    }

    /// Set the number of warmup iterations.
    pub fn warmup(&mut self, count: u32) -> &mut Self {
        self.warmup_count = count;
        self
    }

    /// Set the number of sample iterations.
    pub fn sample_size(&mut self, count: u32) -> &mut Self {
        self.sample_count = count.min(MAX_SAMPLES as u32);
        self
    }

    /// Run the benchmark closure, performing warmup then sampling.
    ///
    /// The closure is called `warmup + samples` times. During the sampling
    /// phase, each call is bracketed by fenced TSC reads to measure cycles.
    pub fn iter<F, R>(&mut self, mut f: F)
    where
        F: FnMut() -> R,
    {
        // Warmup phase: run without timing.
        for _ in 0..self.warmup_count {
            black_box(f());
        }

        // Sampling phase: measure each iteration.
        self.samples_collected = 0;
        for i in 0..self.sample_count {
            // Serialize before start measurement.
            let start = fenced_rdtsc();
            let result = f();
            let end = fenced_rdtsc();
            black_box(result);

            let elapsed = end.saturating_sub(start);
            self.samples[i as usize] = elapsed;
            self.samples_collected += 1;
        }
    }

    /// Returns the collected samples as a mutable slice.
    pub fn samples_mut(&mut self) -> &mut [u64] {
        &mut self.samples[..self.samples_collected as usize]
    }

    /// Returns the collected samples as a slice.
    pub fn samples(&self) -> &[u64] {
        &self.samples[..self.samples_collected as usize]
    }

    /// Returns the number of samples collected.
    pub fn samples_collected(&self) -> u32 {
        self.samples_collected
    }
}

/// Compiler optimization barrier. Prevents the compiler from optimizing
/// away the result of a benchmark computation.
///
/// Uses a volatile read to force the compiler to materialize the value.
/// The original value is forgotten to prevent double-drop when `T` has
/// a destructor (e.g. `Box`, `Vec`).
#[inline]
pub fn black_box<T>(x: T) -> T {
    // SAFETY: We read from a reference to `x` using a volatile read,
    // then forget `x` so only the returned copy exists. This prevents
    // double-free for types with destructors.
    unsafe {
        let ptr = &x as *const T;
        let ret = core::ptr::read_volatile(ptr);
        core::mem::forget(x);
        ret
    }
}

/// Read TSC with serialization via `LFENCE` + `RDTSC` + `LFENCE`.
///
/// `LFENCE` before serializes prior instructions so the read isn't
/// reordered earlier. `LFENCE` after prevents subsequent instructions
/// from executing before the read completes. Uses `RDTSC` (not `RDTSCP`)
/// for compatibility with all QEMU CPU models.
#[cfg(target_arch = "x86_64")]
#[inline]
fn fenced_rdtsc() -> u64 {
    let lo: u32;
    let hi: u32;
    // SAFETY: LFENCE and RDTSC are always available on x86_64 and have
    // no side effects beyond reading the timestamp counter.
    unsafe {
        core::arch::asm!(
            "lfence",
            "rdtsc",
            "lfence",
            out("eax") lo,
            out("edx") hi,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(hi) << 32) | u64::from(lo)
}

#[cfg(target_arch = "aarch64")]
#[inline]
fn fenced_rdtsc() -> u64 {
    todo!("aarch64 fenced_rdtsc")
}
