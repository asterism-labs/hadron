//! Pmt (Pinned Memory Token) kernel object.
//!
//! Represents a set of physical frames pinned for DMA. The Pmt owns the
//! IOVA mapping and physical frames — unpinning unmaps the IOVAs and
//! returns frames to the PMM.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use hadron_core::addr::PhysAddr;
use hadron_core::paging::{PhysFrame, Size4KiB};
use hadron_core::sync::SpinLock;
use hadron_iommu::domain::DomainId;
use hadron_iommu::hw::{DmaPermission, IommuError, IommuHardware};
use hadron_objects::object::{KernelObject, Koid, ObjectType, Signals};
use hadron_objects::observer::{ObserverList, PortDispatch};

/// Page size for IOVA mappings.
const PAGE_SIZE: u64 = 4096;

/// Kernel object representing pinned physical frames for DMA.
///
/// Owns both the IOVA mapping in the IOMMU and the physical frames from PMM.
/// When unpinned (explicitly or via Drop), the IOVA mapping is removed and
/// frames are returned to the PMM.
pub struct Pmt {
    /// Unique kernel object ID.
    koid: Koid,
    /// Index of the IOMMU unit managing this mapping.
    iommu_index: usize,
    /// Domain ID of the Bti that created this Pmt.
    domain_id: DomainId,
    /// Starting IOVA of the mapping.
    iova_base: u64,
    /// Physical frames owned by this Pmt (taken on unpin).
    phys_frames: SpinLock<Vec<PhysAddr>>,
    /// DMA permission bits for this mapping.
    #[allow(dead_code)] // Phase 4d: used by driver quarantine logic
    perm: DmaPermission,
    /// Whether the mapping has been unpinned.
    unpinned: AtomicBool,
    /// Per-object signal state.
    signals: AtomicU32,
    /// Observer registrations.
    observers: ObserverList,
}

impl Pmt {
    /// Create a new Pmt owning the given IOVA mapping and physical frames.
    pub(super) fn new(
        iommu_index: usize,
        domain_id: DomainId,
        iova_base: u64,
        phys_addrs: Vec<PhysAddr>,
        perm: DmaPermission,
    ) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            iommu_index,
            domain_id,
            iova_base,
            phys_frames: SpinLock::new(phys_addrs),
            perm,
            unpinned: AtomicBool::new(false),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Unpin the DMA mapping: unmap IOVAs and return frames to PMM.
    ///
    /// This is idempotent — calling it after the Pmt is already unpinned is a no-op.
    pub fn unpin(&self) -> Result<(), IommuError> {
        // Prevent double-unpin.
        if self
            .unpinned
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }

        // Take ownership of the frames.
        let frames = core::mem::take(&mut *self.phys_frames.lock());
        let page_count = frames.len();

        if page_count > 0 {
            // Unmap from IOMMU.
            let _ = hadron_iommu::with_unit(self.iommu_index, |unit| {
                unit.unmap_pages(self.domain_id, self.iova_base, page_count)
            });

            // Return frames to PMM.
            hadron_mm::pmm::with(|pmm| {
                for phys in frames {
                    let frame = PhysFrame::<Size4KiB>::containing_address(phys);
                    // SAFETY: These frames were allocated by bti::allocate_frames()
                    // and are no longer mapped anywhere.
                    unsafe {
                        let _ = pmm.deallocate_frame(frame);
                    }
                }
            });
        }

        Ok(())
    }

    /// Returns the physical addresses of the pinned frames.
    ///
    /// Returns an empty vec if the Pmt has been unpinned.
    #[must_use]
    pub fn phys_addrs(&self) -> Vec<u64> {
        self.phys_frames.lock().iter().map(|p| p.as_u64()).collect()
    }

    /// Returns the starting IOVA of this mapping.
    #[must_use]
    pub fn iova_base(&self) -> u64 {
        self.iova_base
    }
}

impl Drop for Pmt {
    fn drop(&mut self) {
        // Safety net: unpin if not already done.
        let _ = self.unpin();
    }
}

impl KernelObject for Pmt {
    fn object_type(&self) -> ObjectType {
        ObjectType::Pmt
    }

    fn koid(&self) -> Koid {
        self.koid
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
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
