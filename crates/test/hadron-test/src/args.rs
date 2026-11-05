//! Minimal no_std command-line argument parser for test binaries.
//!
//! Handles libtest-compatible arguments received via the kernel command line:
//! - Positional filter string (substring match on test names)
//! - `--exact` for exact name matching
//! - `--list` to list tests without running them
//! - `--quiet` / `-q` for reduced output
//! - `--nocapture`, `--test-threads` are accepted but ignored (always the case in kernel)

/// Parsed test arguments from the kernel command line.
#[derive(Debug)]
pub struct TestArgs<'a> {
    /// Filter string — run only tests whose name contains this.
    pub filter: Option<&'a str>,
    /// If true, match filter exactly (not substring).
    pub exact: bool,
    /// If true, list tests without running them.
    pub list: bool,
    /// If true, less output.
    pub quiet: bool,
}

impl<'a> TestArgs<'a> {
    /// Parse test arguments from the kernel command line string.
    ///
    /// Accepts the same arguments as libtest:
    /// - First positional (non-flag) token is the filter
    /// - `--exact`, `--list`, `--quiet`/`-q` are recognized
    /// - `--nocapture`, `--test-threads N` are accepted but ignored
    /// - Unknown `--` flags are ignored for forward compatibility
    pub fn parse(cmdline: Option<&'a str>) -> Self {
        let mut args = Self {
            filter: None,
            exact: false,
            list: false,
            quiet: false,
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
                "--nocapture" => {} // always the case in kernel
                "--test-threads" => {
                    let _ = iter.next(); // consume and ignore value
                }
                _ if token.starts_with("--") => {} // unknown flag, ignore
                _ => {
                    // First non-flag token is the filter
                    if args.filter.is_none() {
                        args.filter = Some(token);
                    }
                }
            }
        }

        args
    }

    /// Check if a test name matches the current filter settings.
    pub fn matches(&self, test_name: &str) -> bool {
        match self.filter {
            None => true,
            Some(filter) => {
                if self.exact {
                    test_name == filter
                } else {
                    test_name.contains(filter)
                }
            }
        }
    }
}
