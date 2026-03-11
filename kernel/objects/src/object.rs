//! Core kernel object trait and types.
//!
//! Defines the [`KernelObject`] trait that all kernel resources implement,
//! along with [`Koid`] (kernel object ID), [`ObjectType`], and [`Signals`].

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU64, Ordering};

use bitflags::bitflags;

use crate::observer::PortDispatch;

/// Globally unique kernel object identifier.
///
/// Every kernel object receives a unique `Koid` at creation time. Koids are
/// never reused within a single boot session. They serve as stable identifiers
/// for debugging, tracing, and peer correlation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Koid(u64);

/// Global counter for allocating unique [`Koid`] values.
static NEXT_KOID: AtomicU64 = AtomicU64::new(1);

impl Koid {
    /// The invalid/null koid, used for objects with no related peer.
    pub const INVALID: Self = Self(0);

    /// Allocate a fresh, globally unique koid.
    pub fn alloc() -> Self {
        Self(NEXT_KOID.fetch_add(1, Ordering::Relaxed))
    }

    /// Return the raw `u64` value.
    #[must_use]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Discriminant for every kernel object type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum ObjectType {
    /// No type (sentinel).
    None = 0,
    /// Process — address space + handle table + thread group.
    Process = 1,
    /// Thread — schedulable execution context.
    Thread = 2,
    /// VMO — virtual memory object (physical page container).
    Vmo = 3,
    /// VMAR — virtual memory address region (address space tree node).
    Vmar = 4,
    /// Channel — bidirectional message + handle passing IPC.
    Channel = 5,
    /// Socket — streaming byte IPC.
    Socket = 6,
    /// Port — async event aggregation.
    Port = 7,
    /// Event — one-shot signaling primitive.
    Event = 8,
    /// EventPair — paired signaling primitive.
    EventPair = 9,
    /// Timer — deadline-based timer.
    Timer = 10,
    /// Fifo — fixed-size element queue.
    Fifo = 11,
    /// Resource — hierarchical capability tree node.
    Resource = 12,
    /// Job — process group container.
    Job = 13,
    /// Interrupt — hardware IRQ object.
    Interrupt = 14,
    /// Iommu — IOMMU context (VT-d domain).
    Iommu = 15,
    /// Bti — bus transaction initiator (DMA grant).
    Bti = 16,
    /// Pmt — pinned memory token (pinned VMO region for DMA).
    Pmt = 17,
}

bitflags! {
    /// Signal bits observable on any kernel object.
    ///
    /// Signals are a bitmask of conditions. Userspace can wait for specific
    /// signal combinations via ports or blocking waits. Objects raise signals
    /// when state changes (e.g., a channel becomes readable).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct Signals: u32 {
        /// Object-specific signal 0 (e.g., READABLE for channels).
        const SIGNAL_0 = 1 << 0;
        /// Object-specific signal 1 (e.g., WRITABLE for channels).
        const SIGNAL_1 = 1 << 1;
        /// Object-specific signal 2.
        const SIGNAL_2 = 1 << 2;
        /// Object-specific signal 3.
        const SIGNAL_3 = 1 << 3;
        /// Object-specific signal 4.
        const SIGNAL_4 = 1 << 4;

        /// The object's peer has been closed (channels, sockets, event pairs).
        const PEER_CLOSED = 1 << 24;
        /// The object handle has been closed.
        const HANDLE_CLOSED = 1 << 25;

        // Convenience aliases for common IPC patterns.

        /// Channel/socket: data available to read.
        const READABLE = Self::SIGNAL_0.bits();
        /// Channel/socket: peer can accept writes.
        const WRITABLE = Self::SIGNAL_1.bits();

        /// Process/thread: has terminated.
        const TERMINATED = Self::SIGNAL_0.bits();
    }
}

/// The core trait implemented by every kernel object.
///
/// All kernel resources — processes, threads, channels, VMOs, etc. — implement
/// this trait. Objects are reference-counted (`Arc<dyn KernelObject>`) and
/// accessed exclusively through [`HandleEntry`](super::handle::HandleEntry)
/// entries in a process's [`HandleTable`](super::handle::HandleTable).
pub trait KernelObject: Send + Sync + 'static {
    /// The type discriminant for this object.
    fn object_type(&self) -> ObjectType;

    /// The globally unique identifier for this object.
    fn koid(&self) -> Koid;

    /// Downcast support — returns `self` as `&dyn Any` for type-safe
    /// downcasting in syscall handlers.
    fn as_any(&self) -> &dyn core::any::Any;

    /// The koid of a related/peer object (e.g., the other end of a channel).
    ///
    /// Returns [`Koid::INVALID`] if there is no related object.
    fn related_koid(&self) -> Koid {
        Koid::INVALID
    }

    /// Current signal state of this object.
    fn get_signals(&self) -> Signals;

    /// Register a port observer for signal changes.
    ///
    /// When any of the specified `signals` become asserted, a packet is queued
    /// to the given `port` with the provided `key`.
    fn add_observer(&self, port: Arc<dyn PortDispatch>, key: u64, signals: Signals);

    /// Remove a previously registered port observer.
    fn remove_observer(&self, port: &Arc<dyn PortDispatch>);

    /// Called when the last handle to this object is closed.
    ///
    /// Used by paired objects (Channel, Socket, EventPair, Fifo) to assert
    /// `PEER_CLOSED` on the surviving peer.
    fn on_zero_handles(&self) {
        // Default: no-op. Override for paired objects.
    }
}
