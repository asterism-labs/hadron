//! Binary wire format for benchmark results.
//!
//! Emits a compact binary format over serial for host-side parsing by gluon.
//!
//! # Format (all little-endian)
//!
//! ```text
//! HEADER (16 bytes):
//!   magic: b"HBENCH\x01\x00" (8B), bench_count: u32, reserved: u32
//!
//! RECORD (variable, per benchmark):
//!   name_len: u16, name: [u8], sample_count: u32, samples: [u64]
//!
//! FOOTER (24 bytes):
//!   tsc_freq_khz: u64, total_nanos: u64, magic: b"HBEND\x01\x00\x00"
//! ```

use crate::serial;

/// HBENCH header magic bytes.
const HEADER_MAGIC: [u8; 8] = *b"HBENCH\x01\x00";

/// HBEND footer magic bytes.
const FOOTER_MAGIC: [u8; 8] = *b"HBEND\x01\x00\x00";

/// Emit the binary header indicating the start of benchmark data.
pub fn emit_header(bench_count: u32) {
    serial::write_bytes(&HEADER_MAGIC);
    serial::write_bytes(&bench_count.to_le_bytes());
    serial::write_bytes(&0u32.to_le_bytes()); // reserved
}

/// Emit a single benchmark record with its name and raw cycle samples.
pub fn emit_record(name: &str, samples: &[u64]) {
    let name_bytes = name.as_bytes();
    let name_len = name_bytes.len().min(u16::MAX as usize) as u16;
    serial::write_bytes(&name_len.to_le_bytes());
    serial::write_bytes(&name_bytes[..name_len as usize]);

    let sample_count = samples.len() as u32;
    serial::write_bytes(&sample_count.to_le_bytes());
    for &sample in samples {
        serial::write_bytes(&sample.to_le_bytes());
    }
}

/// Emit the binary footer with TSC frequency and total elapsed time.
pub fn emit_footer(tsc_freq_khz: u64, total_nanos: u64) {
    serial::write_bytes(&tsc_freq_khz.to_le_bytes());
    serial::write_bytes(&total_nanos.to_le_bytes());
    serial::write_bytes(&FOOTER_MAGIC);
}
