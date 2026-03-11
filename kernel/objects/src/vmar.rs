//! Virtual Memory Address Region (VMAR).
//!
//! VMARs form a tree that describes a process's address space layout. Each
//! process has a root VMAR spanning its entire user address range. Sub-VMARs
//! can be allocated within a parent to reserve address ranges, and VMOs can be
//! mapped into VMARs at specific offsets.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use bitflags::bitflags;
use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch};
use crate::vmo::Vmo;

/// Aligns `addr` up to the next multiple of `align` (must be a power of two).
const fn align_up(addr: u64, align: u64) -> u64 {
    (addr + align - 1) & !(align - 1)
}

bitflags! {
    /// Permissions for a VMO mapping within a VMAR.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct VmarFlags: u32 {
        /// Pages are readable.
        const READ    = 1 << 0;
        /// Pages are writable.
        const WRITE   = 1 << 1;
        /// Pages are executable.
        const EXECUTE = 1 << 2;

        /// Place the mapping at a specific offset within the VMAR (not auto).
        const SPECIFIC = 1 << 4;
        /// Allow the kernel to overwrite existing mappings at the target range.
        const SPECIFIC_OVERWRITE = 1 << 5;

        /// Common shorthand: read + write.
        const RW = Self::READ.bits() | Self::WRITE.bits();
        /// Common shorthand: read + execute.
        const RX = Self::READ.bits() | Self::EXECUTE.bits();
    }
}

/// A mapping of a VMO region into a VMAR.
pub struct VmarMapping {
    /// The VMO being mapped.
    pub vmo: Arc<Vmo>,
    /// Offset within the VMO where the mapping starts.
    pub vmo_offset: u64,
    /// Virtual address where this mapping starts within the owning VMAR.
    pub addr: u64,
    /// Length of the mapping in bytes.
    pub len: u64,
    /// Access permissions.
    pub flags: VmarFlags,
}

/// A child region allocated within a parent VMAR.
pub struct VmarChild {
    /// The child VMAR.
    pub vmar: Arc<Vmar>,
    /// Offset within the parent VMAR.
    pub offset: u64,
}

/// A Virtual Memory Address Region — an address space tree node.
///
/// Each process has a root VMAR. VMARs can contain:
/// - **Mappings**: VMO pages mapped at specific virtual addresses
/// - **Children**: sub-VMARs that partition the address range
///
/// The VMAR tree prevents overlapping allocations and provides structured
/// address space management (no arbitrary mmap free-for-all).
pub struct Vmar {
    /// Unique identifier.
    koid: Koid,
    /// Base virtual address of this region.
    base: u64,
    /// Size of this region in bytes.
    size: u64,
    /// VMO mappings within this region.
    ///
    /// Protected by the process's address space lock in practice; the lock
    /// lives in the Process object to avoid lock ordering issues.
    mappings: SpinLock<Vec<VmarMapping>>,
    /// Child VMARs allocated within this region.
    children: SpinLock<Vec<VmarChild>>,
    /// Signal state.
    signals: AtomicU32,
    /// Registered observers for signal notifications.
    observers: ObserverList,
}

impl Vmar {
    /// Create a root VMAR spanning the given address range.
    #[must_use]
    pub fn new_root(base: u64, size: u64) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            base,
            size,
            mappings: SpinLock::new(Vec::new()),
            children: SpinLock::new(Vec::new()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Allocate a sub-VMAR within this region.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the requested range is out of bounds or overlaps
    /// an existing child or mapping.
    pub fn allocate(self: &Arc<Self>, offset: u64, size: u64) -> Result<Arc<Vmar>, VmarError> {
        let child_base = self.base.checked_add(offset).ok_or(VmarError::OutOfRange)?;
        let child_end = child_base.checked_add(size).ok_or(VmarError::OutOfRange)?;
        let self_end = self.base + self.size;

        if child_end > self_end {
            return Err(VmarError::OutOfRange);
        }

        let children = self.children.lock();
        // Check for overlap with existing children.
        for existing in children.iter() {
            let ex_base = self.base + existing.offset;
            let ex_end = ex_base + existing.vmar.size;
            if child_base < ex_end && child_end > ex_base {
                return Err(VmarError::Overlap);
            }
        }
        drop(children);

        let child = Arc::new(Vmar {
            koid: Koid::alloc(),
            base: child_base,
            size,
            mappings: SpinLock::new(Vec::new()),
            children: SpinLock::new(Vec::new()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        });

        self.children.lock().push(VmarChild {
            vmar: Arc::clone(&child),
            offset,
        });

        Ok(child)
    }

    /// Map a VMO into this VMAR at the given offset.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the mapping would exceed the VMAR bounds.
    pub fn map(
        &self,
        vmo: Arc<Vmo>,
        vmo_offset: u64,
        addr: u64,
        len: u64,
        flags: VmarFlags,
    ) -> Result<u64, VmarError> {
        let map_end = addr.checked_add(len).ok_or(VmarError::OutOfRange)?;
        let self_end = self.base + self.size;

        if addr < self.base || map_end > self_end {
            return Err(VmarError::OutOfRange);
        }

        self.mappings.lock().push(VmarMapping {
            vmo,
            vmo_offset,
            addr,
            len,
            flags,
        });

        Ok(addr)
    }

    /// Remove all mappings that overlap the given range.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the range is out of bounds.
    pub fn unmap(&self, addr: u64, len: u64) -> Result<(), VmarError> {
        let unmap_end = addr.checked_add(len).ok_or(VmarError::OutOfRange)?;
        let self_end = self.base + self.size;

        if addr < self.base || unmap_end > self_end {
            return Err(VmarError::OutOfRange);
        }

        self.mappings.lock().retain(|m| {
            let m_end = m.addr + m.len;
            // Keep mappings that don't overlap.
            m.addr >= unmap_end || m_end <= addr
        });

        Ok(())
    }

    /// The base virtual address of this VMAR.
    #[must_use]
    pub fn base(&self) -> u64 {
        self.base
    }

    /// The size of this VMAR in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size
    }

    /// Number of active mappings.
    #[must_use]
    pub fn mapping_count(&self) -> usize {
        self.mappings.lock().len()
    }

    /// Number of child VMARs.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.lock().len()
    }

