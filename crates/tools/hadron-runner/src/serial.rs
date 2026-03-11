//! Serial port configuration types and PTY-based serial capture for QEMU.

use std::path::PathBuf;

/// Configuration for a single QEMU serial port.
#[derive(Debug, Clone)]
pub enum SerialConfig {
    /// Route to process stdio (COM1 console).
    Stdio,
    /// Route to a file (COM2 logging).
    File(PathBuf),
    /// Route to a Unix socket.
    Socket(PathBuf),
    /// Route through a PTY pair for programmatic read/write.
    Pty(PathBuf),
    /// Disable this serial port.
    None,
}

impl SerialConfig {
    /// Convert to QEMU `-serial` argument value.
    pub(crate) fn to_qemu_arg(&self) -> String {
        match self {
            Self::Stdio => "stdio".to_string(),
            Self::File(path) => format!("file:{}", path.display()),
            Self::Socket(path) => format!("unix:{},server=on,wait=off", path.display()),
            Self::Pty(path) => path.to_string_lossy().into_owned(),
            Self::None => "none".to_string(),
        }
    }
}

// ── PTY-based serial I/O ─────────────────────────────────────────────────

use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

/// A PTY-based serial capture for QEMU.
///
/// Opens a pseudo-terminal pair. The slave path is passed to QEMU via
/// `-serial <path>`. The master side is used for reading/writing serial data.
pub struct SerialPty {
    /// Master file descriptor for reading/writing.
    master_fd: i32,
    /// Path to the slave PTY device (e.g. `/dev/pts/N`).
    slave_path: PathBuf,
    /// Accumulated serial output lines for the log.
    log: Arc<Mutex<Vec<String>>>,
    /// Reader thread handle.
    _reader_thread: Option<std::thread::JoinHandle<()>>,
    /// Line buffer shared with reader thread.
    lines: Arc<Mutex<Vec<String>>>,
}

