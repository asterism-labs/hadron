//! Binary format deserialization for HBENCH and HPRF formats.
//!
//! Parses the compact binary data emitted by the kernel benchmark harness
//! and sampling profiler over serial.

use anyhow::{Result, bail};

/// HBENCH header magic.
const HBENCH_MAGIC: &[u8; 8] = b"HBENCH\x01\x00";
/// HBEND footer magic.
const HBEND_MAGIC: &[u8; 8] = b"HBEND\x01\x00\x00";

/// HPRF header magic.
const HPRF_MAGIC: &[u8; 4] = b"HPRF";

/// Parsed benchmark results from the HBENCH binary format.
#[derive(Debug)]
pub struct HBenchResults {
    /// Individual benchmark records.
    pub records: Vec<BenchRecord>,
    /// TSC frequency in kHz.
    pub tsc_freq_khz: u64,
    /// Total elapsed time in nanoseconds.
    pub total_nanos: u64,
}

/// A single benchmark record with name and raw cycle samples.
#[derive(Debug)]
pub struct BenchRecord {
    /// Benchmark name.
    pub name: String,
    /// Raw cycle samples.
    pub samples: Vec<u64>,
}

/// Parse HBENCH binary format from serial data.
///
/// Scans for the HBENCH header magic, then reads records and footer.
/// The serial stream may contain text output before the binary data.
pub fn parse_hbench(data: &[u8]) -> Result<HBenchResults> {
    // Find the header magic in the serial stream.
    let header_pos = find_magic(data, HBENCH_MAGIC)
        .ok_or_else(|| anyhow::anyhow!("HBENCH header magic not found in serial data"))?;

    let mut pos = header_pos + 8;

    // Header: bench_count (u32) + reserved (u32).
    if pos + 8 > data.len() {
        bail!("HBENCH header truncated");
    }
    let bench_count = read_u32(data, &mut pos);
    let _reserved = read_u32(data, &mut pos);

    // Records.
    let mut records = Vec::with_capacity(bench_count as usize);
    for _ in 0..bench_count {
        if pos + 2 > data.len() {
            bail!("HBENCH record truncated (name_len)");
        }
        let name_len = read_u16(data, &mut pos) as usize;

        if pos + name_len > data.len() {
            bail!("HBENCH record truncated (name)");
        }
        let name = String::from_utf8_lossy(&data[pos..pos + name_len]).to_string();
        pos += name_len;

        if pos + 4 > data.len() {
            bail!("HBENCH record truncated (sample_count)");
        }
        let sample_count = read_u32(data, &mut pos) as usize;

        if pos + sample_count * 8 > data.len() {
            bail!("HBENCH record truncated (samples)");
        }
        let mut samples = Vec::with_capacity(sample_count);
        for _ in 0..sample_count {
            samples.push(read_u64(data, &mut pos));
        }

        records.push(BenchRecord { name, samples });
    }

    // Footer: tsc_freq_khz (u64) + total_nanos (u64) + magic (8B).
    if pos + 24 > data.len() {
        bail!("HBENCH footer truncated");
    }
    let tsc_freq_khz = read_u64(data, &mut pos);
    let total_nanos = read_u64(data, &mut pos);

    // Verify footer magic.
    if &data[pos..pos + 8] != HBEND_MAGIC {
        bail!("HBENCH footer magic mismatch");
    }

    Ok(HBenchResults {
        records,
        tsc_freq_khz,
        total_nanos,
    })
}

// ---------------------------------------------------------------------------
// HPRF format (profiling data)
// ---------------------------------------------------------------------------

/// Parsed profiling results from the HPRF binary format.
#[derive(Debug)]
pub struct HPrfResults {
    /// HPRF version.
    pub version: u16,
    /// Flags: bit0=samples, bit1=ftrace.
    pub flags: u16,
    /// TSC frequency in Hz.
    pub tsc_freq_hz: u64,
    /// Kernel virtual base address.
    pub kernel_vbase: u64,
    /// Number of CPUs.
    pub cpu_count: u32,
    /// Sample records.
    pub samples: Vec<SampleRecord>,
    /// Ftrace entry records.
    pub ftrace_entries: Vec<FtraceRecord>,
}

/// A CPU sample record.
#[derive(Debug)]
pub struct SampleRecord {
    /// CPU that took the sample.
    pub cpu_id: u8,
    /// Stack depth.
    pub depth: u16,
    /// TSC timestamp.
    pub tsc: u64,
    /// Stack addresses (instruction pointers).
    pub stack: Vec<u64>,
}

/// An ftrace entry record.
#[derive(Debug)]
pub struct FtraceRecord {
    /// CPU ID.
    pub cpu_id: u8,
    /// TSC timestamp.
    pub tsc: u64,
    /// Function entry address.
    pub func_addr: u64,
}

