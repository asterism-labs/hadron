//! Integration test: emit raw HPRF binary data over serial.
//!
//! Writes a minimal but valid HPRF stream (header + 2 sample records +
//! end-of-stream marker) to COM1 using direct port I/O. This test
//! verifies the full pipeline: kernel emits HPRF → serial capture →
//! `gluon perf report` can parse the output.
//!
//! Uses `test_entry_point!()` — no full kernel init needed, just serial
//! and QEMU exit.

#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(hadron_test::test_runner)]
#![reexport_test_harness_main = "test_main"]

hadron_test::test_entry_point!();

// ---------------------------------------------------------------------------
// Raw COM1 byte output (bypasses serial_println! text translation)
// ---------------------------------------------------------------------------

const COM1: u16 = 0x3F8;

/// Write a single byte to COM1, polling the LSR for TX-ready first.
fn serial_write_byte(byte: u8) {
    unsafe {
        // Wait for transmit holding register empty (bit 5 of LSR).
        loop {
            let lsr: u8;
            core::arch::asm!(
                "in al, dx",
                in("dx") COM1 + 5,
                out("al") lsr,
                options(nomem, nostack, preserves_flags),
            );
            if lsr & 0x20 != 0 {
                break;
            }
        }
        core::arch::asm!(
            "out dx, al",
            in("dx") COM1,
            in("al") byte,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Write a slice of bytes to COM1.
fn serial_write_bytes(data: &[u8]) {
    for &b in data {
        serial_write_byte(b);
    }
}

fn serial_write_u16(v: u16) {
    serial_write_bytes(&v.to_le_bytes());
}

fn serial_write_u32(v: u32) {
    serial_write_bytes(&v.to_le_bytes());
}

fn serial_write_u64(v: u64) {
    serial_write_bytes(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// HPRF payload construction + emission
// ---------------------------------------------------------------------------

/// Emit a complete HPRF stream: header + 2 sample records + end marker.
fn emit_hprf_stream() {
    // --- Header (32 bytes) ---
    serial_write_bytes(b"HPRF"); // magic (4B)
    serial_write_u16(1); // version (2B)
    serial_write_u16(1); // flags: FLAG_SAMPLES (2B)
    serial_write_u64(1_000_000); // tsc_freq_hz (8B)
    serial_write_u64(0xFFFF_8000_0000_0000); // kernel_vbase (8B)
    serial_write_u32(1); // cpu_count (4B)
    serial_write_u32(0); // reserved (4B)

    // --- Sample record 1 (depth=2) ---
    serial_write_byte(0x01); // record type (1B)
    serial_write_byte(0); // cpu_id (1B)
    serial_write_u16(2); // depth (2B)
    serial_write_u32(0); // reserved+padding (4B)
    serial_write_u64(42); // tsc (8B)
    serial_write_u64(0xFFFF_8000_0010_0000); // stack[0] (8B)
    serial_write_u64(0xFFFF_8000_0020_0000); // stack[1] (8B)

    // --- Sample record 2 (depth=1) ---
    serial_write_byte(0x01); // record type (1B)
    serial_write_byte(0); // cpu_id (1B)
    serial_write_u16(1); // depth (2B)
    serial_write_u32(0); // reserved+padding (4B)
    serial_write_u64(100); // tsc (8B)
    serial_write_u64(0xFFFF_8000_0030_0000); // stack[0] (8B)

    // --- End-of-stream marker (8 bytes) ---
    serial_write_byte(0xFF); // record type (1B)
    serial_write_bytes(&[0; 7]); // reserved (7B)
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[test_case]
fn emit_hprf_binary() {
    emit_hprf_stream();
}
