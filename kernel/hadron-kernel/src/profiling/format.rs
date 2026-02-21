//! HPRF binary format emission for profiling data.
//!
//! Shared between the sampling profiler and function tracer. Emits
//! records to serial in the HPRF format consumed by gluon's `perf` module.
//!
//! Format (all little-endian):
//!
//! ```text
//! HEADER (32 bytes):
//!   magic: b"HPRF" (4B), version: u16, flags: u16
//!   tsc_freq_hz: u64, kernel_vbase: u64, cpu_count: u32, reserved: u32
//!
//! RECORDS (variable, tagged):
//!   Type 0x01 (Sample): cpu_id:u8, depth:u16, reserved:u8, pad:u32,
//!                        tsc:u64, stack:[u64;depth]
//!   Type 0x02 (FtraceEntry): cpu_id:u8, reserved:u16, pad:u8, pad32:u32,
//!                             tsc:u64, func_addr:u64
//!   Type 0xFF (EndOfStream): 7 reserved bytes
//! ```

use crate::drivers::early_console::EarlySerial;

/// COM1 serial port used for binary emission.
const SERIAL: EarlySerial = EarlySerial::new(crate::drivers::early_console::COM1);

/// HPRF format version.
const HPRF_VERSION: u16 = 1;

/// Record type: profiling sample with stack trace.
const RECORD_SAMPLE: u8 = 0x01;

/// Record type: function trace entry.
#[allow(dead_code)] // used when ftrace is wired up
const RECORD_FTRACE: u8 = 0x02;

/// Record type: end of stream marker.
const RECORD_END: u8 = 0xFF;

/// Flag bit: stream contains sampling profiler data.
pub const FLAG_SAMPLES: u16 = 1 << 0;

/// Flag bit: stream contains ftrace data.
#[allow(dead_code)] // used when ftrace is wired up
pub const FLAG_FTRACE: u16 = 1 << 1;

// ---------------------------------------------------------------------------
// Serial binary output helpers
// ---------------------------------------------------------------------------

/// Write raw bytes to COM1 serial for binary data emission.
fn emit_bytes(data: &[u8]) {
    for &byte in data {
        SERIAL.write_byte(byte);
    }
}

fn emit_u8(v: u8) {
    emit_bytes(&[v]);
}

fn emit_u16(v: u16) {
    emit_bytes(&v.to_le_bytes());
}

fn emit_u32(v: u32) {
    emit_bytes(&v.to_le_bytes());
}

fn emit_u64(v: u64) {
    emit_bytes(&v.to_le_bytes());
}

// ---------------------------------------------------------------------------
// HPRF emission
// ---------------------------------------------------------------------------

/// Emit the HPRF header.
///
/// `tsc_freq_hz` should be the calibrated TSC frequency. Pass 0 if unknown.
pub fn emit_header(flags: u16, tsc_freq_hz: u64, kernel_vbase: u64, cpu_count: u32) {
    emit_bytes(b"HPRF");       // magic (4B)
    emit_u16(HPRF_VERSION);    // version (2B)
    emit_u16(flags);           // flags (2B)
    emit_u64(tsc_freq_hz);    // tsc_freq_hz (8B)
    emit_u64(kernel_vbase);   // kernel_vbase (8B)
    emit_u32(cpu_count);      // cpu_count (4B)
    emit_u32(0);               // reserved (4B)
    // Total: 32 bytes
}

/// Emit a sample record.
pub fn emit_sample_record(cpu_id: u8, tsc_val: u64, stack: &[u64], depth: u16) {
    emit_u8(RECORD_SAMPLE);   // record type (1B)
    emit_u8(cpu_id);          // cpu_id (1B)
    emit_u16(depth);          // depth (2B)
    emit_u32(0);               // reserved+padding (4B)
    emit_u64(tsc_val);        // tsc (8B)
    for i in 0..depth as usize {
        if i < stack.len() {
            emit_u64(stack[i]);
        } else {
            emit_u64(0);
        }
    }
}

/// Emit a function trace entry record.
#[allow(dead_code)] // used when ftrace is wired up
pub fn emit_ftrace_record(cpu_id: u8, tsc_val: u64, func_addr: u64) {
    emit_u8(RECORD_FTRACE);   // record type (1B)
    emit_u8(cpu_id);          // cpu_id (1B)
    emit_u16(0);               // reserved (2B)
    emit_u32(0);               // padding (4B)
    emit_u64(tsc_val);        // tsc (8B)
    emit_u64(func_addr);      // func_addr (8B)
}

/// Emit the end-of-stream marker.
pub fn emit_end_of_stream() {
    emit_u8(RECORD_END);      // record type (1B)
    emit_bytes(&[0; 7]);      // reserved (7B)
}
