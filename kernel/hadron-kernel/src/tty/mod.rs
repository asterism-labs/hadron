//! TTY subsystem.
//!
//! Provides virtual terminal (VT) abstractions backed by a line discipline,
//! keyboard input, and per-VT framebuffer console instances. Each [`Tty`] owns
//! a [`LineDiscipline`](ldisc::LineDiscipline) for cooked-mode editing, an
//! optional [`FbCon`](crate::drivers::fbcon::FbCon) for display output, a
//! per-VT foreground process group, and a waker slot for async reader
//! notification.
//!
//! The keyboard IRQ handler feeds scancodes into the active TTY's line
//! discipline; userspace reads go through [`DevTty`](device::DevTty) inodes
//! registered in devfs.
//!
//! **VT switching:** Alt+F1..F6 switches the active VT. Only the active VT's
//! fbcon renders to the physical framebuffer; inactive VTs update their cell
//! buffer silently and redraw when reactivated.

extern crate alloc;

pub mod device;
pub mod ldisc;
pub mod pty;

use alloc::sync::Arc;
use hadron_core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::task::Waker;

use crate::drivers::fbcon::FbCon;
use crate::sync::{IrqSpinLock, SpinLock};
use hadron_syscall::{ECHO, ICANON, ISIG, Termios};
use ldisc::{LdiscAction, LineDiscipline};
use planck_noalloc::ringbuf::RingBuf;

/// Maximum number of virtual terminals.
pub const MAX_TTYS: usize = 6;

/// Capacity of the shared scancode ring buffer filled by the keyboard IRQ.
const SCANCODE_BUF_SIZE: usize = 64;

/// Index of the currently active virtual terminal in [`TTY_TABLE`].
static ACTIVE_VT: AtomicUsize = AtomicUsize::new(0);

/// Set by the keyboard IRQ handler when scancodes are buffered.
/// Checked by the syscall return path to process input during tight
/// syscall loops (e.g. `yes` writing in a loop) where the normal
/// TTY read path never runs.
static SCANCODES_PENDING: AtomicBool = AtomicBool::new(false);

/// Scancode ring buffer filled by the keyboard IRQ handler.
///
/// The IRQ handler drains the PS/2 hardware FIFO into this buffer.
/// [`Tty::poll_hardware`] on the active TTY processes scancodes from here.
static SCANCODE_BUF: IrqSpinLock<RingBuf<u8, SCANCODE_BUF_SIZE>> =
    IrqSpinLock::leveled("TTY_SCANCODE", 10, RingBuf::new());

/// Default termios: canonical mode with echo and signal generation.
const fn default_termios() -> Termios {
    let mut cc = [0u8; 32];
    cc[hadron_syscall::VINTR] = 0x03; // Ctrl+C
    cc[hadron_syscall::VQUIT] = 0x1C; // Ctrl+backslash
    cc[hadron_syscall::VERASE] = 0x7F; // DEL
    cc[hadron_syscall::VKILL] = 0x15; // Ctrl+U
    cc[hadron_syscall::VEOF] = 0x04; // Ctrl+D
    cc[hadron_syscall::VMIN] = 1;
    cc[hadron_syscall::VTIME] = 0;
    Termios {
        iflag: hadron_syscall::ICRNL,
        oflag: hadron_syscall::OPOST | hadron_syscall::ONLCR,
        cflag: 0,
        lflag: hadron_syscall::ICANON | hadron_syscall::ECHO | hadron_syscall::ISIG,
        cc,
    }
}

/// Global TTY table. Slots are initialized lazily by [`init`].
static TTY_TABLE: [Tty; MAX_TTYS] = [const { Tty::new() }; MAX_TTYS];

/// A virtual terminal.
///
/// Each TTY owns a line discipline for cooked-mode editing, a foreground
/// process group ID for signal delivery (Ctrl+C → SIGINT), an optional
/// framebuffer console for display output, and a waker slot for notifying
/// a blocked reader when input arrives.
pub struct Tty {
    /// Line discipline handling scancode processing and line editing.
    ldisc: IrqSpinLock<LineDiscipline>,
    /// Process group that receives terminal signals (SIGINT, SIGQUIT, etc.).
    foreground_pgid: AtomicU32,
    /// Single-waker slot for the reader future.
    input_waker: IrqSpinLock<Option<Waker>>,
    /// Per-VT framebuffer console (set during boot, then read-only).
    fbcon: SpinLock<Option<Arc<FbCon>>>,
    /// Terminal I/O settings (termios).
    termios: IrqSpinLock<Termios>,
}

