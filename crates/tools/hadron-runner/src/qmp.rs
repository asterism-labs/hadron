//! QMP (QEMU Machine Protocol) client.
//!
//! Connects to a QEMU QMP Unix socket for programmatic control:
//! keyboard/mouse input, screenshots, VM status, and graceful shutdown.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;

/// QMP protocol client connected to a QEMU instance.
pub struct QmpClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

/// Mouse button identifiers.
#[derive(Debug, Clone, Copy)]
pub enum MouseButton {
    /// Left mouse button.
    Left,
    /// Right mouse button.
    Right,
    /// Middle mouse button.
    Middle,
}

/// VM status from `query-status`.
#[derive(Debug)]
pub struct VmStatus {
    /// Whether the VM is currently running.
    pub running: bool,
    /// The VM status string (e.g. `"running"`, `"paused"`).
    pub status: String,
}

impl QmpClient {
    /// Connect to a QMP Unix socket and perform capability negotiation.
    ///
    /// Retries connection for up to 2 seconds to handle QEMU startup delay.
    ///
    /// # Errors
    ///
    /// Returns an error if connection, greeting, or negotiation fails.
    pub fn connect(path: &Path) -> Result<Self> {
        let stream = Self::connect_with_retry(path, Duration::from_secs(2))?;
        let writer = stream.try_clone().context("cloning QMP socket")?;
        let mut reader = BufReader::new(stream);

        // Read QMP greeting
        let mut greeting = String::new();
        reader
            .read_line(&mut greeting)
            .context("reading QMP greeting")?;

        let greeting_json: Value =
            serde_json::from_str(&greeting).context("parsing QMP greeting")?;
        if greeting_json.get("QMP").is_none() {
            bail!("unexpected QMP greeting: {greeting}");
        }

        let mut client = Self { reader, writer };

        // Send qmp_capabilities to enter command mode
        let resp = client.execute("qmp_capabilities", Value::Null)?;
        if resp.get("return").is_none() {
            bail!("qmp_capabilities failed: {resp}");
        }

        Ok(client)
    }

    /// Send a QMP command and wait for the response.
    fn execute(&mut self, command: &str, args: Value) -> Result<Value> {
        let mut cmd = serde_json::json!({ "execute": command });
        if !args.is_null() {
            cmd["arguments"] = args;
        }

        let mut msg = serde_json::to_string(&cmd).context("serializing QMP command")?;
        msg.push('\n');

        self.writer
            .write_all(msg.as_bytes())
            .with_context(|| format!("sending QMP command '{command}'"))?;
        self.writer.flush()?;

        // Read lines until we get a response (skip async events)
        loop {
            let mut line = String::new();
            self.reader
                .read_line(&mut line)
                .with_context(|| format!("reading QMP response for '{command}'"))?;

            if line.is_empty() {
                bail!("QMP connection closed while waiting for '{command}' response");
            }

            let json: Value = serde_json::from_str(line.trim())
                .with_context(|| format!("parsing QMP response: {line}"))?;

            // Skip async events (they have "event" key)
            if json.get("event").is_some() {
                continue;
            }

            return Ok(json);
        }
    }

    /// Send a key press via the QEMU human monitor command.
    ///
    /// # Errors
    ///
    /// Returns an error if the QMP command fails.
    pub fn send_key(&mut self, keys: &[&str]) -> Result<()> {
        let key_combo = keys.join("-");
        self.execute(
            "human-monitor-command",
            serde_json::json!({ "command-line": format!("sendkey {key_combo}") }),
        )?;
        Ok(())
    }

    /// Send text as sequential keystrokes.
    ///
    /// # Errors
    ///
    /// Returns an error if any keystroke QMP command fails.
    pub fn send_text(&mut self, text: &str) -> Result<()> {
        for ch in text.chars() {
            let key = char_to_qemu_key(ch);
            self.send_key(&[key])?;
            // Small delay between keystrokes
            std::thread::sleep(Duration::from_millis(50));
        }
        Ok(())
    }

