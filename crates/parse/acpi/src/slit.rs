//! System Locality Information Table (SLIT) parsing.
//!
//! The SLIT provides a matrix of relative distances between NUMA nodes
//! (system localities). Each entry is a `u8` value where 10 represents
//! the local distance and higher values indicate increasing cost.

use hadron_binparse::FromBytes;

use crate::sdt::SdtHeader;
use crate::{AcpiError, AcpiHandler};

/// SLIT table signature.
pub const SLIT_SIGNATURE: &[u8; 4] = b"SLIT";

/// Parsed SLIT table.
pub struct Slit {
    /// Number of system localities (NUMA nodes).
    num_localities: u64,
    /// The N×N distance matrix as a flat byte slice.
    matrix_data: &'static [u8],
}

impl Slit {
    /// Byte offset of `Number of System Localities` within the SLIT
    /// (immediately after the SDT header).
    const NUM_LOCALITIES_OFFSET: usize = SdtHeader::SIZE;

    /// Size of the fixed header fields after the SDT header (8 bytes for
    /// the locality count).
    const FIELDS_SIZE: usize = 8;

    /// Parse a SLIT from the given physical address.
    ///
    /// # Errors
    ///
    /// Returns [`AcpiError::InvalidSignature`] if the table signature is not
    /// `SLIT`, or [`AcpiError::InvalidChecksum`] if the checksum is invalid.
    pub fn parse(handler: &impl AcpiHandler, phys: u64) -> Result<Self, AcpiError> {
        let table = crate::sdt::load_table(handler, phys, SLIT_SIGNATURE)?;

        let num_localities = u64::read_at(table.data, Self::NUM_LOCALITIES_OFFSET)
            .ok_or(AcpiError::TruncatedData)?;

        let matrix_offset = SdtHeader::SIZE + Self::FIELDS_SIZE;
        let matrix_data = table.data.get(matrix_offset..).unwrap_or(&[]);

        Ok(Self {
            num_localities,
            matrix_data,
        })
    }

    /// Returns the number of system localities (NUMA nodes).
    #[must_use]
    pub fn num_localities(&self) -> u64 {
        self.num_localities
    }

    /// Returns the relative distance between two localities.
    ///
    /// A distance of 10 indicates the same locality. Higher values indicate
    /// greater access cost. Returns `None` if either index is out of range
    /// or the matrix data is truncated.
    #[must_use]
    pub fn distance(&self, from: u64, to: u64) -> Option<u8> {
        if from >= self.num_localities || to >= self.num_localities {
            return None;
        }
        let index = (from * self.num_localities + to) as usize;
        self.matrix_data.get(index).copied()
    }
}
