//! The 8254 programmable interval timer, used only as a stopwatch to
//! calibrate the local APIC timer.
//!
//! The kernel uses channel 2, whose gate and output are wired to port
//! 0x61 on PCs. So it can start the count and poll for the end without an
//! interrupt.

use super::port;
use crate::percpu::PreemptGuard;

/// The PIT's input clock.
const FREQUENCY_HZ: u64 = 1_193_182;
/// Channel 2's count register.
const CHANNEL_2: u16 = 0x42;
/// The mode/command register.
const COMMAND: u16 = 0x43;
/// Port 0x61, which holds channel 2's gate and output.
const GATE_PORT: u16 = 0x61;
/// Port 0x61 bit 0: channel 2 counts while it is set.
const GATE_2: u8 = 1 << 0;
/// Port 0x61 bit 1: channel 2 drives the speaker.
const SPEAKER: u8 = 1 << 1;
/// Port 0x61 bit 5: channel 2's output.
const OUT_2: u8 = 1 << 5;
/// Command `0b1011_0000`: channel 2 (bits 6-7), low byte then high byte
/// (bits 4-5), mode 0, interrupt on terminal count (bits 1-3), binary
/// (bit 0).
const CHANNEL_2_MODE_0: u8 = 0b1011_0000;
/// The longest wait one 16-bit count allows.
pub(crate) const MAX_WAIT_MS: u32 = 50;

/// Busy-waits `ms` milliseconds. Calls `start` right before the count
/// starts, so the caller can start its own clock.
///
/// Interrupts stay on, but preemption is off while it waits, so no other
/// thread on this CPU uses the PIT at the same time. There is one CPU.
///
/// # Panics
///
/// Panics if `ms` is above [`MAX_WAIT_MS`].
pub(crate) fn wait_ms(ms: u32, start: impl FnOnce()) {
    assert!(ms <= MAX_WAIT_MS, "the PIT waits at most {MAX_WAIT_MS} ms");
    #[allow(clippy::cast_possible_truncation)] // at most 59659
    let count = (FREQUENCY_HZ * u64::from(ms) / 1000) as u16;
    let [low, high] = count.to_le_bytes();
    let _preempt = PreemptGuard::new();
    // SAFETY: ports 0x42, 0x43 and 0x61 belong to the PIT and the keyboard
    // controller's port B on PCs. Only this function touches channel 2,
    // and it changes no other bit of port 0x61.
    unsafe {
        let other = port::inb(GATE_PORT) & !(GATE_2 | SPEAKER);
        port::outb(GATE_PORT, other);
        // In mode 0, the output goes low now and high when the count ends.
        port::outb(COMMAND, CHANNEL_2_MODE_0);
        port::outb(CHANNEL_2, low);
        port::outb(CHANNEL_2, high);
        start();
        port::outb(GATE_PORT, other | GATE_2);
        while port::inb(GATE_PORT) & OUT_2 == 0 {
            core::hint::spin_loop();
        }
        port::outb(GATE_PORT, other);
    }
}
