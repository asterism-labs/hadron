//! ACPI resource template parser.
//!
//! Parses the byte-encoded resource descriptors found inside `_CRS`, `_PRS`,
//! and similar ACPI buffer objects. The parser handles both small (1-byte tag)
//! and large (3-byte tag) descriptors as defined in ACPI 6.5 §6.4.

/// A decoded ACPI resource descriptor from a `_CRS` buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcpiResource {
    /// I/O port range (small resource tag 0x47).
    Io {
        /// Base I/O port address.
        base: u16,
        /// Number of ports.
        length: u16,
    },
    /// Fixed I/O port range (small resource tag 0x4B).
    FixedIo {
        /// Base I/O port address.
        base: u16,
        /// Number of ports.
        length: u8,
    },
    /// Interrupt line from small IRQ descriptor (tags 0x22/0x23).
    Irq {
        /// IRQ number (0-15).
        irq: u8,
        /// Whether the interrupt is edge-triggered (vs level-triggered).
        edge_triggered: bool,
        /// Whether the interrupt is active-low (vs active-high).
        active_low: bool,
    },
    /// 32-bit memory range (large resource tag 0x05).
    Memory32 {
        /// Base physical address.
        base: u32,
        /// Length in bytes.
        length: u32,
        /// Whether the region is writable.
        writable: bool,
    },
    /// 32-bit fixed memory range (large resource tag 0x06).
    FixedMemory32 {
        /// Base physical address.
        base: u32,
        /// Length in bytes.
        length: u32,
        /// Whether the region is writable.
        writable: bool,
    },
    /// 64-bit memory region from QWord address space (large resource tag 0x0A).
    Memory64 {
        /// Base physical address.
        base: u64,
        /// Length in bytes.
        length: u64,
        /// Whether the region is writable.
        writable: bool,
    },
    /// Extended IRQ descriptor (large resource tag 0x09).
    ExtendedIrq {
        /// Global System Interrupt number.
        gsi: u32,
        /// Whether the interrupt is edge-triggered.
        edge_triggered: bool,
        /// Whether the interrupt is active-low.
        active_low: bool,
    },
    /// DMA channel (small resource tag 0x2A).
    Dma {
        /// DMA channel number (0-7).
        channel: u8,
        /// Whether the channel supports bus mastering.
        bus_master: bool,
    },
}

