//! TTY subsystem.
//!
//! Provides virtual terminal (VT) abstractions backed by a line discipline
//! and keyboard input. Each [`Tty`] owns a [`LineDiscipline`](ldisc::LineDiscipline)
//! for cooked-mode editing, a per-VT foreground process group, and a waker
//! slot for async reader notification.
//!
//! The keyboard IRQ handler feeds scancodes into the active TTY's line
//! discipline; userspace reads go through [`DevTty`](device::DevTty) inodes
//! registered in devfs.

pub mod device;
pub mod ldisc;

use core::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use core::task::Waker;

use crate::sync::IrqSpinLock;
use ldisc::{LdiscAction, LineDiscipline};
use planck_noalloc::ringbuf::RingBuf;

/// Maximum number of virtual terminals.
pub const MAX_TTYS: usize = 6;

/// Capacity of the shared scancode ring buffer filled by the keyboard IRQ.
const SCANCODE_BUF_SIZE: usize = 64;

/// Index of the currently active virtual terminal in [`TTY_TABLE`].
static ACTIVE_VT: AtomicUsize = AtomicUsize::new(0);

/// Scancode ring buffer filled by the keyboard IRQ handler.
///
/// The IRQ handler drains the PS/2 hardware FIFO into this buffer.
/// [`Tty::poll_hardware`] on the active TTY processes scancodes from here.
static SCANCODE_BUF: IrqSpinLock<RingBuf<u8, SCANCODE_BUF_SIZE>> =
    IrqSpinLock::leveled("TTY_SCANCODE", 10, RingBuf::new());

/// Global TTY table. Slots are initialized lazily by [`init`].
static TTY_TABLE: [Tty; MAX_TTYS] = [const { Tty::new() }; MAX_TTYS];

/// A virtual terminal.
///
/// Each TTY owns a line discipline for cooked-mode editing, a foreground
/// process group ID for signal delivery (Ctrl+C → SIGINT), and a waker
/// slot for notifying a blocked reader when input arrives.
pub struct Tty {
    /// Line discipline handling scancode processing and line editing.
    ldisc: IrqSpinLock<LineDiscipline>,
    /// Process group that receives terminal signals (SIGINT, SIGQUIT, etc.).
    foreground_pgid: AtomicU32,
    /// Single-waker slot for the reader future.
    input_waker: IrqSpinLock<Option<Waker>>,
}

// SAFETY: Tty is Sync because all mutable state is behind IrqSpinLock or atomic.
unsafe impl Sync for Tty {}

impl Tty {
    /// Create a new uninitialized TTY.
    const fn new() -> Self {
        Self {
            ldisc: IrqSpinLock::leveled("TTY_LDISC", 10, LineDiscipline::new()),
            foreground_pgid: AtomicU32::new(0),
            input_waker: IrqSpinLock::named("TTY_WAKER", None),
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
    fn wake(&self) {
        if let Some(w) = self.input_waker.lock().take() {
            w.wake();
        }
    }

    /// Drain buffered scancodes, decode them, and push into the line discipline.
    ///
    /// Processes scancodes from two sources:
    /// 1. The shared [`SCANCODE_BUF`] ring buffer (filled by the IRQ handler)
    /// 2. The PS/2 hardware FIFO directly (catches bytes from before IRQ setup)
    ///
    /// Echo output is deferred until after the ldisc lock is released to avoid
    /// holding an IRQ-disabling lock while writing to the logger.
    pub fn poll_hardware(&self) {
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

        // Phase 2: process under ldisc lock, collect actions.
        let mut actions: [Option<LdiscAction>; SCANCODE_BUF_SIZE] =
            [const { None }; SCANCODE_BUF_SIZE];
        let mut action_count = 0;
        {
            let mut ldisc = self.ldisc.lock();
            for &scancode in &raw[..count] {
                if let Some(action) = ldisc.process_scancode(scancode) {
                    actions[action_count] = Some(action);
                    action_count += 1;
                }
            }
        }
        // ldisc released — interrupts re-enabled.

        // Phase 3: echo and signal delivery with no locks held.
        for action in &actions[..action_count] {
            match action {
                Some(LdiscAction::Backspace) => crate::kprint!("\x08 \x08"),
                Some(LdiscAction::Newline) => crate::kprint!("\n"),
                Some(LdiscAction::Char(ch)) => crate::kprint!("{}", *ch as char),
                Some(LdiscAction::Interrupt) => {
                    crate::kprint!("^C\n");
                    // Send SIGINT to the foreground process group.
                    if let Some(fg_pgid) = self.foreground_pgid() {
                        crate::proc::signal_process_group(fg_pgid, crate::syscall::SIGINT);
                    }
                }
                Some(LdiscAction::Eof) | Some(LdiscAction::FlushLine) => {}
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
    // Wake the active TTY's reader.
    let vt = ACTIVE_VT.load(Ordering::Acquire);
    TTY_TABLE[vt].wake();
}

// ── Public API ───────────────────────────────────────────────────────

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

/// Initialize the TTY subsystem.
///
/// Registers the keyboard IRQ1 handler and unmasks it in the I/O APIC.
/// Must be called after APIC initialization.
pub fn init() {
    use crate::arch::x86_64::interrupts::dispatch;

    let vector = dispatch::vectors::isa_irq_vector(1);
    dispatch::register_handler(vector, keyboard_irq_handler)
        .expect("tty: failed to register keyboard IRQ handler");

    crate::arch::x86_64::acpi::with_io_apic(|ioapic| ioapic.unmask(1));

    crate::kinfo!("TTY: keyboard IRQ1 enabled (vector {}), tty0 active", vector);
}
