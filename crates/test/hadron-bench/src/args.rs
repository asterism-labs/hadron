//! Command-line argument parser for benchmark binaries.
//!
//! Handles arguments received via the kernel command line:
//! - Positional filter string (substring match on benchmark names)
//! - `--exact` for exact name matching
//! - `--list` to list benchmarks without running them
//! - `--quiet` / `-q` for reduced output
//! - `--warmup N` to set warmup iterations
//! - `--samples N` to set sample count
//! - `--skip NAME` to skip specific benchmarks

/// Parsed benchmark arguments from the kernel command line.
#[derive(Debug)]
pub struct BenchArgs<'a> {
    /// Filter string — run only benchmarks whose name contains this.
    pub filter: Option<&'a str>,
    /// If true, match filter exactly (not substring).
    pub exact: bool,
    /// If true, list benchmarks without running them.
    pub list: bool,
    /// If true, less output.
    pub quiet: bool,
    /// Number of warmup iterations before sampling.
    pub warmup: u32,
    /// Number of sample iterations to collect.
    pub samples: u32,
    /// Benchmark name to skip (substring match).
    pub skip: Option<&'a str>,
}

/// Default warmup iterations.
const DEFAULT_WARMUP: u32 = 100;
/// Default sample count.
const DEFAULT_SAMPLES: u32 = 100;

impl<'a> BenchArgs<'a> {
    /// Parse benchmark arguments from the kernel command line string.
    pub fn parse(cmdline: Option<&'a str>) -> Self {
        let mut args = Self {
            filter: None,
            exact: false,
            list: false,
            quiet: false,
            warmup: DEFAULT_WARMUP,
            samples: DEFAULT_SAMPLES,
            skip: None,
        };

        let Some(cmdline) = cmdline else {
            return args;
        };

        if cmdline.is_empty() {
            return args;
        }

        let mut iter = cmdline.split_whitespace();
        while let Some(token) = iter.next() {
            match token {
                "--list" => args.list = true,
                "--exact" => args.exact = true,
                "--quiet" | "-q" => args.quiet = true,
                "--warmup" => {
                    if let Some(val) = iter.next() {
                        args.warmup = parse_u32(val, DEFAULT_WARMUP);
                    }
                }
                "--samples" => {
                    if let Some(val) = iter.next() {
                        args.samples = parse_u32(val, DEFAULT_SAMPLES);
                    }
                }
                "--skip" => {
                    args.skip = iter.next();
                }
                "--nocapture" | "--test-threads" => {
                    let _ = iter.next(); // consume and ignore
                }
                _ if token.starts_with("--") => {} // unknown flag, ignore
                _ => {
                    if args.filter.is_none() {
                        args.filter = Some(token);
                    }
                }
            }
        }

        args
    }

    /// Check if a benchmark name matches the current filter settings.
    pub fn matches(&self, name: &str) -> bool {
        // Check skip filter first.
        if let Some(skip) = self.skip {
            if name.contains(skip) {
                return false;
            }
        }

        match self.filter {
            None => true,
            Some(filter) => {
                if self.exact {
                    name == filter
                } else {
                    name.contains(filter)
                }
            }
        }
    }
}

/// Parse a `&str` into `u32`, returning `default` on failure.
fn parse_u32(s: &str, default: u32) -> u32 {
    let mut result: u32 = 0;
    for &b in s.as_bytes() {
        if b.is_ascii_digit() {
            result = result.saturating_mul(10).saturating_add(u32::from(b - b'0'));
        } else {
            return default;
        }
    }
    if s.is_empty() { default } else { result }
}
