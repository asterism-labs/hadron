//! `hadron-fdt` --- a standalone, `no_std` Flattened Device Tree (DTB) parser.
//!
//! This crate parses DTB blobs as defined by the Devicetree Specification.
//! It provides zero-copy access to nodes, properties, and memory reservations
//! from a `&[u8]` slice containing the raw DTB data.
//!
//! # Usage
//!
//! ```ignore
//! let fdt = Fdt::parse(dtb_bytes)?;
//! let root = fdt.root();
//! for child in root.children() {
//!     // ...
//! }
//! if let Some(uart) = fdt.find_node("/soc/serial@10000000") {
//!     let reg = uart.property("reg").unwrap();
//! }
//! ```

#![no_std]
#![warn(missing_docs)]

pub mod header;
pub mod node;
pub mod property;
pub mod reservation;

pub use node::FdtNode;
pub use property::{FdtProperty, PropertyIter, StrListIter};
pub use reservation::{MemReservation, MemReservationIter};

use hadron_binparse::FromBytes;
use header::{FDT_MAGIC, FDT_MIN_COMPAT_VERSION, RawFdtHeader};

/// Errors that can occur during FDT parsing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FdtError {
    /// The magic number was not `0xd00dfeed`.
    InvalidMagic,
    /// The `last_comp_version` field is below the minimum we support (16).
    UnsupportedVersion,
    /// The DTB data is shorter than the header or declared block offsets.
    TruncatedData,
    /// A structural invariant was violated (e.g. overlapping blocks).
    InvalidStructure,
}

/// Parsed Flattened Device Tree.
///
/// Borrows the raw DTB `&[u8]` and provides access to nodes, properties,
/// and memory reservations.
pub struct Fdt<'a> {
    data: &'a [u8],
    struct_block: &'a [u8],
    strings_block: &'a [u8],
    boot_cpuid: u32,
    mem_rsv_data: &'a [u8],
}

impl<'a> Fdt<'a> {
    /// Parses a DTB blob from raw bytes.
    ///
    /// Validates the header magic, version, and bounds-checks all block
    /// offsets against the data length.
    ///
    /// # Errors
    ///
    /// Returns an [`FdtError`] if the blob is malformed.
    pub fn parse(data: &'a [u8]) -> Result<Self, FdtError> {
        let hdr = RawFdtHeader::read_from(data).ok_or(FdtError::TruncatedData)?;

        if hdr.magic.get() != FDT_MAGIC {
            return Err(FdtError::InvalidMagic);
        }

        if hdr.last_comp_version.get() < FDT_MIN_COMPAT_VERSION {
            return Err(FdtError::UnsupportedVersion);
        }

        let total_size = hdr.totalsize.get() as usize;
        if data.len() < total_size {
            return Err(FdtError::TruncatedData);
        }

        let struct_off = hdr.off_dt_struct.get() as usize;
        let struct_len = hdr.size_dt_struct.get() as usize;
        let strings_off = hdr.off_dt_strings.get() as usize;
        let strings_len = hdr.size_dt_strings.get() as usize;
        let mem_rsv_off = hdr.off_mem_rsvmap.get() as usize;

        // Bounds-check all block regions.
        let struct_end = struct_off
            .checked_add(struct_len)
            .ok_or(FdtError::InvalidStructure)?;
        let strings_end = strings_off
            .checked_add(strings_len)
            .ok_or(FdtError::InvalidStructure)?;

        if struct_end > total_size || strings_end > total_size || mem_rsv_off > total_size {
            return Err(FdtError::TruncatedData);
        }

        let struct_block = &data[struct_off..struct_end];
        let strings_block = &data[strings_off..strings_end];
        // Reservation block extends from its offset to the start of the struct block.
        let mem_rsv_end = struct_off.min(total_size);
        let mem_rsv_data = if mem_rsv_off <= mem_rsv_end {
            &data[mem_rsv_off..mem_rsv_end]
        } else {
            &data[mem_rsv_off..mem_rsv_off]
        };

        Ok(Self {
            data,
            struct_block,
            strings_block,
            boot_cpuid: hdr.boot_cpuid_phys.get(),
            mem_rsv_data,
        })
    }

