//! Resource object — hierarchical capability tree for hardware access.
//!
//! Resources represent authority over hardware regions (IRQs, MMIO, I/O ports).
//! The root resource is omnipotent; child resources are restricted subsets of
//! their parent. This prevents userspace from accessing hardware without
//! explicit capability grants.

use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch};

/// The kind of hardware resource this object represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// Root resource — grants authority over everything.
    Root,
    /// IRQ range.
    Irq {
        /// First IRQ vector in the range.
        base: u32,
        /// Number of vectors.
        count: u32,
    },
    /// Memory-mapped I/O region.
    Mmio {
        /// Physical base address.
        base: u64,
        /// Size in bytes.
        size: u64,
    },
    /// x86 I/O port range.
    IoPort {
        /// First port number.
        base: u16,
        /// Number of ports.
        count: u16,
    },
    /// System-level resource (e.g., power management, firmware tables).
    System,
}

/// Errors from resource operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceError {
    /// The requested child range exceeds the parent's range.
    OutOfRange,
    /// Cannot create children from a non-root resource of different kind.
    KindMismatch,
}

/// A resource — a node in the capability tree for hardware access.
///
/// The root resource (created at boot) is the ultimate authority. Drivers
/// receive child resources restricted to only the hardware they need.
pub struct Resource {
    /// Unique identifier.
    koid: Koid,
    /// What kind of hardware this resource grants access to.
    kind: ResourceKind,
    /// Parent resource (weak to avoid cycles). `None` for root.
    parent: Option<Weak<Resource>>,
    /// Child resources carved from this one.
    children: SpinLock<Vec<Arc<Resource>>>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Resource {
    /// Create the root resource (omnipotent).
    #[must_use]
    pub fn create_root() -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            kind: ResourceKind::Root,
            parent: None,
            children: SpinLock::new(Vec::new()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Create a child resource restricted to the given kind/range.
    ///
    /// # Errors
    ///
    /// - [`ResourceError::OutOfRange`] if the child range exceeds the parent
    /// - [`ResourceError::KindMismatch`] if the parent is not root and the
    ///   child kind differs
    pub fn create_child(self: &Arc<Self>, kind: ResourceKind) -> Result<Arc<Self>, ResourceError> {
        self.validate_child(&kind)?;

        let child = Arc::new(Resource {
            koid: Koid::alloc(),
            kind,
            parent: Some(Arc::downgrade(self)),
            children: SpinLock::new(Vec::new()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        });

        self.children.lock().push(Arc::clone(&child));
        Ok(child)
    }

    /// Validate that a child resource is within the parent's range.
    fn validate_child(&self, child_kind: &ResourceKind) -> Result<(), ResourceError> {
        match &self.kind {
            // Root can create any kind of child.
            ResourceKind::Root => Ok(()),
            // Non-root: child must be same kind and within range.
            ResourceKind::Irq { base, count } => {
                if let ResourceKind::Irq {
                    base: cb,
                    count: cc,
                } = child_kind
                {
                    if *cb >= *base && cb.saturating_add(*cc) <= base.saturating_add(*count) {
                        Ok(())
                    } else {
                        Err(ResourceError::OutOfRange)
                    }
                } else {
                    Err(ResourceError::KindMismatch)
                }
            }
            ResourceKind::Mmio { base, size } => {
                if let ResourceKind::Mmio { base: cb, size: cs } = child_kind {
                    if *cb >= *base && cb.saturating_add(*cs) <= base.saturating_add(*size) {
                        Ok(())
                    } else {
                        Err(ResourceError::OutOfRange)
                    }
                } else {
                    Err(ResourceError::KindMismatch)
                }
            }
            ResourceKind::IoPort { base, count } => {
                if let ResourceKind::IoPort {
                    base: cb,
                    count: cc,
                } = child_kind
                {
                    if *cb >= *base
                        && (u32::from(*cb) + u32::from(*cc))
                            <= (u32::from(*base) + u32::from(*count))
                    {
                        Ok(())
                    } else {
                        Err(ResourceError::OutOfRange)
                    }
                } else {
                    Err(ResourceError::KindMismatch)
                }
            }
            ResourceKind::System => {
                if matches!(child_kind, ResourceKind::System) {
                    Ok(())
                } else {
                    Err(ResourceError::KindMismatch)
                }
            }
        }
    }

    /// The kind of resource.
    #[must_use]
    pub fn kind(&self) -> ResourceKind {
        self.kind
    }

    /// The parent resource, if any.
    #[must_use]
    pub fn parent(&self) -> Option<Arc<Resource>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }

    /// Number of child resources.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.lock().len()
    }
}

impl KernelObject for Resource {
    fn object_type(&self) -> ObjectType {
        ObjectType::Resource
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_create_root() {
        let root = Resource::create_root();
        assert_eq!(root.object_type(), ObjectType::Resource);
        assert_eq!(root.kind(), ResourceKind::Root);
        assert!(root.parent().is_none());
    }

    #[test]
    fn resource_root_creates_irq_child() {
        let root = Resource::create_root();
        let irq = root
            .create_child(ResourceKind::Irq {
                base: 32,
                count: 16,
            })
            .unwrap();
        assert_eq!(
            irq.kind(),
            ResourceKind::Irq {
                base: 32,
                count: 16
            }
        );
        assert_eq!(root.child_count(), 1);
    }

    #[test]
    fn resource_subdivide_irq() {
        let root = Resource::create_root();
        let parent_irq = root
            .create_child(ResourceKind::Irq {
                base: 32,
                count: 16,
            })
            .unwrap();
        let child_irq = parent_irq
            .create_child(ResourceKind::Irq { base: 32, count: 8 })
            .unwrap();
        assert_eq!(child_irq.kind(), ResourceKind::Irq { base: 32, count: 8 });
    }

    #[test]
    fn resource_irq_out_of_range() {
        let root = Resource::create_root();
        let parent = root
            .create_child(ResourceKind::Irq { base: 32, count: 8 })
            .unwrap();
        // Child exceeds parent range.
        assert!(matches!(
            parent.create_child(ResourceKind::Irq {
                base: 32,
                count: 16
            }),
            Err(ResourceError::OutOfRange)
        ));
    }

    #[test]
    fn resource_kind_mismatch() {
        let root = Resource::create_root();
        let irq = root
            .create_child(ResourceKind::Irq {
                base: 0,
                count: 256,
            })
            .unwrap();
        assert!(matches!(
            irq.create_child(ResourceKind::Mmio {
                base: 0,
                size: 4096
            }),
            Err(ResourceError::KindMismatch)
        ));
    }

    #[test]
    fn resource_root_creates_mmio_child() {
        let root = Resource::create_root();
        let mmio = root
            .create_child(ResourceKind::Mmio {
                base: 0xFEE0_0000,
                size: 0x1000,
            })
            .unwrap();
        assert_eq!(
            mmio.kind(),
            ResourceKind::Mmio {
                base: 0xFEE0_0000,
                size: 0x1000
            }
        );
    }

    #[test]
    fn resource_root_creates_ioport_child() {
        let root = Resource::create_root();
        let io = root
            .create_child(ResourceKind::IoPort {
                base: 0x3F8,
                count: 8,
            })
            .unwrap();
        assert_eq!(
            io.kind(),
            ResourceKind::IoPort {
                base: 0x3F8,
                count: 8
            }
        );
    }
}