/// Iterator over resource descriptors in a resource template buffer.
#[derive(Clone)]
pub struct ResourceIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> ResourceIter<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> &'a [u8] {
        self.data.get(self.pos..).unwrap_or(&[])
    }

    fn read_u8(&mut self) -> Option<u8> {
        let v = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(v)
    }

    fn read_u16_le(&mut self) -> Option<u16> {
        let lo = *self.data.get(self.pos)? as u16;
        let hi = *self.data.get(self.pos + 1)? as u16;
        self.pos += 2;
        Some(lo | (hi << 8))
    }

    fn read_u32_le(&mut self) -> Option<u32> {
        let b = self.data.get(self.pos..self.pos + 4)?;
        self.pos += 4;
        Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64_le(&mut self) -> Option<u64> {
        let b = self.data.get(self.pos..self.pos + 8)?;
        self.pos += 8;
        Some(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn skip(&mut self, n: usize) {
        self.pos = (self.pos + n).min(self.data.len());
    }

    /// Parse a small resource descriptor (bit 7 of tag byte = 0).
    fn parse_small(&mut self, tag_byte: u8) -> Option<AcpiResource> {
        let tag_type = (tag_byte >> 3) & 0x0F;
        let length = (tag_byte & 0x07) as usize;

        match tag_type {
            // IRQ descriptor (tag types 0x04 and 0x04 with flags)
            // Small resource type 0x04 = IRQ format, tag bytes 0x22/0x23
            0x04 => {
                // IRQ mask: 16-bit bitmask
                if length < 2 {
                    self.skip(length);
                    return None;
                }
                let mask = self.read_u16_le()?;
                let (edge_triggered, active_low) = if length >= 3 {
                    let flags = self.read_u8()?;
                    let edge = (flags & 0x01) != 0;
                    let low = (flags & 0x08) != 0;
                    // Skip any remaining bytes
                    if length > 3 {
                        self.skip(length - 3);
                    }
                    (edge, low)
                } else {
                    // No flags byte: default is edge-triggered, active-high (ISA)
                    (true, false)
                };
                // Find first set bit in mask
                let irq = mask.trailing_zeros();
                if irq >= 16 {
                    return None;
                }
                Some(AcpiResource::Irq {
                    irq: irq as u8,
                    edge_triggered,
                    active_low,
                })
            }
            // DMA descriptor (tag type 0x05)
            0x05 => {
                if length < 2 {
                    self.skip(length);
                    return None;
                }
                let channel_mask = self.read_u8()?;
                let flags = self.read_u8()?;
                if length > 2 {
                    self.skip(length - 2);
                }
                let channel = channel_mask.trailing_zeros();
                if channel >= 8 {
                    return None;
                }
                let bus_master = (flags & 0x04) != 0;
                Some(AcpiResource::Dma {
                    channel: channel as u8,
                    bus_master,
                })
            }
            // I/O port descriptor (tag type 0x08)
            0x08 => {
                if length < 7 {
                    self.skip(length);
                    return None;
                }
                let _decode = self.read_u8()?; // decode type (10-bit vs 16-bit)
                let min_base = self.read_u16_le()?;
                let _max_base = self.read_u16_le()?;
                let _alignment = self.read_u8()?;
                let range_len = self.read_u8()?;
                if length > 7 {
                    self.skip(length - 7);
                }
                Some(AcpiResource::Io {
                    base: min_base,
                    length: range_len as u16,
                })
            }
            // Fixed I/O port descriptor (tag type 0x09)
            0x09 => {
                if length < 3 {
                    self.skip(length);
                    return None;
                }
                let base = self.read_u16_le()?;
                let range_len = self.read_u8()?;
                if length > 3 {
                    self.skip(length - 3);
                }
                Some(AcpiResource::FixedIo {
                    base,
                    length: range_len,
                })
            }
            // End Tag (tag type 0x0F)
            0x0F => None,
            // Unknown small resource — skip
            _ => {
                self.skip(length);
                None
            }
        }
    }

    /// Parse a large resource descriptor (bit 7 of tag byte = 1).
    fn parse_large(&mut self, tag_byte: u8) -> Option<AcpiResource> {
        let tag_type = tag_byte & 0x7F;

        // Large descriptors have a 16-bit length field following the tag.
        let length = self.read_u16_le()? as usize;
        let body_start = self.pos;

        let result = match tag_type {
            // 24-bit memory range (tag 0x01) — rarely used, skip
            0x01 => None,
            // 32-bit memory range (tag 0x05)
            0x05 => {
                if length < 9 {
                    None
                } else {
                    let flags = self.read_u8()?;
                    let _min = self.read_u32_le()?;
                    let _max = self.read_u32_le()?;
                    // Descriptor also has alignment and length
                    if length >= 17 {
                        let _alignment = self.read_u32_le()?;
                        let range_len = self.read_u32_le()?;
                        Some(AcpiResource::Memory32 {
                            base: _min,
                            length: range_len,
                            writable: (flags & 0x01) != 0,
                        })
                    } else {
                        None
                    }
                }
            }
            // 32-bit fixed memory range (tag 0x06)
            0x06 => {
                if length < 9 {
                    None
                } else {
                    let flags = self.read_u8()?;
                    let base = self.read_u32_le()?;
                    let range_len = self.read_u32_le()?;
                    Some(AcpiResource::FixedMemory32 {
                        base,
                        length: range_len,
                        writable: (flags & 0x01) != 0,
                    })
                }
            }
            // DWord address space (tag 0x07) — decode as memory if resource type = 0
            0x07 => {
                if length < 23 {
                    None
                } else {
                    let resource_type = self.read_u8()?;
                    let _general_flags = self.read_u8()?;
                    let type_flags = self.read_u8()?;
                    let _granularity = self.read_u32_le()?;
                    let min = self.read_u32_le()?;
                    let _max = self.read_u32_le()?;
                    let _translation = self.read_u32_le()?;
                    let range_len = self.read_u32_le()?;
                    if resource_type == 0 {
                        // Memory range
                        Some(AcpiResource::Memory32 {
                            base: min,
                            length: range_len,
                            writable: (type_flags & 0x01) != 0,
                        })
                    } else if resource_type == 1 {
                        // I/O range
                        Some(AcpiResource::Io {
                            base: min as u16,
                            length: range_len as u16,
                        })
                    } else {
                        None
                    }
                }
            }
            // Extended IRQ descriptor (tag 0x09)
            0x09 => {
                if length < 6 {
                    None
                } else {
                    let flags = self.read_u8()?;
                    let count = self.read_u8()?;
                    if count == 0 {
                        None
                    } else {
                        let gsi = self.read_u32_le()?;
                        let edge_triggered = (flags & 0x02) != 0;
                        let active_low = (flags & 0x08) != 0;
                        Some(AcpiResource::ExtendedIrq {
                            gsi,
                            edge_triggered,
                            active_low,
                        })
                    }
                }
            }
            // QWord address space (tag 0x0A) — decode as memory if resource type = 0
            0x0A => {
                if length < 43 {
                    None
                } else {
                    let resource_type = self.read_u8()?;
                    let _general_flags = self.read_u8()?;
                    let type_flags = self.read_u8()?;
                    let _granularity = self.read_u64_le()?;
                    let min = self.read_u64_le()?;
                    let _max = self.read_u64_le()?;
                    let _translation = self.read_u64_le()?;
                    let range_len = self.read_u64_le()?;
                    if resource_type == 0 {
                        Some(AcpiResource::Memory64 {
                            base: min,
                            length: range_len,
                            writable: (type_flags & 0x01) != 0,
                        })
                    } else {
                        None
                    }
                }
            }
            _ => None,
        };

        // Always advance past the full descriptor body.
        self.pos = body_start + length;
        result
    }
}

impl<'a> Iterator for ResourceIter<'a> {
    type Item = AcpiResource;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.pos >= self.data.len() {
                return None;
            }

            let tag_byte = self.read_u8()?;

            // End Tag: small resource type 0x0F → tag byte 0x79
            if tag_byte == 0x79 {
                return None;
            }

            let is_large = (tag_byte & 0x80) != 0;
            let result = if is_large {
                self.parse_large(tag_byte)
            } else {
                self.parse_small(tag_byte)
            };

            if let Some(resource) = result {
                return Some(resource);
            }
            // If the descriptor was unrecognised or empty, loop to next.
        }
    }
}