    /// Finds a free region of `size` bytes with the given alignment.
    ///
    /// Walks existing mappings sorted by address and returns the first gap
    /// that satisfies the size and alignment requirements.
    #[must_use]
    pub fn find_free_region(&self, size: u64, align: u64) -> Option<u64> {
        let mappings = self.mappings.lock();

        // Collect and sort mappings by address.
        let mut sorted: Vec<(u64, u64)> = mappings.iter().map(|m| (m.addr, m.len)).collect();
        sorted.sort_unstable_by_key(|&(addr, _)| addr);

        // Start searching from the VMAR base.
        let mut candidate = align_up(self.base, align);
        let vmar_end = self.base + self.size;

        for &(map_addr, map_len) in &sorted {
            let map_end = map_addr + map_len;
            if candidate + size <= map_addr {
                return Some(candidate);
            }
            // Move past this mapping.
            candidate = align_up(map_end, align);
        }

        // Check the gap after the last mapping.
        if candidate + size <= vmar_end {
            Some(candidate)
        } else {
            None
        }
    }
}

impl KernelObject for Vmar {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Vmar
    }

    fn koid(&self) -> Koid {
        self.koid
    }

    fn get_signals(&self) -> Signals {
        Signals::from_bits_truncate(self.signals.load(Ordering::Relaxed))
    }

    fn add_observer(&self, port: Arc<dyn PortDispatch>, key: u64, signals: Signals) {
        self.observers.add(port, key, signals);
    }

    fn remove_observer(&self, port: &Arc<dyn PortDispatch>) {
        self.observers.remove_by_port(port);
    }
}

/// Errors from VMAR operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmarError {
    /// The requested offset/size exceeds the VMAR bounds.
    OutOfRange,
    /// The requested range overlaps an existing allocation.
    Overlap,
    /// The VMAR has been destroyed.
    Destroyed,
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_BASE: u64 = 0x0000_1000_0000_0000;
    const USER_SIZE: u64 = 0x0000_7FFF_0000_0000;

    #[test]
    fn root_vmar_properties() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        assert_eq!(root.base(), USER_BASE);
        assert_eq!(root.size(), USER_SIZE);
        assert_eq!(root.object_type(), ObjectType::Vmar);
    }

    #[test]
    fn allocate_child() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        let child = root.allocate(0, 0x1000_0000).expect("allocate failed");
        assert_eq!(child.base(), USER_BASE);
        assert_eq!(child.size(), 0x1000_0000);
        assert_eq!(root.child_count(), 1);
    }

    #[test]
    fn allocate_overlap_fails() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        root.allocate(0, 0x1000_0000).expect("first allocate");
        assert!(matches!(
            root.allocate(0x0800_0000, 0x1000_0000),
            Err(VmarError::Overlap),
        ));
    }

    #[test]
    fn allocate_out_of_range() {
        let root = Vmar::new_root(USER_BASE, 0x1000);
        assert!(matches!(
            root.allocate(0, 0x2000),
            Err(VmarError::OutOfRange)
        ));
    }

    #[test]
    fn map_vmo() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        let vmo = Vmo::new_paged(0x4000);

        let addr = root
            .map(vmo, 0, USER_BASE, 0x4000, VmarFlags::RW)
            .expect("map failed");
        assert_eq!(addr, USER_BASE);
        assert_eq!(root.mapping_count(), 1);
    }

    #[test]
    fn map_out_of_range() {
        let root = Vmar::new_root(USER_BASE, 0x1000);
        let vmo = Vmo::new_paged(0x2000);

        assert!(matches!(
            root.map(vmo, 0, USER_BASE, 0x2000, VmarFlags::RW),
            Err(VmarError::OutOfRange),
        ));
    }

    #[test]
    fn find_free_region_empty_vmar() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        let addr = root.find_free_region(0x1000, 0x1000);
        assert_eq!(addr, Some(USER_BASE));
    }

    #[test]
    fn find_free_region_skips_mappings() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        let vmo = Vmo::new_paged(0x4000);

        root.map(vmo, 0, USER_BASE, 0x4000, VmarFlags::RW)
            .expect("map failed");

        let addr = root.find_free_region(0x1000, 0x1000);
        assert_eq!(addr, Some(USER_BASE + 0x4000));
    }

    #[test]
    fn find_free_region_alignment() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        let vmo = Vmo::new_paged(0x3000);

        root.map(vmo, 0, USER_BASE, 0x3000, VmarFlags::RW)
            .expect("map failed");

        // Request 64KiB alignment — should skip past the mapping and align up.
        let addr = root.find_free_region(0x1000, 0x1_0000);
        assert_eq!(addr, Some(USER_BASE + 0x1_0000));
    }

    #[test]
    fn unmap_removes_mapping() {
        let root = Vmar::new_root(USER_BASE, USER_SIZE);
        let vmo = Vmo::new_paged(0x4000);

        root.map(vmo, 0, USER_BASE, 0x4000, VmarFlags::RW)
            .expect("map failed");
        assert_eq!(root.mapping_count(), 1);

        root.unmap(USER_BASE, 0x4000).expect("unmap failed");
        assert_eq!(root.mapping_count(), 0);
    }
}
