//! VT-d root table, context table, and second-level page table structures.
//!
//! The root table has 256 entries (one per PCI bus). Each root entry points
//! to a context table with 256 entries (32 devices * 8 functions). Context
//! entries contain the domain ID and second-level page table pointer.
//!
//! Reference: Intel VT-d Specification, Sections 9.1-9.3.

use hadron_core::addr::PhysAddr;

/// Number of entries in the root table (one per PCI bus).
pub const ROOT_TABLE_ENTRIES: usize = 256;

/// Number of entries in a context table (32 devices * 8 functions).
pub const CONTEXT_TABLE_ENTRIES: usize = 256;

/// A single root table entry (128 bits = two `u64`s).
///
/// Layout:
/// - Low `u64`: bit 0 = Present, bits 63:12 = Context Table Pointer (4 KiB aligned)
/// - High `u64`: Reserved (must be zero)
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct RootEntry {
    low: u64,
    high: u64,
}

impl RootEntry {
    /// An empty (not-present) root entry.
    pub const EMPTY: Self = Self { low: 0, high: 0 };

    /// Returns true if this entry is present.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.low & 1 != 0
    }

    /// Set the context table pointer and mark as present.
    ///
    /// `ct_phys` must be 4 KiB aligned.
    pub fn set_context_table(&mut self, ct_phys: PhysAddr) {
        debug_assert!(
            ct_phys.as_u64() & 0xFFF == 0,
            "context table not 4 KiB aligned"
        );
        self.low = (ct_phys.as_u64() & !0xFFF) | 1; // Set present bit
        self.high = 0;
    }
}

/// A single context table entry (128 bits = two `u64`s).
///
/// Layout (low `u64`):
/// - Bit 0: Present
/// - Bit 1: Fault Processing Disable
/// - Bits 3:2: Translation Type (00 = multi-level translation)
/// - Bits 15:12 + bits 63:12 of high: depends on TT
///
/// Layout (high `u64`):
/// - Bits 2:0: Address Width (AGAW, must match supported from CAP.SAGAW)
/// - Bits 8:3: Available
/// - Bits 23:8: Domain ID
/// - Bits 63:12: Second-Level Page Table Pointer (4 KiB aligned)
#[derive(Clone, Copy)]
#[repr(C, align(16))]
pub struct ContextEntry {
    low: u64,
    high: u64,
}

/// Address width encoding for context entries.
///
/// Selects the number of page table levels for second-level translation.
#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum AddressWidth {
    /// 30-bit AGAW: 2-level page table (rare, not commonly used).
    Agaw30 = 0b001,
    /// 39-bit AGAW: 3-level page table, 512 GiB address space.
    Agaw39 = 0b010,
    /// 48-bit AGAW: 4-level page table, 256 TiB address space.
    Agaw48 = 0b011,
}

impl ContextEntry {
    /// An empty (not-present) context entry.
    pub const EMPTY: Self = Self { low: 0, high: 0 };

    /// Returns true if this entry is present.
    #[must_use]
    pub fn is_present(&self) -> bool {
        self.low & 1 != 0
    }

    /// Configure this context entry for second-level translation.
    ///
    /// - `domain_id`: DMA domain ID (max 16 bits)
    /// - `slpt_phys`: Physical address of the second-level page table root (4 KiB aligned)
    /// - `aw`: Address width (number of page table levels)
    pub fn set_translation(&mut self, domain_id: u16, slpt_phys: PhysAddr, aw: AddressWidth) {
        debug_assert!(
            slpt_phys.as_u64() & 0xFFF == 0,
            "second-level page table not 4 KiB aligned"
        );
        // Low: Present=1, FPD=0, TT=00 (multi-level)
        self.low = 1;
        // High: AW | (domain_id << 8) | (slpt_phys & ~0xFFF)
        self.high = (aw as u64) | (u64::from(domain_id) << 8) | (slpt_phys.as_u64() & !0xFFF);
    }

    /// Returns the domain ID from this entry, if present.
    #[must_use]
    pub fn domain_id(&self) -> Option<u16> {
        if self.is_present() {
            Some(((self.high >> 8) & 0xFFFF) as u16)
        } else {
            None
        }
    }
}

/// Allocate a zeroed 4 KiB frame for use as a root or context table.
///
/// Returns the physical address of the allocated frame.
///
/// # Panics
///
/// Panics if the PMM cannot allocate a frame (OOM).
pub fn alloc_table_frame() -> PhysAddr {
    hadron_mm::pmm::with(|pmm| {
        let frame = pmm
            .allocate_frame()
            .expect("OOM: cannot allocate IOMMU table frame");
        let virt = hadron_mm::hhdm::phys_to_virt(frame.start_address());
        // SAFETY: The frame was just allocated and is identity-mapped via HHDM.
        let slice = unsafe { core::slice::from_raw_parts_mut(virt.as_u64() as *mut u8, 4096) };
        slice.fill(0);
        frame.start_address()
    })
}