/// Parse a resource template buffer into an iterator of [`AcpiResource`] items.
///
/// `data` should point to the raw byte contents of the resource template buffer
/// (i.e., the initializer data from a `Buffer` AML object). The iterator yields
/// resources until the End Tag descriptor (0x79) is encountered or the data is
/// exhausted.
pub fn parse_resource_template(data: &[u8]) -> ResourceIter<'_> {
    ResourceIter::new(data)
}

/// Checks whether the given byte slice looks like a valid resource template.
///
/// A resource template starts with a valid descriptor tag and should end with
/// an End Tag (0x79). This is a quick heuristic check, not a full validation.
pub fn looks_like_resource_template(data: &[u8]) -> bool {
    if data.is_empty() {
        return false;
    }

    let first = data[0];
    // Must start with a valid small or large resource tag
    let valid_start = if first & 0x80 != 0 {
        // Large descriptor: tag type 0x01-0x0F are common
        let tag = first & 0x7F;
        tag >= 0x01 && tag <= 0x0E
    } else {
        // Small descriptor: check known tag types
        let tag_type = (first >> 3) & 0x0F;
        matches!(tag_type, 0x04 | 0x05 | 0x06 | 0x08 | 0x09 | 0x0E)
    };

    if !valid_start {
        return false;
    }

    // Check for End Tag somewhere in the buffer
    data.iter().any(|&b| b == 0x79)
}

#[cfg(test)]
mod tests {
    extern crate std;
    use std::vec::Vec;
    use super::*;

