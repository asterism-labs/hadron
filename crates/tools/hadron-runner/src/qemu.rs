//! QEMU process launcher and configuration.
//!
//! Builds QEMU command lines from [`QemuConfig`], spawns child processes,
//! and manages timeouts and exit code interpretation.

use std::io::Read as _;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context, Result};

use crate::qmp::QmpClient;
use crate::serial::SerialConfig;

/// QEMU machine configuration.
#[derive(Debug, Clone)]
pub struct QemuConfig {
    /// QEMU machine type (e.g. `"q35"`).
    pub machine: String,
    /// Memory in megabytes.
    pub memory: u32,
    /// Number of CPU cores.
    pub cores: u32,
    /// CPU model (e.g. `"max"`).
    pub cpu: String,
    /// Boot-specific arguments (e.g. `["-cdrom", "image.iso"]` or `["-bios", "OVMF.fd", "-kernel", "boot.efi"]`).
    pub boot_args: Vec<String>,
    /// Serial port configurations (one per QEMU `-serial` flag).
    pub serial: Vec<SerialConfig>,
    /// Path for QMP Unix socket, if QMP is desired.
    pub qmp_socket: Option<PathBuf>,
    /// Display configuration.
    pub display: DisplayConfig,
    /// Extra QEMU command-line arguments.
    pub extra_args: Vec<String>,
    /// Test mode configuration.
    pub test_mode: Option<TestConfig>,
}

/// Test-specific QEMU configuration.
#[derive(Debug, Clone)]
pub struct TestConfig {
    /// Exit code that indicates test success (typically 33 via `isa-debug-exit`).
    pub success_exit_code: i32,
    /// Maximum seconds before killing QEMU.
    pub timeout_secs: u64,
}

/// QEMU display backend.
#[derive(Debug, Clone)]
pub enum DisplayConfig {
    /// No display (`-display none`).
    None,
    /// GTK display.
    Gtk,
    /// Use QEMU default display.
    Default,
}

/// A running QEMU process.
pub struct RunningQemu {
    child: std::process::Child,
    qmp_socket: Option<PathBuf>,
    test_mode: Option<TestConfig>,
}

/// Result of a QEMU execution.
#[derive(Debug, Clone)]
pub struct QemuExit {
    /// Raw exit code from the process.
    pub exit_code: i32,
    /// Whether the process was killed due to timeout.
    pub timed_out: bool,
    /// Whether the exit code indicates success.
    pub success: bool,
}

/// IO handler callback for serial output capture.
pub trait IoHandler: Send {
    /// Called when serial output data is received on stdout.
    ///
    /// Return `true` to continue, `false` to kill QEMU.
    fn on_output(&mut self, data: &[u8]) -> bool;
}

/// Internal events from reader threads.
enum IoEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    StdoutClosed,
    StderrClosed,
}

impl QemuConfig {
    /// Build a QEMU command from this configuration.
    fn build_command(&self) -> Command {
        let mut cmd = Command::new("qemu-system-x86_64");

        cmd.arg("-machine").arg(&self.machine);
        cmd.arg("-m").arg(self.memory.to_string());

        if self.cores > 1 {
            cmd.arg("-smp").arg(self.cores.to_string());
        }

        cmd.arg("-cpu").arg(&self.cpu);

        // Boot-specific args (e.g. -cdrom, -bios/-kernel, etc.)
        for arg in &self.boot_args {
            cmd.arg(arg);
        }

        // Serial ports
        for serial in &self.serial {
            cmd.arg("-serial").arg(serial.to_qemu_arg());
        }
        // Disable QEMU monitor to keep stdout clean
        cmd.arg("-monitor").arg("none");

        // QMP socket
        if let Some(ref socket) = self.qmp_socket {
            cmd.arg("-qmp")
                .arg(format!("unix:{},server=on,wait=off", socket.display()));
        }

        // Display
        match &self.display {
            DisplayConfig::None | DisplayConfig::Gtk => {
                let name = match self.display {
                    DisplayConfig::None => "none",
                    DisplayConfig::Gtk => "gtk",
                    DisplayConfig::Default => unreachable!(),
                };
                cmd.arg("-display").arg(name);
            }
            DisplayConfig::Default => {}
        }

        // Test mode: isa-debug-exit device + no-reboot
        if self.test_mode.is_some() {
            cmd.args([
                "-device",
                "isa-debug-exit,iobase=0xf4,iosize=0x04",
                "-no-reboot",
            ]);
        }

        // Extra args
        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        cmd
    }

    /// Spawn QEMU, inheriting stdio for interactive use.
    ///
    /// # Errors
    ///
    /// Returns an error if QEMU cannot be spawned.
    pub fn spawn(&self) -> Result<RunningQemu> {
        let mut cmd = self.build_command();

        cmd.stdin(Stdio::inherit());
        cmd.stdout(Stdio::inherit());
        cmd.stderr(Stdio::inherit());

        let child = cmd.spawn().context("spawning qemu-system-x86_64")?;

        Ok(RunningQemu {
            child,
            qmp_socket: self.qmp_socket.clone(),
            test_mode: self.test_mode.clone(),
        })
    }

    /// Spawn QEMU with piped stdio for programmatic IO capture.
    ///
    /// When using this mode, serial must be configured to `Stdio` for
    /// output to arrive on the child's stdout.
    ///
    /// # Errors
    ///
    /// Returns an error if QEMU cannot be spawned.
    pub fn spawn_piped(&self) -> Result<RunningQemu> {
        let mut cmd = self.build_command();

        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let child = cmd.spawn().context("spawning qemu-system-x86_64 (piped)")?;

        Ok(RunningQemu {
            child,
            qmp_socket: self.qmp_socket.clone(),
            test_mode: self.test_mode.clone(),
        })
    }
}

