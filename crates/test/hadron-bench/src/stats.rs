//! Integer-only statistics for benchmark results.
//!
//! All calculations use `u64` arithmetic — no floating point, suitable for
//! kernel environments without FPU access.

/// Computed statistics for a benchmark run.
#[derive(Debug, Clone)]
pub struct BenchStats {
    /// Minimum sample value (cycles).
    pub min: u64,
    /// Maximum sample value (cycles).
    pub max: u64,
    /// Median sample value (cycles).
    pub median: u64,
    /// Mean sample value (cycles).
    pub mean: u64,
    /// Standard deviation (integer approximation, cycles).
    pub stddev: u64,
    /// Number of samples.
    pub count: u32,
}

impl BenchStats {
    /// Compute statistics from a mutable slice of samples.
    ///
    /// The slice is sorted in place. Returns `None` if the slice is empty.
    pub fn compute(samples: &mut [u64]) -> Option<Self> {
        let n = samples.len();
        if n == 0 {
            return None;
        }

        // Sort for median.
        samples.sort_unstable();

        let min = samples[0];
        let max = samples[n - 1];
        let median = if n % 2 == 0 {
            (samples[n / 2 - 1] + samples[n / 2]) / 2
        } else {
            samples[n / 2]
        };

        // Mean via u128 to avoid overflow.
        let sum: u128 = samples.iter().map(|&s| u128::from(s)).sum();
        let mean = (sum / n as u128) as u64;

        // Stddev via integer square root of variance.
        let variance = if n > 1 {
            let var_sum: u128 = samples
                .iter()
                .map(|&s| {
                    let diff = if s >= mean { s - mean } else { mean - s };
                    u128::from(diff) * u128::from(diff)
                })
                .sum();
            (var_sum / (n as u128 - 1)) as u64
        } else {
            0
        };
        let stddev = isqrt(variance);

        Some(Self {
            min,
            max,
            median,
            mean,
            stddev,
            count: n as u32,
        })
    }

    /// Convert cycles to nanoseconds given TSC frequency in kHz.
    pub fn cycles_to_nanos(cycles: u64, tsc_freq_khz: u64) -> u64 {
        if tsc_freq_khz == 0 {
            return 0;
        }
        // cycles * 1_000_000 / tsc_freq_khz = nanoseconds
        (u128::from(cycles) * 1_000_000 / u128::from(tsc_freq_khz)) as u64
    }
}

/// Integer square root via Newton's method.
fn isqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_single_sample() {
        let mut samples = [42u64];
        let stats = BenchStats::compute(&mut samples).unwrap();
        assert_eq!(stats.min, 42);
        assert_eq!(stats.max, 42);
        assert_eq!(stats.median, 42);
        assert_eq!(stats.mean, 42);
        assert_eq!(stats.stddev, 0);
        assert_eq!(stats.count, 1);
    }

    #[test]
    fn stats_two_samples() {
        let mut samples = [10u64, 20];
        let stats = BenchStats::compute(&mut samples).unwrap();
        assert_eq!(stats.min, 10);
        assert_eq!(stats.max, 20);
        assert_eq!(stats.median, 15);
        assert_eq!(stats.mean, 15);
        assert_eq!(stats.count, 2);
    }

    #[test]
    fn stats_odd_count() {
        let mut samples = [5u64, 1, 9, 3, 7];
        let stats = BenchStats::compute(&mut samples).unwrap();
        assert_eq!(stats.min, 1);
        assert_eq!(stats.max, 9);
        assert_eq!(stats.median, 5);
        assert_eq!(stats.mean, 5);
        assert_eq!(stats.count, 5);
    }

    #[test]
    fn stats_empty() {
        let mut samples: [u64; 0] = [];
        assert!(BenchStats::compute(&mut samples).is_none());
    }

    #[test]
    fn isqrt_values() {
        assert_eq!(isqrt(0), 0);
        assert_eq!(isqrt(1), 1);
        assert_eq!(isqrt(4), 2);
        assert_eq!(isqrt(9), 3);
        assert_eq!(isqrt(100), 10);
        assert_eq!(isqrt(101), 10);
    }

    #[test]
    fn cycles_to_nanos_conversion() {
        // 2 GHz = 2_000_000 kHz, 2000 cycles = 1000 ns
        assert_eq!(BenchStats::cycles_to_nanos(2000, 2_000_000), 1000);
    }
}