    #[test]
    fn parse_io_descriptor() {
        // I/O descriptor: tag 0x47, decode=1, min=0x03F8, max=0x03F8, align=1, len=8
        let data = [0x47, 0x01, 0xF8, 0x03, 0xF8, 0x03, 0x01, 0x08, 0x79, 0x00];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::Io {
                base: 0x03F8,
                length: 8
            }
        );
    }

    #[test]
    fn parse_fixed_io_descriptor() {
        // Fixed I/O descriptor: tag 0x4B, base=0x0060, len=1
        let data = [0x4B, 0x60, 0x00, 0x01, 0x79, 0x00];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::FixedIo {
                base: 0x0060,
                length: 1
            }
        );
    }

    #[test]
    fn parse_irq_descriptor_no_flags() {
        // IRQ descriptor without flags: tag 0x22 (type 0x04, len 2), mask=0x0010 (IRQ 4)
        let data = [0x22, 0x10, 0x00, 0x79, 0x00];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::Irq {
                irq: 4,
                edge_triggered: true,
                active_low: false,
            }
        );
    }

    #[test]
    fn parse_irq_descriptor_with_flags() {
        // IRQ descriptor with flags: tag 0x23 (type 0x04, len 3), mask=0x0010 (IRQ 4)
        // flags: edge=1, active-low=1
        let data = [0x23, 0x10, 0x00, 0x09, 0x79, 0x00];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::Irq {
                irq: 4,
                edge_triggered: true,
                active_low: true,
            }
        );
    }

    #[test]
    fn parse_extended_irq() {
        // Extended IRQ: large tag 0x89, length=6, flags=0x02 (edge), count=1, GSI=9
        let data = [
            0x89, 0x06, 0x00, // tag + length
            0x02, // flags: edge-triggered
            0x01, // interrupt count
            0x09, 0x00, 0x00, 0x00, // GSI 9
            0x79, 0x00, // end tag
        ];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::ExtendedIrq {
                gsi: 9,
                edge_triggered: true,
                active_low: false,
            }
        );
    }

    #[test]
    fn parse_fixed_memory32() {
        // Fixed Memory32: large tag 0x86, length=9, flags=1 (writable),
        // base=0xFED00000, length=0x1000
        let data = [
            0x86, 0x09, 0x00, // tag + length
            0x01, // flags: writable
            0x00, 0x00, 0xD0, 0xFE, // base 0xFED00000
            0x00, 0x10, 0x00, 0x00, // length 0x1000
            0x79, 0x00, // end tag
        ];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::FixedMemory32 {
                base: 0xFED0_0000,
                length: 0x1000,
                writable: true,
            }
        );
    }

    #[test]
    fn parse_dma_descriptor() {
        // DMA descriptor: tag 0x2A (type 0x05, len 2), channel_mask=0x04 (ch 2), flags=0x04
        let data = [0x2A, 0x04, 0x04, 0x79, 0x00];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 1);
        assert_eq!(
            resources[0],
            AcpiResource::Dma {
                channel: 2,
                bus_master: true,
            }
        );
    }

    #[test]
    fn parse_multiple_resources() {
        // COM1: I/O 0x3F8 len 8, IRQ 4
        let data = [
            0x47, 0x01, 0xF8, 0x03, 0xF8, 0x03, 0x01, 0x08, // I/O descriptor
            0x22, 0x10, 0x00, // IRQ descriptor (IRQ 4)
            0x79, 0x00, // end tag
        ];
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert_eq!(resources.len(), 2);
        assert_eq!(
            resources[0],
            AcpiResource::Io {
                base: 0x03F8,
                length: 8
            }
        );
        assert_eq!(
            resources[1],
            AcpiResource::Irq {
                irq: 4,
                edge_triggered: true,
                active_low: false,
            }
        );
    }

    #[test]
    fn empty_template() {
        let data = [0x79, 0x00]; // just end tag
        let resources: Vec<_> = parse_resource_template(&data).collect();
        assert!(resources.is_empty());
    }

    #[test]
    fn looks_like_resource_template_checks() {
        // Valid: starts with I/O descriptor and has end tag
        assert!(looks_like_resource_template(&[
            0x47, 0x01, 0xF8, 0x03, 0xF8, 0x03, 0x01, 0x08, 0x79, 0x00
        ]));
        // Just end tag
        assert!(!looks_like_resource_template(&[0x79, 0x00]));
        // Empty
        assert!(!looks_like_resource_template(&[]));
        // Random data
        assert!(!looks_like_resource_template(&[0x00, 0x01, 0x02]));
    }
}