// SAFETY: Tty is Sync because all mutable state is behind IrqSpinLock,
// SpinLock, or atomics.
unsafe impl Sync for Tty {}

impl Tty {
    /// Create a new uninitialized TTY.
    const fn new() -> Self {
        Self {
            ldisc: IrqSpinLock::leveled("TTY_LDISC", 10, LineDiscipline::new()),
            foreground_pgid: AtomicU32::new(0),
            input_waker: IrqSpinLock::named("TTY_WAKER", None),
            fbcon: SpinLock::named("TTY_FBCON", None),
            termios: IrqSpinLock::named("TTY_TERMIOS", default_termios()),
        }
    }

    /// Get the current termios settings.
    pub fn get_termios(&self) -> Termios {
        *self.termios.lock()
    }

    /// Set the termios settings.
    pub fn set_termios(&self, t: &Termios) {
        *self.termios.lock() = *t;
    }

    /// Get the window size from the attached framebuffer console.
    pub fn get_winsize(&self) -> hadron_syscall::Winsize {
        let fbcon = self.fbcon.lock().clone();
        if let Some(ref fbcon) = fbcon {
            let (cols, rows) = fbcon.dimensions();
            hadron_syscall::Winsize {
                rows: rows as u16,
                cols: cols as u16,
                xpixel: 0,
                ypixel: 0,
            }
        } else {
            hadron_syscall::Winsize {
                rows: 25,
                cols: 80,
                xpixel: 0,
                ypixel: 0,
            }
        }
    }

    /// Attach a framebuffer console to this TTY.
    ///
    /// Called once during boot to give each VT its own display output.
    pub fn set_fbcon(&self, fbcon: Arc<FbCon>) {
        *self.fbcon.lock() = Some(fbcon);
    }

    /// Write output to this TTY's framebuffer console.
    ///
    /// If no fbcon is attached, the output is silently discarded.
    /// Clones the `Arc<FbCon>` so the TTY fbcon lock is released before the
    /// (potentially slow) write — matching the pattern used by [`switch_vt`].
    pub fn write_output(&self, s: &str) {
        let fbcon = self.fbcon.lock().clone();
        if let Some(ref fbcon) = fbcon {
            fbcon.write_str(s);
        }
    }

    /// Set the foreground process group ID for this TTY.
    pub fn set_foreground_pgid(&self, pgid: u32) {
        self.foreground_pgid.store(pgid, Ordering::Release);
    }

    /// Get the foreground process group ID, or `None` if none is set.
    pub fn foreground_pgid(&self) -> Option<u32> {
        let raw = self.foreground_pgid.load(Ordering::Acquire);
        if raw == 0 { None } else { Some(raw) }
    }

    /// Register a waker to be notified when input arrives.
    pub fn subscribe(&self, waker: &Waker) {
        *self.input_waker.lock() = Some(waker.clone());
    }

    /// Wake the registered reader (called from IRQ context or after processing).
    ///
    /// Takes the waker out of the lock before invoking it so the
    /// `input_waker` IrqSpinLock is not held while `waker.wake()` acquires
    /// the executor's `ready_queues` IrqSpinLock. This matches the pattern
    /// used by [`HeapWaitQueue`](crate::sync::HeapWaitQueue).
    fn wake(&self) {
        let waker = self.input_waker.lock().take();
        if let Some(w) = waker {
            w.wake();
        }
    }