/// Parse HPRF binary format from serial data.
pub fn parse_hprf(data: &[u8]) -> Result<HPrfResults> {
    let header_pos = find_magic(data, HPRF_MAGIC)
        .ok_or_else(|| anyhow::anyhow!("HPRF header magic not found in serial data"))?;

    let mut pos = header_pos + 4;

    if pos + 28 > data.len() {
        bail!("HPRF header truncated");
    }

    let version = read_u16(data, &mut pos);
    let flags = read_u16(data, &mut pos);
    let tsc_freq_hz = read_u64(data, &mut pos);
    let kernel_vbase = read_u64(data, &mut pos);
    let cpu_count = read_u32(data, &mut pos);
    let _reserved = read_u32(data, &mut pos);

    let mut samples = Vec::new();
    let mut ftrace_entries = Vec::new();

    // Parse tagged records until end-of-stream or data exhaustion.
    while pos < data.len() {
        if pos + 1 > data.len() {
            break;
        }
        let record_type = data[pos];
        pos += 1;

        match record_type {
            0x01 => {
                // Sample record.
                if pos + 4 > data.len() {
                    break;
                }
                let cpu_id = data[pos];
                pos += 1;
                let depth = read_u16(data, &mut pos);
                let _reserved = data[pos];
                pos += 1;
                let tsc = read_u64(data, &mut pos);

                let mut stack = Vec::with_capacity(depth as usize);
                for _ in 0..depth {
                    if pos + 8 > data.len() {
                        break;
                    }
                    stack.push(read_u64(data, &mut pos));
                }

                samples.push(SampleRecord {
                    cpu_id,
                    depth,
                    tsc,
                    stack,
                });
            }
            0x02 => {
                // Ftrace entry.
                if pos + 17 > data.len() {
                    break;
                }
                let cpu_id = data[pos];
                pos += 1;
                let _reserved = read_u16(data, &mut pos);
                let tsc = read_u64(data, &mut pos);
                let func_addr = read_u64(data, &mut pos);

                ftrace_entries.push(FtraceRecord {
                    cpu_id,
                    tsc,
                    func_addr,
                });
            }
            0xFF => {
                // End of stream.
                break;
            }
            _ => {
                // Unknown record type, stop parsing.
                break;
            }
        }
    }

    Ok(HPrfResults {
        version,
        flags,
        tsc_freq_hz,
        kernel_vbase,
        cpu_count,
        samples,
        ftrace_entries,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the position of a magic byte sequence in data.
fn find_magic(data: &[u8], magic: &[u8]) -> Option<usize> {
    data.windows(magic.len()).position(|w| w == magic)
}

fn read_u16(data: &[u8], pos: &mut usize) -> u16 {
    let val = u16::from_le_bytes([data[*pos], data[*pos + 1]]);
    *pos += 2;
    val
}

fn read_u32(data: &[u8], pos: &mut usize) -> u32 {
    let val = u32::from_le_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]);
    *pos += 4;
    val
}

fn read_u64(data: &[u8], pos: &mut usize) -> u64 {
    let val = u64::from_le_bytes([
        data[*pos],
        data[*pos + 1],
        data[*pos + 2],
        data[*pos + 3],
        data[*pos + 4],
        data[*pos + 5],
        data[*pos + 6],
        data[*pos + 7],
    ]);
    *pos += 8;
    val
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_hbench() {
        let mut data = Vec::new();
        // Header.
        data.extend_from_slice(HBENCH_MAGIC);
        data.extend_from_slice(&0u32.to_le_bytes()); // bench_count = 0
        data.extend_from_slice(&0u32.to_le_bytes()); // reserved
        // Footer.
        data.extend_from_slice(&2_000_000u64.to_le_bytes()); // tsc_freq_khz
        data.extend_from_slice(&1_000_000u64.to_le_bytes()); // total_nanos
        data.extend_from_slice(HBEND_MAGIC);

        let results = parse_hbench(&data).unwrap();
        assert_eq!(results.records.len(), 0);
        assert_eq!(results.tsc_freq_khz, 2_000_000);
        assert_eq!(results.total_nanos, 1_000_000);
    }

    #[test]
    fn parse_hbench_with_record() {
        let mut data = Vec::new();
        // Header.
        data.extend_from_slice(HBENCH_MAGIC);
        data.extend_from_slice(&1u32.to_le_bytes()); // bench_count = 1
        data.extend_from_slice(&0u32.to_le_bytes()); // reserved
        // Record.
        let name = b"test_bench";
        data.extend_from_slice(&(name.len() as u16).to_le_bytes());
        data.extend_from_slice(name);
        data.extend_from_slice(&2u32.to_le_bytes()); // 2 samples
        data.extend_from_slice(&100u64.to_le_bytes());
        data.extend_from_slice(&200u64.to_le_bytes());
        // Footer.
        data.extend_from_slice(&2_000_000u64.to_le_bytes());
        data.extend_from_slice(&500_000u64.to_le_bytes());
        data.extend_from_slice(HBEND_MAGIC);

        let results = parse_hbench(&data).unwrap();
        assert_eq!(results.records.len(), 1);
        assert_eq!(results.records[0].name, "test_bench");
        assert_eq!(results.records[0].samples, vec![100, 200]);
    }
}
