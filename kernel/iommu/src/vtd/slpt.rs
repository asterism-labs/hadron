//! VT-d Second-Level Page Table (SLPT).
//!
//! Implements the 4-level page table used for DMA address translation. SLPT
//! entries use VT-d permission bits (bit 0 = Read, bit 1 = Write), which
//! differ from the CPU page table format.
//!
//! Reference: Intel VT-d Specification, Section 9.6 — Second-Level Translation.

use hadron_core::addr::PhysAddr;

use crate::hw::{DmaPermission, IommuError};

use super::tables::{self, AddressWidth};

/// Page size for IOVA mappings (4 KiB).
const PAGE_SIZE: u64 = 4096;

/// Mask for extracting the physical address from an SLPT entry.
const ADDR_MASK: u64 = 0x000F_FFFF_FFFF_F000;

/// Number of entries per page table level (512 entries * 8 bytes = 4096).
const ENTRIES_PER_TABLE: usize = 512;

/// SLPT entry: read permission (bit 0).
const SLPTE_READ: u64 = 1 << 0;

/// SLPT entry: write permission (bit 1).
const SLPTE_WRITE: u64 = 1 << 1;

/// VT-d second-level page table for a single domain.
///
/// Manages the 4-level IOVA→physical address translation for one DMA domain.
/// Intermediate table pages are allocated on demand and freed when the `Slpt`
/// is dropped.
pub struct Slpt {
    /// Physical address of the PML4 (root) table.
    root_phys: PhysAddr,
    /// Address width (number of page table levels).
    agaw: AddressWidth,
}

impl Slpt {
    /// Create a new SLPT with a zeroed PML4 table.
    #[must_use]
    pub fn new(agaw: AddressWidth) -> Self {
        let root_phys = tables::alloc_table_frame();
        Self { root_phys, agaw }
    }

    /// Returns the physical address of the PML4 table.
    #[must_use]
    pub fn root_phys(&self) -> PhysAddr {
        self.root_phys
    }

    /// Returns the address width of this page table.
    #[must_use]
    pub fn agaw(&self) -> AddressWidth {
        self.agaw
    }

    /// Map a single 4 KiB page at `iova` to physical address `phys`.
    ///
    /// Allocates intermediate page tables as needed.
    pub fn map_page(&mut self, iova: u64, phys: PhysAddr, perm: DmaPermission) {
        debug_assert!(iova & 0xFFF == 0, "IOVA not page-aligned");
        debug_assert!(phys.as_u64() & 0xFFF == 0, "phys not page-aligned");

        let levels = self.level_count();
        let perm_bits = perm_to_bits(perm);
        let mut table_phys = self.root_phys;

        // Walk levels top-down, allocating intermediate tables as needed.
        for level in (1..levels).rev() {
            let index = iova_index(iova, level);
            let entry_ptr = entry_ptr(table_phys, index);

            // SAFETY: The table frame was allocated and zeroed, and the entry
            // pointer is within the HHDM-mapped region.
            let entry = unsafe { core::ptr::read_volatile(entry_ptr) };

            if entry & (SLPTE_READ | SLPTE_WRITE) == 0 {
                // No next-level table — allocate one.
                let new_table = tables::alloc_table_frame();
                let new_entry = (new_table.as_u64() & ADDR_MASK) | SLPTE_READ | SLPTE_WRITE;
                // SAFETY: Writing to a valid SLPT entry within an allocated table.
                unsafe { core::ptr::write_volatile(entry_ptr, new_entry) };
                table_phys = new_table;
            } else {
                table_phys = PhysAddr::new(entry & ADDR_MASK);
            }
        }

        // Write the leaf entry (level 0).
        let index = iova_index(iova, 0);
        let leaf_entry = (phys.as_u64() & ADDR_MASK) | perm_bits;
        let entry_ptr = entry_ptr(table_phys, index);
        // SAFETY: Writing to a valid SLPT leaf entry within an allocated table.
        unsafe { core::ptr::write_volatile(entry_ptr, leaf_entry) };
    }

    /// Unmap a single 4 KiB page at `iova`.
    ///
    /// Clears the leaf entry. Does not free intermediate tables (they are
    /// freed when the entire `Slpt` is dropped).
    pub fn unmap_page(&mut self, iova: u64) {
        debug_assert!(iova & 0xFFF == 0, "IOVA not page-aligned");

        let levels = self.level_count();
        let mut table_phys = self.root_phys;

        // Walk to the leaf table.
        for level in (1..levels).rev() {
            let index = iova_index(iova, level);
            let entry_ptr = entry_ptr(table_phys, index);
            // SAFETY: Reading a valid SLPT entry within an allocated table.
            let entry = unsafe { core::ptr::read_volatile(entry_ptr) };

            if entry & (SLPTE_READ | SLPTE_WRITE) == 0 {
                // Not mapped — nothing to unmap.
                return;
            }
            table_phys = PhysAddr::new(entry & ADDR_MASK);
        }

        // Clear the leaf entry.
        let index = iova_index(iova, 0);
        let entry_ptr = entry_ptr(table_phys, index);
        // SAFETY: Clearing a valid SLPT leaf entry within an allocated table.
        unsafe { core::ptr::write_volatile(entry_ptr, 0) };
    }