    /// Drain buffered scancodes, decode them, and push into the line discipline.
    ///
    /// Processes scancodes from two sources:
    /// 1. The shared [`SCANCODE_BUF`] ring buffer (filled by the IRQ handler)
    /// 2. The PS/2 hardware FIFO directly (catches bytes from before IRQ setup)
    ///
    /// Only the active VT drains scancodes — inactive VTs return immediately
    /// to avoid stealing input from the active terminal.
    ///
    /// Echo output is deferred until after the ldisc lock is released to avoid
    /// holding an IRQ-disabling lock while writing to the logger.
    pub fn poll_hardware(&self) {
        // Only the active VT should drain the shared keyboard scancode buffer.
        let active_vt = ACTIVE_VT.load(Ordering::Acquire);
        if !core::ptr::eq(self, &TTY_TABLE[active_vt]) {
            return;
        }

        // Phase 1: drain scancodes (interrupts disabled briefly).
        let mut raw = [0u8; SCANCODE_BUF_SIZE];
        let mut count = 0;

        {
            let mut buf = SCANCODE_BUF.lock();

            // Drain the IRQ-buffered scancodes.
            while let Some(sc) = buf.pop() {
                if count < raw.len() {
                    raw[count] = sc;
                    count += 1;
                }
            }

            // Also drain hardware FIFO directly.
            while let Some(sc) = try_read_keyboard_scancode() {
                if count < raw.len() {
                    raw[count] = sc;
                    count += 1;
                }
            }
        }

        if count == 0 {
            return;
        }

        crate::kdebug!(
            "TTY: tty{} received {} scancode(s): {:02x?}",
            active_vt,
            count,
            &raw[..count]
        );

        // Snapshot termios flags before taking the ldisc lock.
        let termios = *self.termios.lock();
        let icanon = termios.lflag & ICANON != 0;
        let isig = termios.lflag & ISIG != 0;
        let echo = termios.lflag & ECHO != 0;

        // Phase 2: process under ldisc lock, collect actions.
        let mut actions: [Option<LdiscAction>; SCANCODE_BUF_SIZE] =
            [const { None }; SCANCODE_BUF_SIZE];
        let mut action_count = 0;
        {
            let mut ldisc = self.ldisc.lock();
            for &scancode in &raw[..count] {
                if let Some(action) = ldisc.process_scancode(scancode, icanon, isig) {
                    actions[action_count] = Some(action);
                    action_count += 1;
                }
            }
        }
        // ldisc released — interrupts re-enabled.

        // Phase 3: echo and signal delivery with no locks held.
        // Echo output is gated on the ECHO termios flag, except for ^C
        // which is always echoed (matches Linux behavior).
        for action in &actions[..action_count] {
            match action {
                Some(LdiscAction::Backspace) => {
                    if echo {
                        crate::kprint!("\x08 \x08");
                    }
                }
                Some(LdiscAction::Newline) => {
                    if echo {
                        crate::kprint!("\n");
                    }
                }
                Some(LdiscAction::Char(ch)) => {
                    if echo {
                        crate::kprint!("{}", *ch as char);
                    }
                }
                Some(LdiscAction::Interrupt) => {
                    crate::kprint!("^C\n");
                    // Send SIGINT to the foreground process group.
                    if let Some(fg_pgid) = self.foreground_pgid() {
                        crate::proc::signal_process_group(fg_pgid, crate::syscall::SIGINT);
                    }
                }
                Some(LdiscAction::Eof) | Some(LdiscAction::FlushLine) => {}
                Some(LdiscAction::SwitchVt(vt)) => {
                    switch_vt(*vt);
                }
                None => {}
            }
        }
    }

    /// Non-blocking read from this TTY's line discipline.
    ///
    /// Returns `Some(n)` with data, `Some(0)` for EOF, or `None` for no data.
    pub fn try_read(&self, buf: &mut [u8]) -> Option<usize> {
        self.ldisc.lock().try_read(buf)
    }

    /// Check if the line discipline has data available for reading.
    ///
    /// Non-destructive: does not consume any data.
    pub fn has_input(&self) -> bool {
        self.ldisc.lock().has_data()
    }
}

// ── VT switching ─────────────────────────────────────────────────────

/// Switch the active virtual terminal.
///
/// Deactivates the old VT's fbcon (stops pixel rendering), activates the
/// new VT's fbcon, and redraws its cell buffer to the physical framebuffer.
fn switch_vt(new_vt: usize) {
    if new_vt >= MAX_TTYS {
        return;
    }

    let old = ACTIVE_VT.swap(new_vt, Ordering::AcqRel);
    if old == new_vt {
        return;
    }

    // Clone Arc references to avoid holding both TTY_FBCON locks simultaneously.
    let old_fbcon = TTY_TABLE[old].fbcon.lock().clone();
    let new_fbcon = TTY_TABLE[new_vt].fbcon.lock().clone();

    if let Some(fbcon) = old_fbcon {
        fbcon.set_active(false);
    }
    if let Some(fbcon) = new_fbcon {
        fbcon.set_active(true);
        fbcon.redraw_all();
    }

    // Wake the new VT's reader in case it's blocked waiting for input.
    TTY_TABLE[new_vt].wake();

    crate::kinfo!("VT: switched to tty{}", new_vt);
}

// ── Hardware helpers ─────────────────────────────────────────────────