impl SerialPty {
    /// Creates a new PTY pair and starts a background reader thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the PTY cannot be created or duplicated.
    pub fn new() -> Result<Self> {
        // SAFETY: openpty allocates a PTY pair. Both fds are valid on success.
        let (master_fd, slave_fd) = unsafe {
            let mut master: libc::c_int = 0;
            let mut slave: libc::c_int = 0;
            if libc::openpty(
                &raw mut master,
                &raw mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            ) != 0
            {
                bail!("openpty failed: {}", std::io::Error::last_os_error());
            }
            (master, slave)
        };

        // Get the slave device path.
        let slave_path = unsafe {
            let name_ptr = libc::ptsname(master_fd);
            if name_ptr.is_null() {
                libc::close(master_fd);
                libc::close(slave_fd);
                bail!("ptsname failed: {}", std::io::Error::last_os_error());
            }
            let cstr = std::ffi::CStr::from_ptr(name_ptr);
            PathBuf::from(cstr.to_string_lossy().into_owned())
        };

        // Close slave fd — QEMU will open it by path.
        // SAFETY: slave_fd is a valid file descriptor from openpty.
        unsafe { libc::close(slave_fd) };

        // Set master to non-blocking for reads.
        unsafe {
            let flags = libc::fcntl(master_fd, libc::F_GETFL);
            libc::fcntl(master_fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        let lines = Arc::new(Mutex::new(Vec::new()));
        let log = Arc::new(Mutex::new(Vec::new()));

        let lines_clone = Arc::clone(&lines);
        let log_clone = Arc::clone(&log);

        // SAFETY: We duplicate the master fd for the reader thread.
        // The fd is valid and will remain open for the lifetime of SerialPty.
        let reader_fd = unsafe { libc::dup(master_fd) };
        if reader_fd < 0 {
            // SAFETY: master_fd is valid.
            unsafe { libc::close(master_fd) };
            bail!("dup failed: {}", std::io::Error::last_os_error());
        }

        // Wrap reader_fd in a File for BufReader. Set it to blocking for the reader thread.
        let reader_thread = std::thread::spawn(move || {
            // SAFETY: reader_fd is a valid file descriptor from dup.
            unsafe {
                let flags = libc::fcntl(reader_fd, libc::F_GETFL);
                libc::fcntl(reader_fd, libc::F_SETFL, flags & !libc::O_NONBLOCK);
            }
            // SAFETY: reader_fd is a valid, open file descriptor.
            let file: std::fs::File = unsafe { std::os::fd::FromRawFd::from_raw_fd(reader_fd) };
            let reader = BufReader::new(file);

            for line_result in reader.lines() {
                match line_result {
                    Ok(line) => {
                        if let Ok(mut log) = log_clone.lock() {
                            log.push(line.clone());
                        }
                        if let Ok(mut lines) = lines_clone.lock() {
                            lines.push(line);
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            master_fd,
            slave_path,
            log,
            _reader_thread: Some(reader_thread),
            lines,
        })
    }

    /// Returns the path to the slave PTY device for QEMU's `-serial` flag.
    #[must_use]
    pub fn slave_path(&self) -> &std::path::Path {
        &self.slave_path
    }

    /// Waits for a line containing the given substring pattern.
    ///
    /// Returns the first matching line, or an error on timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if no matching line appears before the timeout.
    ///
    /// # Panics
    ///
    /// Panics if the internal line buffer mutex is poisoned.
    pub fn wait_pattern(&self, pattern: &str, timeout: Duration) -> Result<String> {
        let start = Instant::now();

        loop {
            {
                let mut lines = self.lines.lock().unwrap();
                if let Some(idx) = lines.iter().position(|l| l.contains(pattern)) {
                    let matched = lines.remove(idx);
                    return Ok(matched);
                }
            }

            if start.elapsed() >= timeout {
                bail!("timeout waiting for pattern '{pattern}' after {timeout:?}");
            }

            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Reads one line with a timeout.
    ///
    /// Returns the next available line, or an error on timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if no line is available before the timeout.
    ///
    /// # Panics
    ///
    /// Panics if the internal line buffer mutex is poisoned.
    pub fn read_line_timeout(&self, timeout: Duration) -> Result<String> {
        let start = Instant::now();
        loop {
            {
                let mut lines = self.lines.lock().unwrap();
                if !lines.is_empty() {
                    return Ok(lines.remove(0));
                }
            }

            if start.elapsed() >= timeout {
                bail!("timeout waiting for serial line after {timeout:?}");
            }

            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits for a line matching the given regex pattern.
    ///
    /// Returns the first matching line, or an error on timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if no matching line appears before the timeout.
    ///
    /// # Panics
    ///
    /// Panics if the internal line buffer mutex is poisoned.
    pub fn wait_pattern_regex(&self, re: &regex::Regex, timeout: Duration) -> Result<String> {
        let start = Instant::now();

        loop {
            {
                let mut lines = self.lines.lock().unwrap();
                if let Some(idx) = lines.iter().position(|l| re.is_match(l)) {
                    let matched = lines.remove(idx);
                    return Ok(matched);
                }
            }

            if start.elapsed() >= timeout {
                bail!(
                    "timeout waiting for regex '{}' after {timeout:?}",
                    re.as_str()
                );
            }

            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Returns the last `n` lines from the serial log (for error context).
    ///
    /// # Panics
    ///
    /// Panics if the serial log mutex is poisoned.
    #[must_use]
    pub fn last_lines(&self, n: usize) -> Vec<String> {
        let log = self.log.lock().unwrap();
        let start = log.len().saturating_sub(n);
        log[start..].to_vec()
    }

    /// Sends data through the serial port.
    ///
    /// # Errors
    ///
    /// Returns an error if the write to the PTY master fails.
    pub fn send(&self, data: &str) -> Result<()> {
        let bytes = data.as_bytes();
        // SAFETY: master_fd is a valid file descriptor; write is atomic for small data.
        let written = unsafe { libc::write(self.master_fd, bytes.as_ptr().cast(), bytes.len()) };
        if written < 0 {
            bail!("serial write failed: {}", std::io::Error::last_os_error());
        }
        Ok(())
    }

    /// Drains all currently buffered lines (non-blocking).
    ///
    /// # Panics
    ///
    /// Panics if the internal line buffer mutex is poisoned.
    #[must_use]
    pub fn drain(&self) -> Vec<String> {
        let mut lines = self.lines.lock().unwrap();
        std::mem::take(&mut *lines)
    }

    /// Returns all captured serial output since PTY creation.
    ///
    /// # Panics
    ///
    /// Panics if the serial log mutex is poisoned.
    #[must_use]
    pub fn serial_log(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

impl Drop for SerialPty {
    fn drop(&mut self) {
        // SAFETY: master_fd is a valid file descriptor that we own.
        unsafe { libc::close(self.master_fd) };
    }
}
