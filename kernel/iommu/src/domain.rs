//! Bitmap-based DMA domain ID allocator.
//!
//! Each VT-d unit supports a hardware-defined number of domains (from `CAP.ND`).
//! Domain 0 is reserved as the default/fault domain.

use alloc::vec::Vec;

use crate::hw::IommuError;

/// Opaque domain identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainId(pub(crate) u16);

impl DomainId {
    /// Returns the raw domain ID value.
    #[must_use]
    pub fn as_u16(self) -> u16 {
        self.0
    }
}

/// Bitmap-based domain ID allocator.
///
/// Bit N is set if domain N is allocated. Domain 0 is always reserved.
pub struct DomainAllocator {
    /// Bitmap words.
    bitmap: Vec<u64>,
    /// Maximum number of domains supported.
    max_domains: u16,
}

/// Number of bits per bitmap word.
const BITS_PER_WORD: u16 = 64;

impl DomainAllocator {
    /// Create a new allocator for the given number of domains.
    ///
    /// `max_domains` is decoded from the VT-d `CAP.ND` field:
    /// 0 -> 16, 1 -> 64, 2 -> 256, 3 -> 1024, 4 -> 4096, 5 -> 16384, 6 -> 65536.
    ///
    /// Domain 0 is reserved automatically.
    #[must_use]
    pub fn new(max_domains: u16) -> Self {
        let word_count =
            (u64::from(max_domains) + u64::from(BITS_PER_WORD) - 1) / u64::from(BITS_PER_WORD);
        let mut bitmap = alloc::vec![0u64; word_count as usize];
        // Reserve domain 0.
        if !bitmap.is_empty() {
            bitmap[0] |= 1;
        }
        Self {
            bitmap,
            max_domains,
        }
    }

    /// Decode the `CAP.ND` field value into the actual number of domains.
    #[must_use]
    pub fn decode_nd(nd: u8) -> u16 {
        match nd & 0x07 {
            0 => 16,
            1 => 64,
            2 => 256,
            3 => 1024,
            4 => 4096,
            5 => 16384,
            6 => u16::MAX, // 65536 domains, clamped to u16::MAX
            _ => 16,       // Reserved — fall back to minimum
        }
    }

    /// Allocate a domain ID.
    pub fn alloc(&mut self) -> Result<DomainId, IommuError> {
        for (word_idx, word) in self.bitmap.iter_mut().enumerate() {
            if *word == u64::MAX {
                continue;
            }
            // Find first zero bit.
            let bit = (!*word).trailing_zeros();
            let domain = word_idx as u16 * BITS_PER_WORD + bit as u16;
            if domain >= self.max_domains {
                break;
            }
            *word |= 1u64 << bit;
            return Ok(DomainId(domain));
        }
        Err(IommuError::DomainExhausted)
    }

    /// Free a previously allocated domain ID.
    pub fn free(&mut self, id: DomainId) -> Result<(), IommuError> {
        let domain = id.0;
        if domain == 0 || domain >= self.max_domains {
            return Err(IommuError::InvalidDomain);
        }
        let word_idx = (domain / BITS_PER_WORD) as usize;
        let bit = domain % BITS_PER_WORD;
        let word = self
            .bitmap
            .get_mut(word_idx)
            .ok_or(IommuError::InvalidDomain)?;
        if *word & (1u64 << bit) == 0 {
            return Err(IommuError::InvalidDomain); // Double-free
        }
        *word &= !(1u64 << bit);
        Ok(())
    }

    /// Returns the maximum number of domains.
    #[must_use]
    pub fn max_domains(&self) -> u16 {
        self.max_domains
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alloc_and_free() {
        let mut alloc = DomainAllocator::new(16);
        // Domain 0 is reserved, first alloc should return 1.
        let d1 = alloc.alloc().unwrap();
        assert_eq!(d1.as_u16(), 1);
        let d2 = alloc.alloc().unwrap();
        assert_eq!(d2.as_u16(), 2);
        alloc.free(d1).unwrap();
        // Re-alloc should reuse domain 1.
        let d3 = alloc.alloc().unwrap();
        assert_eq!(d3.as_u16(), 1);
    }

    #[test]
    fn exhaustion() {
        let mut alloc = DomainAllocator::new(16);
        // Allocate all 15 available domains (1-15).
        for _ in 0..15 {
            alloc.alloc().unwrap();
        }
        assert!(matches!(alloc.alloc(), Err(IommuError::DomainExhausted)));
    }

    #[test]
    fn decode_nd_values() {
        assert_eq!(DomainAllocator::decode_nd(0), 16);
        assert_eq!(DomainAllocator::decode_nd(1), 64);
        assert_eq!(DomainAllocator::decode_nd(2), 256);
        assert_eq!(DomainAllocator::decode_nd(3), 1024);
        assert_eq!(DomainAllocator::decode_nd(4), 4096);
    }
}