    /// Move mouse to absolute position.
    ///
    /// # Errors
    ///
    /// Returns an error if the QMP command fails.
    pub fn mouse_move(&mut self, x: i32, y: i32) -> Result<()> {
        self.execute(
            "input-send-event",
            serde_json::json!({
                "events": [
                    { "type": "abs", "data": { "axis": "x", "value": x } },
                    { "type": "abs", "data": { "axis": "y", "value": y } },
                ]
            }),
        )?;
        Ok(())
    }

    /// Click a mouse button.
    ///
    /// # Errors
    ///
    /// Returns an error if the QMP command fails.
    pub fn mouse_click(&mut self, button: MouseButton) -> Result<()> {
        let btn = match button {
            MouseButton::Left => "left",
            MouseButton::Right => "right",
            MouseButton::Middle => "middle",
        };
        self.execute(
            "input-send-event",
            serde_json::json!({
                "events": [
                    { "type": "btn", "data": { "down": true, "button": btn } },
                ]
            }),
        )?;
        std::thread::sleep(Duration::from_millis(50));
        self.execute(
            "input-send-event",
            serde_json::json!({
                "events": [
                    { "type": "btn", "data": { "down": false, "button": btn } },
                ]
            }),
        )?;
        Ok(())
    }

    /// Take a screenshot, saving as PPM to the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the QMP command fails.
    pub fn screendump(&mut self, path: &Path) -> Result<()> {
        self.execute(
            "screendump",
            serde_json::json!({ "filename": path.to_string_lossy() }),
        )?;
        Ok(())
    }

    /// Query VM status.
    ///
    /// # Errors
    ///
    /// Returns an error if the QMP command fails or the response is malformed.
    pub fn query_status(&mut self) -> Result<VmStatus> {
        let resp = self.execute("query-status", Value::Null)?;
        let ret = resp
            .get("return")
            .context("missing 'return' in query-status response")?;

        Ok(VmStatus {
            running: ret.get("running").and_then(Value::as_bool).unwrap_or(false),
            status: ret
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string(),
        })
    }

    /// Gracefully quit QEMU.
    ///
    /// # Errors
    ///
    /// Returns an error if the QMP command fails.
    pub fn quit(&mut self) -> Result<()> {
        self.execute("quit", Value::Null)?;
        Ok(())
    }

    /// Try connecting to the socket with retries.
    fn connect_with_retry(path: &Path, timeout: Duration) -> Result<UnixStream> {
        let start = std::time::Instant::now();
        let retry_interval = Duration::from_millis(100);

        loop {
            match UnixStream::connect(path) {
                Ok(stream) => return Ok(stream),
                Err(e) if start.elapsed() < timeout => {
                    std::thread::sleep(retry_interval);
                    if start.elapsed() >= timeout {
                        return Err(e).with_context(|| {
                            format!("connecting to QMP socket {} (timed out)", path.display())
                        });
                    }
                }
                Err(e) => {
                    return Err(e)
                        .with_context(|| format!("connecting to QMP socket {}", path.display()));
                }
            }
        }
    }
}

/// Map a character to a QEMU key name for `sendkey`.
fn char_to_qemu_key(ch: char) -> &'static str {
    match ch {
        'a' => "a",
        'b' => "b",
        'c' => "c",
        'd' => "d",
        'e' => "e",
        'f' => "f",
        'g' => "g",
        'h' => "h",
        'i' => "i",
        'j' => "j",
        'k' => "k",
        'l' => "l",
        'm' => "m",
        'n' => "n",
        'o' => "o",
        'p' => "p",
        'q' => "q",
        'r' => "r",
        's' => "s",
        't' => "t",
        'u' => "u",
        'v' => "v",
        'w' => "w",
        'x' => "x",
        'y' => "y",
        'z' => "z",
        '0' => "0",
        '1' => "1",
        '2' => "2",
        '3' => "3",
        '4' => "4",
        '5' => "5",
        '6' => "6",
        '7' => "7",
        '8' => "8",
        '9' => "9",
        '\n' => "ret",
        '.' => "dot",
        ',' => "comma",
        '-' => "minus",
        '=' => "equal",
        '/' => "slash",
        '\\' => "backslash",
        ';' => "semicolon",
        '\'' => "apostrophe",
        '[' => "bracket_left",
        ']' => "bracket_right",
        _ => "spc", // fallback for unmapped characters (including space)
    }
}
