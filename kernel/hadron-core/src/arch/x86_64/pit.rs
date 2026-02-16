//! 8254 PIT (Programmable Interval Timer) driver.
//!
//! Used only for LAPIC timer calibration via channel 2 one-shot mode.

use super::Port;

/// PIT oscillator frequency: 1,193,182 Hz.
const PIT_FREQUENCY: u32 = 1_193_182;

const CHANNEL2_DATA: u16 = 0x42;
const PIT_CMD: u16 = 0x43;
/// Port B (NMI status and speaker control).
const PORT_B: u16 = 0x61;

/// Busy-waits for approximately `ms` milliseconds using PIT channel 2.
///
/// # Safety
///
/// Must be called with interrupts disabled. The PIT must not be in use
/// by other code.
pub unsafe fn busy_wait_ms(ms: u32) {
    let count = (PIT_FREQUENCY * ms) / 1000;
    // Clamp to u16 max for the PIT counter.
    let count = if count > 0xFFFF { 0xFFFF } else { count as u16 };

    let channel2 = Port::<u8>::new(CHANNEL2_DATA);
    let cmd = Port::<u8>::new(PIT_CMD);
    let port_b = Port::<u8>::new(PORT_B);

    // SAFETY: All port accesses are to well-known PIT and port B registers.
    // The caller guarantees interrupts are disabled and the PIT is not in use.
    unsafe {
        // Enable PIT channel 2 gate (bit 0 of port 0x61).
        let b = port_b.read();
        // Disable speaker (bit 1 = 0), enable gate (bit 0 = 1).
        port_b.write((b & !0x02) | 0x01);

        // Channel 2, lobyte/hibyte, one-shot (mode 0), binary.
        cmd.write(0b1011_0000);

        // Load count.
        channel2.write(count as u8);
        channel2.write((count >> 8) as u8);

        // Reset the flip-flop: read port B, clear bit 5 (OUT2), then set gate.
        let b = port_b.read();
        port_b.write(b & !0x01); // Gate low
        port_b.write(b | 0x01); // Gate high (starts counting)

        // Wait for OUT2 (bit 5 of port 0x61) to go high.
        loop {
            if port_b.read() & 0x20 != 0 {
                break;
            }
            core::hint::spin_loop();
        }
    }
}