    /// Map consecutive IOVAs starting at `iova_base` to the given physical frames.
    pub fn map_pages(
        &mut self,
        iova_base: u64,
        frames: &[PhysAddr],
        perm: DmaPermission,
    ) -> Result<(), IommuError> {
        for (i, &phys) in frames.iter().enumerate() {
            let iova = iova_base + (i as u64) * PAGE_SIZE;
            self.map_page(iova, phys, perm);
        }
        Ok(())
    }

    /// Unmap `count` consecutive pages starting at `iova_base`.
    pub fn unmap_pages(&mut self, iova_base: u64, count: usize) -> Result<(), IommuError> {
        for i in 0..count {
            let iova = iova_base + (i as u64) * PAGE_SIZE;
            self.unmap_page(iova);
        }
        Ok(())
    }

    /// Returns the number of page table levels for this AGAW.
    fn level_count(&self) -> usize {
        match self.agaw {
            AddressWidth::Agaw30 => 2,
            AddressWidth::Agaw39 => 3,
            AddressWidth::Agaw48 => 4,
        }
    }
}

impl Drop for Slpt {
    fn drop(&mut self) {
        let levels = self.level_count();
        // SAFETY: We own all table pages and no concurrent access is possible
        // during drop (the domain has been freed).
        unsafe { free_table_recursive(self.root_phys, levels - 1) };
    }
}

/// Recursively free all intermediate table pages at `level` and below.
///
/// `level` is 0-indexed from the leaf: level 0 = leaf table (don't recurse),
/// level 1+ = intermediate table (recurse into children).
///
/// # Safety
///
/// The caller must ensure exclusive ownership of all referenced table pages.
unsafe fn free_table_recursive(table_phys: PhysAddr, level: usize) {
    if level > 0 {
        // Scan this table's entries for child tables.
        for i in 0..ENTRIES_PER_TABLE {
            let entry_ptr = entry_ptr(table_phys, i);
            // SAFETY: The table is within HHDM and we have exclusive access.
            let entry = unsafe { core::ptr::read_volatile(entry_ptr) };

            if entry & (SLPTE_READ | SLPTE_WRITE) != 0 {
                let child_phys = PhysAddr::new(entry & ADDR_MASK);
                // SAFETY: Recursive free of owned child table.
                unsafe { free_table_recursive(child_phys, level - 1) };
            }
        }
    }

    // Free this table's frame.
    free_table_frame(table_phys);
}

/// Free a single table frame back to the PMM.
fn free_table_frame(phys: PhysAddr) {
    use hadron_core::paging::{PhysFrame, Size4KiB};

    hadron_mm::pmm::with(|pmm| {
        let frame = PhysFrame::<Size4KiB>::containing_address(phys);
        // SAFETY: The frame was allocated by alloc_table_frame() and is no
        // longer referenced by any SLPT entry.
        unsafe {
            pmm.deallocate_frame(frame)
                .expect("SLPT frame dealloc failed")
        };
    });
}

/// Convert `DmaPermission` to SLPT entry permission bits.
fn perm_to_bits(perm: DmaPermission) -> u64 {
    let mut bits = 0u64;
    if perm.read {
        bits |= SLPTE_READ;
    }
    if perm.write {
        bits |= SLPTE_WRITE;
    }
    bits
}

/// Extract the page table index for `iova` at the given `level`.
///
/// Level 0 = PTE (bits 20:12), level 1 = PDE (bits 29:21), etc.
fn iova_index(iova: u64, level: usize) -> usize {
    ((iova >> (12 + level * 9)) & 0x1FF) as usize
}

/// Returns a mutable pointer to the SLPT entry at `index` in the table at `table_phys`.
fn entry_ptr(table_phys: PhysAddr, index: usize) -> *mut u64 {
    debug_assert!(index < ENTRIES_PER_TABLE, "SLPT index out of bounds");
    let virt = hadron_mm::hhdm::phys_to_virt(table_phys);
    // SAFETY: The table is 4096 bytes and index < 512, so offset is within bounds.
    // The HHDM mapping covers this physical address.
    (virt.as_u64() as *mut u64).wrapping_add(index)
}