    /// Returns the root node of the device tree.
    ///
    /// # Panics
    ///
    /// Panics if the structure block does not start with a valid root node.
    /// This should not happen for a blob that passed [`Fdt::parse`].
    #[must_use]
    pub fn root(&self) -> FdtNode<'a> {
        // The structure block starts with FDT_BEGIN_NODE for the root.
        let tag = node::read_token_tag(self.struct_block, 0).expect("empty struct block");
        assert_eq!(
            tag,
            node::FDT_BEGIN_NODE,
            "struct block must begin with FDT_BEGIN_NODE"
        );

        let name_start = 4;
        let name = node::str_from_offset(self.struct_block, name_start).unwrap_or("");
        let content_offset = node::align4(name_start + name.len() + 1);

        FdtNode::new(self.struct_block, self.strings_block, name, content_offset)
    }

    /// Finds a node by its full path (e.g. `"/cpus/cpu@0"`).
    ///
    /// Returns `None` if any component along the path is not found.
    #[must_use]
    pub fn find_node(&self, path: &str) -> Option<FdtNode<'a>> {
        let mut current = self.root();

        for component in path.split('/') {
            if component.is_empty() {
                continue;
            }
            current = current.find_child(component)?;
        }

        Some(current)
    }

    /// Returns an iterator over the memory reservation entries.
    #[must_use]
    pub fn memory_reservations(&self) -> MemReservationIter<'a> {
        MemReservationIter::new(self.mem_rsv_data)
    }

    /// Returns the physical boot CPU ID.
    #[must_use]
    pub fn boot_cpuid(&self) -> u32 {
        self.boot_cpuid
    }

    /// Returns the total size of the DTB blob in bytes.
    #[must_use]
    pub fn total_size(&self) -> usize {
        self.data.len()
    }
}

