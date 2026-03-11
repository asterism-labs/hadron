//! Bti (Bus Transaction Initiator) kernel object.
//!
//! A Bti represents a DMA domain for a specific PCI device. Userspace drivers
//! create a Bti from an Iommu handle, then pin memory to get IOVA-mapped
//! physical frames for safe DMA transfers.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use hadron_core::addr::PhysAddr;
use hadron_core::paging::{PhysFrame, Size4KiB};
use hadron_core::sync::SpinLock;
use hadron_iommu::domain::DomainId;
use hadron_iommu::hw::{DmaPermission, IommuError, IommuHardware, PciBdf};
use hadron_objects::object::{KernelObject, Koid, ObjectType, Signals};
use hadron_objects::observer::{ObserverList, PortDispatch};

use super::pmt::Pmt;

/// Starting IOVA for the per-Bti bump allocator.
const IOVA_BASE: u64 = 0x1000;

/// Page size for IOVA mappings.
const PAGE_SIZE: u64 = 4096;

/// Kernel object representing a DMA domain for a PCI device.
///
/// Each Bti owns a domain ID and manages IOVA allocation for DMA mappings.
/// When all handles to a Bti are closed, the domain is freed and the device
/// is detached from the IOMMU.
pub struct Bti {
    /// Unique kernel object ID.
    koid: Koid,
    /// Index of the IOMMU unit that owns this domain.
    iommu_index: usize,
    /// PCI Bus/Device/Function of the attached device.
    bdf: PciBdf,
    /// Domain ID allocated from the IOMMU unit.
    domain_id: DomainId,
    /// Bump allocator for IOVA addresses.
    next_iova: SpinLock<u64>,
    /// Active pinned memory tokens.
    pmts: SpinLock<Vec<Arc<Pmt>>>,
    /// Whether this Bti is in quarantine mode (all PMTs released).
    quarantined: AtomicBool,
    /// Per-object signal state.
    signals: AtomicU32,
    /// Observer registrations.
    observers: ObserverList,
}

impl Bti {
    /// Create a new Bti, allocating a domain and attaching the device.
    pub fn new(iommu_index: usize, bdf: PciBdf) -> Result<Arc<Self>, IommuError> {
        let domain_id = hadron_iommu::with_unit(iommu_index, |unit| unit.alloc_domain())
            .ok_or(IommuError::NotInitialized)??;

        // Attach the device to the new domain.
        if let Err(e) =
            hadron_iommu::with_unit(iommu_index, |unit| unit.attach_device(domain_id, bdf))
                .ok_or(IommuError::NotInitialized)?
        {
            // Clean up domain on attach failure.
            let _ = hadron_iommu::with_unit(iommu_index, |unit| unit.free_domain(domain_id));
            return Err(e);
        }

        Ok(Arc::new(Self {
            koid: Koid::alloc(),
            iommu_index,
            bdf,
            domain_id,
            next_iova: SpinLock::new(IOVA_BASE),
            pmts: SpinLock::new(Vec::new()),
            quarantined: AtomicBool::new(false),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        }))
    }

    /// Pin `page_count` physical frames for DMA, returning a Pmt token.
    ///
    /// Allocates fresh physical frames from PMM, maps them at consecutive
    /// IOVAs in the domain's SLPT, and returns a Pmt that owns the mapping.
    pub fn pin(
        self: &Arc<Self>,
        page_count: usize,
        perm: DmaPermission,
    ) -> Result<Arc<Pmt>, IommuError> {
        // Allocate physical frames.
        let frames = allocate_frames(page_count)?;

        // Bump-allocate IOVAs.
        let iova_base = {
            let mut next = self.next_iova.lock();
            let base = *next;
            *next = base + (page_count as u64) * PAGE_SIZE;
            base
        };

        // Map pages in the IOMMU.
        let phys_addrs: Vec<PhysAddr> = frames.iter().map(|f| f.start_address()).collect();
        hadron_iommu::with_unit(self.iommu_index, |unit| {
            unit.map_pages(self.domain_id, iova_base, &phys_addrs, perm)
        })
        .ok_or(IommuError::NotInitialized)??;

        // Create the Pmt.
        let pmt = Pmt::new(
            self.iommu_index,
            self.domain_id,
            iova_base,
            phys_addrs,
            perm,
        );

        self.pmts.lock().push(Arc::clone(&pmt));
        Ok(pmt)
    }

    /// Release quarantine — allows the Bti to be reused after error recovery.
    pub fn release_quarantine(&self) {
        self.quarantined.store(false, Ordering::Release);
    }

    /// Returns the domain ID.
    #[must_use]
    pub fn domain_id(&self) -> DomainId {
        self.domain_id
    }

    /// Unpin all active PMTs and free the domain.
    fn cleanup(&self) {
        // Unpin all PMTs.
        let pmts = core::mem::take(&mut *self.pmts.lock());
        for pmt in &pmts {
            let _ = pmt.unpin();
        }
        drop(pmts);

        // Detach device and free domain.
        let _ = hadron_iommu::with_unit(self.iommu_index, |unit| unit.detach_device(self.bdf));
        let _ = hadron_iommu::with_unit(self.iommu_index, |unit| unit.free_domain(self.domain_id));
    }
}

impl KernelObject for Bti {
    fn object_type(&self) -> ObjectType {
        ObjectType::Bti
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

    fn on_zero_handles(&self) {
        self.cleanup();
    }
}

/// Allocate `count` physical frames from the PMM.
fn allocate_frames(count: usize) -> Result<Vec<PhysFrame<Size4KiB>>, IommuError> {
    hadron_mm::pmm::with(|pmm| {
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count {
            let frame = pmm.allocate_frame().ok_or(IommuError::OutOfMemory)?;
            frames.push(frame);
        }
        Ok(frames)
    })
}
