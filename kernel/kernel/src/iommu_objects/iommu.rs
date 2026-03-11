//! Iommu kernel object.
//!
//! Wraps a VT-d hardware unit index. Userspace uses this handle to create
//! [`Bti`](super::bti::Bti) objects for specific PCI devices.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_objects::object::{KernelObject, Koid, ObjectType, Signals};
use hadron_objects::observer::{ObserverList, PortDispatch};

/// Kernel object representing a single IOMMU hardware unit.
pub struct Iommu {
    /// Unique kernel object ID.
    koid: Koid,
    /// Index into the global `VTD_UNITS` vector.
    unit_index: usize,
    /// Per-object signal state.
    signals: AtomicU32,
    /// Observer registrations for signal notifications.
    observers: ObserverList,
}

impl Iommu {
    /// Create a new `Iommu` object wrapping the VT-d unit at `unit_index`.
    #[must_use]
    pub fn new(unit_index: usize) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            unit_index,
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Returns the VT-d unit index.
    #[must_use]
    pub fn unit_index(&self) -> usize {
        self.unit_index
    }
}

impl KernelObject for Iommu {
    fn object_type(&self) -> ObjectType {
        ObjectType::Iommu
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