#[cfg(test)]
extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    // ---- DTB builder helpers ------------------------------------------------

    fn be32(val: u32) -> [u8; 4] {
        val.to_be_bytes()
    }

    fn be64(val: u64) -> [u8; 8] {
        val.to_be_bytes()
    }

    /// Pads `v` to a 4-byte boundary.
    fn pad4(v: &mut Vec<u8>) {
        while v.len() % 4 != 0 {
            v.push(0);
        }
    }

    /// Appends a `FDT_BEGIN_NODE` token with the given name.
    fn emit_begin_node(v: &mut Vec<u8>, name: &str) {
        v.extend_from_slice(&be32(node::FDT_BEGIN_NODE));
        v.extend_from_slice(name.as_bytes());
        v.push(0); // null terminator
        pad4(v);
    }

    /// Appends an `FDT_END_NODE` token.
    fn emit_end_node(v: &mut Vec<u8>) {
        v.extend_from_slice(&be32(node::FDT_END_NODE));
    }

    /// Appends an `FDT_PROP` token.
    fn emit_prop(v: &mut Vec<u8>, name_offset: u32, data: &[u8]) {
        v.extend_from_slice(&be32(node::FDT_PROP));
        v.extend_from_slice(&be32(data.len() as u32));
        v.extend_from_slice(&be32(name_offset));
        v.extend_from_slice(data);
        pad4(v);
    }

    /// Appends `FDT_END`.
    fn emit_end(v: &mut Vec<u8>) {
        v.extend_from_slice(&be32(node::FDT_END));
    }

    /// Builds a strings block from a list of names.
    /// Returns (strings_block_bytes, Vec<offset_for_each_name>).
    fn build_strings(names: &[&str]) -> (Vec<u8>, Vec<u32>) {
        let mut block = Vec::new();
        let mut offsets = Vec::new();
        for name in names {
            offsets.push(block.len() as u32);
            block.extend_from_slice(name.as_bytes());
            block.push(0);
        }
        (block, offsets)
    }

    /// Builds a complete minimal DTB with the given struct and strings blocks
    /// and optional reservation entries (pairs of (addr, size)).
    fn build_dtb(
        struct_block: &[u8],
        strings_block: &[u8],
        reservations: &[(u64, u64)],
        boot_cpuid: u32,
    ) -> Vec<u8> {
        let header_size = 40u32; // 10 × 4 bytes
        let mem_rsv_off = header_size;

        // Reservation entries: each is 16 bytes (2 × Be64), plus terminator.
        let rsv_size = (reservations.len() + 1) * 16;
        let struct_off = mem_rsv_off as usize + rsv_size;
        let strings_off = struct_off + struct_block.len();
        let total_size = strings_off + strings_block.len();

        let mut dtb = Vec::with_capacity(total_size);

        // Header
        dtb.extend_from_slice(&be32(0xd00d_feed)); // magic
        dtb.extend_from_slice(&be32(total_size as u32)); // totalsize
        dtb.extend_from_slice(&be32(struct_off as u32)); // off_dt_struct
        dtb.extend_from_slice(&be32(strings_off as u32)); // off_dt_strings
        dtb.extend_from_slice(&be32(mem_rsv_off)); // off_mem_rsvmap
        dtb.extend_from_slice(&be32(17)); // version
        dtb.extend_from_slice(&be32(16)); // last_comp_version
        dtb.extend_from_slice(&be32(boot_cpuid)); // boot_cpuid_phys
        dtb.extend_from_slice(&be32(strings_block.len() as u32)); // size_dt_strings
        dtb.extend_from_slice(&be32(struct_block.len() as u32)); // size_dt_struct

        // Reservation entries
        for &(addr, size) in reservations {
            dtb.extend_from_slice(&be64(addr));
            dtb.extend_from_slice(&be64(size));
        }
        // Terminator
        dtb.extend_from_slice(&be64(0));
        dtb.extend_from_slice(&be64(0));

        // Struct block
        dtb.extend_from_slice(struct_block);

        // Strings block
        dtb.extend_from_slice(strings_block);

        assert_eq!(dtb.len(), total_size);
        dtb
    }

    /// Builds a simple DTB with a root node containing one property and one
    /// child node with its own property:
    ///
    /// ```text
    /// / {
    ///     model = "test-board";
    ///     cpus {
    ///         #address-cells = <1>;
    ///         cpu@0 {
    ///             compatible = "arm,cortex-a53\0arm,armv8";
    ///         };
    ///     };
    /// };
    /// ```
    fn build_test_dtb() -> Vec<u8> {
        let (strings, offsets) = build_strings(&["model", "#address-cells", "compatible"]);

        let mut st = Vec::new();

        // Root node
        emit_begin_node(&mut st, "");
        emit_prop(&mut st, offsets[0], b"test-board\0");

        // /cpus
        emit_begin_node(&mut st, "cpus");
        emit_prop(&mut st, offsets[1], &be32(1));

        // /cpus/cpu@0
        emit_begin_node(&mut st, "cpu@0");
        emit_prop(&mut st, offsets[2], b"arm,cortex-a53\0arm,armv8\0");
        emit_end_node(&mut st);

        emit_end_node(&mut st); // /cpus
        emit_end_node(&mut st); // /
        emit_end(&mut st);

        build_dtb(&st, &strings, &[(0x8000_0000, 0x1000)], 0)
    }

    // ---- Header validation tests --------------------------------------------

    #[test]
    fn parse_valid_dtb() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        assert_eq!(fdt.boot_cpuid(), 0);
        assert_eq!(fdt.total_size(), dtb.len());
    }

    #[test]
    fn parse_bad_magic() {
        let mut dtb = build_test_dtb();
        // Corrupt magic.
        dtb[0] = 0;
        assert!(matches!(Fdt::parse(&dtb), Err(FdtError::InvalidMagic)));
    }

    #[test]
    fn parse_bad_version() {
        let mut dtb = build_test_dtb();
        // Set last_comp_version to 15 (below minimum 16).
        let v = 15u32.to_be_bytes();
        dtb[24..28].copy_from_slice(&v);
        assert!(matches!(
            Fdt::parse(&dtb),
            Err(FdtError::UnsupportedVersion)
        ));
    }

    #[test]
    fn parse_truncated() {
        let dtb = build_test_dtb();
        // Provide only the header (no blocks).
        assert!(matches!(
            Fdt::parse(&dtb[..20]),
            Err(FdtError::TruncatedData)
        ));
    }

    // ---- Node traversal tests -----------------------------------------------

    #[test]
    fn root_name_is_empty() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        assert_eq!(fdt.root().name(), "");
    }

    #[test]
    fn root_children() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let names: Vec<&str> = fdt.root().children().map(|n| n.name()).collect();
        assert_eq!(names, &["cpus"]);
    }

    #[test]
    fn nested_children() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let cpus = fdt.root().find_child("cpus").unwrap();
        let names: Vec<&str> = cpus.children().map(|n| n.name()).collect();
        assert_eq!(names, &["cpu@0"]);
    }

    // ---- Property access tests ----------------------------------------------

    #[test]
    fn property_as_str() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let model = fdt.root().property("model").unwrap();
        assert_eq!(model.as_str(), Some("test-board"));
    }

    #[test]
    fn property_as_u32() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let cpus = fdt.find_node("/cpus").unwrap();
        let cells = cpus.property("#address-cells").unwrap();
        assert_eq!(cells.as_u32(), Some(1));
    }

    #[test]
    fn property_as_str_list() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let cpu = fdt.find_node("/cpus/cpu@0").unwrap();
        let compat = cpu.property("compatible").unwrap();
        let list: Vec<&str> = compat.as_str_list().collect();
        assert_eq!(list, &["arm,cortex-a53", "arm,armv8"]);
    }

    #[test]
    fn missing_property_returns_none() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        assert!(fdt.root().property("nonexistent").is_none());
    }

    #[test]
    fn property_as_u64() {
        let (strings, offsets) = build_strings(&["reg"]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_prop(&mut st, offsets[0], &be64(0x4000_0000_0000_0000));
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        let reg = fdt.root().property("reg").unwrap();
        assert_eq!(reg.as_u64(), Some(0x4000_0000_0000_0000));
    }

    #[test]
    fn empty_property() {
        let (strings, offsets) = build_strings(&["ranges"]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_prop(&mut st, offsets[0], &[]);
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        let ranges = fdt.root().property("ranges").unwrap();
        assert!(ranges.is_empty());
        assert_eq!(ranges.as_u32(), None);
        assert_eq!(ranges.as_str(), Some(""));
    }

    // ---- Path-based lookup tests --------------------------------------------

    #[test]
    fn find_node_root() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let root = fdt.find_node("/").unwrap();
        assert_eq!(root.name(), "");
    }

    #[test]
    fn find_node_single_level() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let cpus = fdt.find_node("/cpus").unwrap();
        assert_eq!(cpus.name(), "cpus");
    }

    #[test]
    fn find_node_nested() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let cpu = fdt.find_node("/cpus/cpu@0").unwrap();
        assert_eq!(cpu.name(), "cpu@0");
    }

    #[test]
    fn find_node_missing() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        assert!(fdt.find_node("/nonexistent").is_none());
        assert!(fdt.find_node("/cpus/cpu@1").is_none());
    }

    // ---- Memory reservation tests -------------------------------------------

    #[test]
    fn memory_reservations() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let rsv: Vec<MemReservation> = fdt.memory_reservations().collect();
        assert_eq!(rsv.len(), 1);
        assert_eq!(rsv[0].address, 0x8000_0000);
        assert_eq!(rsv[0].size, 0x1000);
    }

    #[test]
    fn no_reservations() {
        let (strings, _) = build_strings(&[]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        assert_eq!(fdt.memory_reservations().count(), 0);
    }

    // ---- find_compatible test -----------------------------------------------

    #[test]
    fn find_compatible() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let node = fdt.root().find_compatible("arm,cortex-a53").unwrap();
        assert_eq!(node.name(), "cpu@0");
    }

    #[test]
    fn find_compatible_second_entry() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        let node = fdt.root().find_compatible("arm,armv8").unwrap();
        assert_eq!(node.name(), "cpu@0");
    }

    #[test]
    fn find_compatible_missing() {
        let dtb = build_test_dtb();
        let fdt = Fdt::parse(&dtb).unwrap();
        assert!(fdt.root().find_compatible("riscv,sifive").is_none());
    }

    // ---- Empty node test ----------------------------------------------------

    #[test]
    fn empty_node_no_children_no_props() {
        let (strings, _) = build_strings(&[]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        let root = fdt.root();
        assert_eq!(root.properties().count(), 0);
        assert_eq!(root.children().count(), 0);
    }

    // ---- Multiple siblings test ---------------------------------------------

    #[test]
    fn multiple_sibling_children() {
        let (strings, offsets) = build_strings(&["name"]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");

        emit_begin_node(&mut st, "alpha");
        emit_prop(&mut st, offsets[0], b"a\0");
        emit_end_node(&mut st);

        emit_begin_node(&mut st, "beta");
        emit_prop(&mut st, offsets[0], b"b\0");
        emit_end_node(&mut st);

        emit_begin_node(&mut st, "gamma");
        emit_end_node(&mut st);

        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        let names: Vec<&str> = fdt.root().children().map(|n| n.name()).collect();
        assert_eq!(names, &["alpha", "beta", "gamma"]);
    }

    // ---- Boot CPU ID test ---------------------------------------------------

    #[test]
    fn boot_cpuid_nonzero() {
        let (strings, _) = build_strings(&[]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 3);
        let fdt = Fdt::parse(&dtb).unwrap();
        assert_eq!(fdt.boot_cpuid(), 3);
    }

    // ---- Multiple memory reservations test ----------------------------------

    #[test]
    fn multiple_memory_reservations() {
        let (strings, _) = build_strings(&[]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[(0x1000, 0x2000), (0x5000_0000, 0x100)], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        let rsv: Vec<MemReservation> = fdt.memory_reservations().collect();
        assert_eq!(rsv.len(), 2);
        assert_eq!(
            rsv[0],
            MemReservation {
                address: 0x1000,
                size: 0x2000
            }
        );
        assert_eq!(
            rsv[1],
            MemReservation {
                address: 0x5000_0000,
                size: 0x100,
            }
        );
    }

    // ---- Property iteration order -------------------------------------------

    #[test]
    fn property_iteration_order() {
        let (strings, offsets) = build_strings(&["aaa", "bbb", "ccc"]);
        let mut st = Vec::new();
        emit_begin_node(&mut st, "");
        emit_prop(&mut st, offsets[0], &be32(1));
        emit_prop(&mut st, offsets[1], &be32(2));
        emit_prop(&mut st, offsets[2], &be32(3));
        emit_end_node(&mut st);
        emit_end(&mut st);

        let dtb = build_dtb(&st, &strings, &[], 0);
        let fdt = Fdt::parse(&dtb).unwrap();
        let props: Vec<(&str, u32)> = fdt
            .root()
            .properties()
            .map(|p| (p.name(), p.as_u32().unwrap()))
            .collect();
        assert_eq!(props, &[("aaa", 1), ("bbb", 2), ("ccc", 3)]);
    }
}
