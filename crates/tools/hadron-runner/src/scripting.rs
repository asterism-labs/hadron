//! Scriptable VM wrapper for QEMU automation.
//!
//! Combines [`RunningQemu`](crate::RunningQemu), [`SerialPty`](crate::SerialPty),
//! and [`QmpClient`](crate::QmpClient) into a single `ScriptableVm` that
//! exposes high-level operations for test scripts.

use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Result, bail};

use crate::qemu::RunningQemu;
use crate::qmp::QmpClient;
use crate::serial::SerialPty;

/// A running QEMU instance with serial I/O and QMP control.
///
/// Created by the gluon `script` subcommand after booting QEMU with
/// a PTY serial port and QMP socket.
pub struct ScriptableVm {
    /// The QEMU child process.
    qemu: Mutex<RunningQemu>,
    /// PTY-based serial port for reading/writing kernel output.
    serial: SerialPty,
    /// QMP client for machine control (screenshots, key input, quit).
    qmp: Mutex<Option<QmpClient>>,
}

impl ScriptableVm {
    /// Creates a new scriptable VM from its components.
    #[must_use]
    pub fn new(qemu: RunningQemu, serial: SerialPty, qmp: Option<QmpClient>) -> Self {
        Self {
            qemu: Mutex::new(qemu),
            serial,
            qmp: Mutex::new(qmp),
        }
    }

    /// Waits for a serial line containing `pattern`, with a timeout in seconds.
    ///
    /// Returns the full matching line.
    ///
    /// # Errors
    ///
    /// Returns an error if no matching line appears before the timeout.
    ///
    /// # Panics
    ///
    /// Panics if the internal line buffer mutex is poisoned.
    pub fn wait_serial(&self, pattern: &str, timeout_secs: i64) -> Result<String> {
        let timeout = Duration::from_secs(timeout_secs.unsigned_abs());
        self.serial.wait_pattern(pattern, timeout)
    }

    /// Sends text data through the serial port.
    ///
    /// # Errors
    ///
    /// Returns an error if the write to the PTY master fails.
    pub fn send_serial(&self, data: &str) -> Result<()> {
        self.serial.send(data)
    }

    /// Takes a screenshot and saves it to the given path (PPM format).
    ///
    /// # Errors
    ///
    /// Returns an error if QMP is not connected or the screendump command fails.
    ///
    /// # Panics
    ///
    /// Panics if the QMP mutex is poisoned.
    pub fn screenshot(&self, path: &str) -> Result<()> {
        let mut qmp = self.qmp.lock().unwrap();
        let client = qmp
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("QMP not connected — screenshot requires QMP socket"))?;
        client.screendump(Path::new(path))
    }

    /// Sends a key combination via QMP (e.g. `"ctrl-alt-delete"`).
    ///
    /// # Errors
    ///
    /// Returns an error if QMP is not connected or the command fails.
    ///
    /// # Panics
    ///
    /// Panics if the QMP mutex is poisoned.
    pub fn send_key(&self, keys: &str) -> Result<()> {
        let mut qmp = self.qmp.lock().unwrap();
        let client = qmp
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("QMP not connected — send_key requires QMP socket"))?;
        let key_parts: Vec<&str> = keys.split('-').collect();
        client.send_key(&key_parts)
    }

    /// Gracefully quits QEMU via QMP, or kills the process if QMP is unavailable.
    ///
    /// # Errors
    ///
    /// Returns an error if the quit command or kill signal fails.
    ///
    /// # Panics
    ///
    /// Panics if either the QMP or QEMU mutex is poisoned.
    pub fn quit(&self) -> Result<()> {
        let mut qmp = self.qmp.lock().unwrap();
        if let Some(client) = qmp.as_mut() {
            client.quit()
        } else {
            self.qemu.lock().unwrap().kill()
        }
    }

    /// Waits for QEMU to exit and returns the exit code.
    ///
    /// # Errors
    ///
    /// Returns an error if QEMU does not exit within the timeout.
    ///
    /// # Panics
    ///
    /// Panics if the QEMU mutex is poisoned.
    pub fn wait_exit(&self, timeout_secs: i64) -> Result<i64> {
        let timeout = Duration::from_secs(timeout_secs.unsigned_abs());
        let exit = self.qemu.lock().unwrap().wait_with_timeout(timeout)?;
        if exit.timed_out {
            bail!("QEMU did not exit within {timeout_secs}s");
        }
        Ok(i64::from(exit.exit_code))
    }

    /// Waits for QEMU to exit and asserts the expected exit code.
    ///
    /// # Errors
    ///
    /// Returns an error if QEMU exits with a different code than expected.
    ///
    /// # Panics
    ///
    /// Panics if the QEMU mutex is poisoned.
    pub fn assert_exit(&self, expected: i64) -> Result<()> {
        let exit = self.qemu.lock().unwrap().wait()?;
        if i64::from(exit.exit_code) != expected {
            bail!("expected exit code {expected}, got {}", exit.exit_code);
        }
        Ok(())
    }

    /// Waits for a serial line matching the given regex pattern, with a
    /// timeout in seconds.
    ///
    /// Returns the full matching line.
    ///
    /// # Errors
    ///
    /// Returns an error if the regex is invalid or no match before timeout.
    pub fn wait_serial_regex(&self, pattern: &str, timeout_secs: i64) -> Result<String> {
        let re = regex::Regex::new(pattern)
            .map_err(|e| anyhow::anyhow!("invalid regex '{pattern}': {e}"))?;
        let timeout = Duration::from_secs(timeout_secs.unsigned_abs());
        self.serial.wait_pattern_regex(&re, timeout)
    }

    /// Returns the last `n` lines from the serial log (for error context).
    ///
    /// # Panics
    ///
    /// Panics if the serial log mutex is poisoned.
    #[must_use]
    pub fn last_serial_lines(&self, n: usize) -> Vec<String> {
        self.serial.last_lines(n)
    }

    /// Returns all captured serial output lines.
    ///
    /// # Panics
    ///
    /// Panics if the serial log mutex is poisoned.
    #[must_use]
    pub fn serial_log(&self) -> Vec<String> {
        self.serial.serial_log()
    }
}
