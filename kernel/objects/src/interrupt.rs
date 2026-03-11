//! Interrupt object — hardware IRQ delivery to userspace.
//!
//! An interrupt object represents a bound hardware IRQ vector. When the
//! hardware fires the interrupt, the kernel calls [`Interrupt::trigger`],
//! which asserts `SIGNAL_0` and notifies observers. Userspace acknowledges
//! the interrupt to re-arm it.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};
use crate::resource::{Resource, ResourceKind};

/// Flags controlling interrupt behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterruptFlags {
    /// Whether this is a level-triggered interrupt (requires explicit ack).
    pub level_triggered: bool,
}

impl Default for InterruptFlags {
    fn default() -> Self {
        Self {
            level_triggered: false,
        }
    }
}

/// Errors from interrupt operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptError {
    /// The resource does not grant access to the requested IRQ vector.
    AccessDenied,
    /// The interrupt has not fired.
    NotPending,
}

/// Internal state of the interrupt object.
struct InterruptState {
    /// The IRQ vector this interrupt is bound to.
    vector: u32,
    /// Whether a trigger is pending acknowledgment.
    pending: bool,
    /// Configuration flags.
    #[allow(dead_code)] // Phase 6: used by interrupt controller integration
    flags: InterruptFlags,
}

/// An interrupt object — delivers hardware IRQs to userspace.
///
/// Created with a resource that grants access to the IRQ vector.
/// The kernel's interrupt handler calls [`trigger`](Interrupt::trigger) when
/// the hardware fires. Userspace waits on the object and calls
/// [`ack`](Interrupt::ack) to re-arm (for level-triggered interrupts).
pub struct Interrupt {
    /// Unique identifier.
    koid: Koid,
    /// Internal state.
    state: SpinLock<InterruptState>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Interrupt {
    /// Create an interrupt object bound to an IRQ vector.
    ///
    /// The provided resource must grant access to the vector (IRQ kind with
    /// the vector in range, or Root resource).
    ///
    /// # Errors
    ///
    /// Returns [`InterruptError::AccessDenied`] if the resource does not
    /// cover the requested vector.
    pub fn create(
        resource: &Arc<Resource>,
        vector: u32,
        flags: InterruptFlags,
    ) -> Result<Arc<Self>, InterruptError> {
        // Validate the resource covers the requested vector.
        match resource.kind() {
            ResourceKind::Root => {} // Root can bind anything.
            ResourceKind::Irq { base, count } => {
                if vector < base || vector >= base.saturating_add(count) {
                    return Err(InterruptError::AccessDenied);
                }
            }
            _ => return Err(InterruptError::AccessDenied),
        }

        Ok(Arc::new(Self {
            koid: Koid::alloc(),
            state: SpinLock::new(InterruptState {
                vector,
                pending: false,
                flags,
            }),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        }))
    }

    /// Called by the kernel IRQ handler when the hardware interrupt fires.
    ///
    /// Asserts `SIGNAL_0` and marks the interrupt as pending.
    pub fn trigger(&self) {
        self.state.lock().pending = true;
        signal_update(
            &self.signals,
            Signals::SIGNAL_0,
            Signals::empty(),
            &self.observers,
            self.koid,
        );
    }

    /// Acknowledge the interrupt (re-arm for level-triggered).
    ///
    /// Clears `SIGNAL_0` and the pending flag.
    ///
    /// # Errors
    ///
    /// Returns [`InterruptError::NotPending`] if no interrupt is pending.
    pub fn ack(&self) -> Result<(), InterruptError> {
        let mut state = self.state.lock();
        if !state.pending {
            return Err(InterruptError::NotPending);
        }
        state.pending = false;
        drop(state);

        self.signals
            .fetch_and(!Signals::SIGNAL_0.bits(), Ordering::Release);
        Ok(())
    }

    /// The IRQ vector this interrupt is bound to.
    #[must_use]
    pub fn vector(&self) -> u32 {
        self.state.lock().vector
    }

    /// Whether an interrupt is pending acknowledgment.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.state.lock().pending
    }
}

impl KernelObject for Interrupt {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Interrupt
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
    fn interrupt_create_with_root_resource() {
        let root = Resource::create_root();
        let intr = Interrupt::create(&root, 33, InterruptFlags::default()).unwrap();
        assert_eq!(intr.object_type(), ObjectType::Interrupt);
        assert_eq!(intr.vector(), 33);
        assert!(!intr.is_pending());
    }

    #[test]
    fn interrupt_create_with_irq_resource() {
        let root = Resource::create_root();
        let irq_res = root
            .create_child(ResourceKind::Irq {
                base: 32,
                count: 16,
            })
            .unwrap();
        let intr = Interrupt::create(&irq_res, 40, InterruptFlags::default()).unwrap();
        assert_eq!(intr.vector(), 40);
    }

    #[test]
    fn interrupt_access_denied_wrong_resource() {
        let root = Resource::create_root();
        let mmio = root
            .create_child(ResourceKind::Mmio {
                base: 0,
                size: 4096,
            })
            .unwrap();
        assert!(matches!(
            Interrupt::create(&mmio, 33, InterruptFlags::default()),
            Err(InterruptError::AccessDenied)
        ));
    }

    #[test]
    fn interrupt_access_denied_out_of_range() {
        let root = Resource::create_root();
        let irq_res = root
            .create_child(ResourceKind::Irq { base: 32, count: 8 })
            .unwrap();
        // Vector 50 is out of the 32..40 range.
        assert!(matches!(
            Interrupt::create(&irq_res, 50, InterruptFlags::default()),
            Err(InterruptError::AccessDenied)
        ));
    }

    #[test]
    fn interrupt_trigger_and_ack() {
        let root = Resource::create_root();
        let intr = Interrupt::create(&root, 33, InterruptFlags::default()).unwrap();

        intr.trigger();
        assert!(intr.is_pending());
        assert!(intr.get_signals().contains(Signals::SIGNAL_0));

        intr.ack().unwrap();
        assert!(!intr.is_pending());
        assert!(!intr.get_signals().contains(Signals::SIGNAL_0));
    }

    #[test]
    fn interrupt_ack_without_pending() {
        let root = Resource::create_root();
        let intr = Interrupt::create(&root, 33, InterruptFlags::default()).unwrap();
        assert_eq!(intr.ack(), Err(InterruptError::NotPending));
    }

    #[test]
    fn interrupt_trigger_notifies_observer() {
        use alloc::sync::Arc;
        use alloc::vec::Vec;
        use hadron_core::sync::SpinLock;

        use crate::port_packet::PortPacket;

        struct MockPort {
            packets: SpinLock<Vec<PortPacket>>,
        }
        impl MockPort {
            fn new() -> Arc<Self> {
                Arc::new(Self {
                    packets: SpinLock::new(Vec::new()),
                })
            }
        }
        impl PortDispatch for MockPort {
            fn queue_packet(&self, packet: PortPacket) {
                self.packets.lock().push(packet);
            }
        }

        let root = Resource::create_root();
        let intr = Interrupt::create(&root, 33, InterruptFlags::default()).unwrap();
        let port = MockPort::new();

        intr.add_observer(port.clone(), 99, Signals::SIGNAL_0);
        intr.trigger();

        assert_eq!(port.packets.lock().len(), 1);
        assert_eq!(port.packets.lock()[0].key, 99);
    }
}