/// Reads a keyboard scancode from the PS/2 data port, if available.
#[cfg(target_arch = "x86_64")]
fn try_read_keyboard_scancode() -> Option<u8> {
    use crate::arch::x86_64::Port;
    // SAFETY: Reading status and data ports is a standard PS/2 operation.
    let status = unsafe { Port::<u8>::new(0x64).read() };
    // Bit 0: output buffer full, bit 5: mouse data.
    if status & 0x01 != 0 && status & 0x20 == 0 {
        Some(unsafe { Port::<u8>::new(0x60).read() })
    } else {
        None
    }
}

// ── IRQ handler ──────────────────────────────────────────────────────

/// Keyboard IRQ handler — drains the PS/2 FIFO and wakes the active TTY.
fn keyboard_irq_handler(_vector: crate::id::IrqVector) {
    {
        let mut buf = SCANCODE_BUF.lock();
        while let Some(scancode) = try_read_keyboard_scancode() {
            let _ = buf.try_push(scancode);
        }
    }
    // Signal that scancodes are available for processing.
    SCANCODES_PENDING.store(true, Ordering::Release);
    // Wake the active TTY's reader.
    let vt = ACTIVE_VT.load(Ordering::Acquire);
    TTY_TABLE[vt].wake();
}

// ── Public API ───────────────────────────────────────────────────────

/// Process any pending keyboard input on the active TTY.
///
/// Called from the syscall return path so that Ctrl+C / Alt+Fn are
/// recognised even when no process is reading from the terminal (e.g.
/// during a tight `yes` write loop). The fast path is a single atomic
/// load when no scancodes are pending.
pub fn process_pending_input() {
    if SCANCODES_PENDING.swap(false, Ordering::AcqRel) {
        active_tty().poll_hardware();
    }
}

/// Returns a reference to the active TTY.
pub fn active_tty() -> &'static Tty {
    let vt = ACTIVE_VT.load(Ordering::Acquire);
    &TTY_TABLE[vt]
}

/// Returns a reference to a specific TTY by index.
pub fn tty(index: usize) -> Option<&'static Tty> {
    if index < MAX_TTYS {
        Some(&TTY_TABLE[index])
    } else {
        None
    }
}

/// Attach a framebuffer console to a specific VT.
///
/// Called during boot to give each VT its own display output.
pub fn set_vt_fbcon(index: usize, fbcon: Arc<FbCon>) {
    if index < MAX_TTYS {
        TTY_TABLE[index].set_fbcon(fbcon);
    }
}

/// Write a string to the active VT's framebuffer console.
///
/// Used by [`VtConsoleSink`] to route kernel log output to the current display.
pub fn write_active_vt(s: &str) {
    let vt = ACTIVE_VT.load(Ordering::Acquire);
    TTY_TABLE[vt].write_output(s);
}

/// Initialize the TTY subsystem.
///
/// Registers the keyboard IRQ1 handler and unmasks it in the I/O APIC.
/// Must be called after APIC initialization.
pub fn init() {
    use crate::arch::x86_64::interrupts::dispatch;

    let vector = dispatch::vectors::isa_irq_vector(1);
    dispatch::register_handler(vector, keyboard_irq_handler)
        .expect("tty: failed to register keyboard IRQ handler");

    crate::arch::x86_64::acpi::Acpi::with_io_apic(|ioapic| ioapic.unmask(1));

    crate::kinfo!(
        "TTY: keyboard IRQ1 enabled (vector {}), tty0 active",
        vector
    );
}

// ── VtConsoleSink — LogSink for active VT ────────────────────────────

/// A [`LogSink`](crate::log::LogSink) that routes output to the active VT's
/// framebuffer console.
///
/// Replaces the per-FbCon [`FbConSink`](crate::drivers::fbcon::FbConSink) in
/// the global logger so that kernel messages always appear on the active VT.
pub struct VtConsoleSink {
    /// Maximum log level accepted by this sink.
    max_level: crate::log::LogLevel,
}

impl VtConsoleSink {
    /// Creates a new VT console sink.
    pub fn new(max_level: crate::log::LogLevel) -> Self {
        Self { max_level }
    }
}

impl crate::log::LogSink for VtConsoleSink {
    fn write_str(&self, s: &str) {
        write_active_vt(s);
    }

    fn max_level(&self) -> crate::log::LogLevel {
        self.max_level
    }

    fn name(&self) -> &str {
        "framebuffer"
    }
}