/// Send SIGKILL to a process by PID.
#[cfg(unix)]
fn kill_process(pid: u32) {
    // SAFETY: Sending SIGKILL to a process we spawned. The PID was valid
    // at spawn time; if the process has already exited, kill() is a no-op.
    unsafe {
        libc::kill(i32::try_from(pid).unwrap_or(0), libc::SIGKILL);
    }
}

impl RunningQemu {
    /// Wait for QEMU to exit, interpreting the exit code.
    ///
    /// # Errors
    ///
    /// Returns an error if waiting for the child process fails.
    pub fn wait(&mut self) -> Result<QemuExit> {
        let status = self.child.wait().context("waiting for QEMU")?;

        let exit_code = status.code().unwrap_or(-1);
        let success = self.is_success(exit_code);

        Ok(QemuExit {
            exit_code,
            timed_out: false,
            success,
        })
    }

    /// Wait for QEMU with a timeout. Kills the process if it exceeds the timeout.
    ///
    /// # Errors
    ///
    /// Returns an error if waiting for the child process fails.
    pub fn wait_with_timeout(&mut self, timeout: Duration) -> Result<QemuExit> {
        let timed_out = Arc::new(AtomicBool::new(false));
        let flag = timed_out.clone();
        let child_id = self.child.id();

        let _watchdog = std::thread::spawn(move || {
            std::thread::sleep(timeout);
            if !flag.swap(true, Ordering::SeqCst) {
                #[cfg(unix)]
                kill_process(child_id);
            }
        });

        let status = self.child.wait().context("waiting for QEMU")?;

        let was_timed_out = timed_out.swap(true, Ordering::SeqCst);
        let exit_code = status.code().unwrap_or(-1);
        let success = !was_timed_out && self.is_success(exit_code);

        Ok(QemuExit {
            exit_code,
            timed_out: was_timed_out,
            success,
        })
    }

    /// Wait for QEMU with an IO handler that captures serial output.
    ///
    /// The QEMU process must have been spawned with [`QemuConfig::spawn_piped`].
    ///
    /// # Errors
    ///
    /// Returns an error if stdout/stderr are not piped or waiting fails.
    #[allow(clippy::too_many_lines)]
    pub fn wait_with_io(&mut self, handler: &mut dyn IoHandler) -> Result<QemuExit> {
        let child_stdout = self
            .child
            .stdout
            .take()
            .context("QEMU stdout not piped — use spawn_piped()")?;
        let child_stderr = self
            .child
            .stderr
            .take()
            .context("QEMU stderr not piped — use spawn_piped()")?;
        let mut child_stdin = self.child.stdin.take();

        let (tx, rx) = mpsc::channel::<IoEvent>();

        // Stdout reader thread
        let tx_out = tx.clone();
        let stdout_thread = std::thread::spawn(move || {
            let mut reader = child_stdout;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx_out.send(IoEvent::StdoutClosed);
                        break;
                    }
                    Ok(n) => {
                        if tx_out.send(IoEvent::Stdout(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Stderr reader thread
        let stderr_thread = std::thread::spawn(move || {
            let mut reader = child_stderr;
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) | Err(_) => {
                        let _ = tx.send(IoEvent::StderrClosed);
                        break;
                    }
                    Ok(n) => {
                        if tx.send(IoEvent::Stderr(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Timeout watchdog
        let timed_out = Arc::new(AtomicBool::new(false));
        if let Some(ref test) = self.test_mode {
            let flag = timed_out.clone();
            let timeout = test.timeout_secs;
            let child_id = self.child.id();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(timeout));
                if !flag.swap(true, Ordering::SeqCst) {
                    #[cfg(unix)]
                    kill_process(child_id);
                }
            });
        }

        // Main IO event loop
        let mut stdout_closed = false;
        let mut stderr_closed = false;

        while !stdout_closed || !stderr_closed {
            let Ok(event) = rx.recv() else { break };

            match event {
                IoEvent::Stdout(data) => {
                    if !handler.on_output(&data) {
                        let _ = self.child.kill();
                        break;
                    }
                }
                IoEvent::Stderr(data) => {
                    // Tee stderr to the host's stderr
                    let _ = std::io::stderr().write_all(&data);
                }
                IoEvent::StdoutClosed => stdout_closed = true,
                IoEvent::StderrClosed => stderr_closed = true,
            }
        }

        // Drop stdin to unblock child
        drop(child_stdin.take());

        let status = self
            .child
            .wait()
            .context("waiting for QEMU after IO loop")?;

        let _ = stdout_thread.join();
        let _ = stderr_thread.join();

        // Signal the timeout thread that we're done
        let was_timed_out = timed_out.swap(true, Ordering::SeqCst);
        let exit_code = status.code().unwrap_or(-1);
        let success = !was_timed_out && self.is_success(exit_code);

        Ok(QemuExit {
            exit_code,
            timed_out: was_timed_out,
            success,
        })
    }

    /// Kill the QEMU process.
    ///
    /// # Errors
    ///
    /// Returns an error if the kill signal cannot be sent.
    pub fn kill(&mut self) -> Result<()> {
        self.child.kill().context("killing QEMU")
    }

    /// Connect to the QMP socket, if configured.
    ///
    /// # Errors
    ///
    /// Returns an error if QMP is not configured or connection fails.
    pub fn connect_qmp(&self) -> Result<QmpClient> {
        let socket = self
            .qmp_socket
            .as_ref()
            .context("QMP socket not configured")?;
        QmpClient::connect(socket)
    }

    /// Check whether an exit code indicates success.
    fn is_success(&self, exit_code: i32) -> bool {
        if let Some(ref test) = self.test_mode {
            exit_code == test.success_exit_code
        } else {
            exit_code == 0
        }
    }
}
