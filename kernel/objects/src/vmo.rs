//! Virtual Memory Object (VMO).
//!
//! A VMO is a container of physical pages, independent of any address space.
//! VMOs can be mapped into multiple processes simultaneously for shared memory.
//! They support copy-on-write cloning and pager-backed demand paging.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch};

/// The kind of backing store for a VMO.
pub enum VmoKind {
    /// Committed physical pages (standard allocation).
    Paged,
    /// Copy-on-write clone of a parent VMO.
    Cow {
        /// The parent VMO this was cloned from.
        parent: Arc<Vmo>,
        /// Byte offset into the parent.
        offset: u64,
    },
    /// Backed by a userspace pager (for filesystems).
    ///
    /// On page fault, the kernel sends a request to the pager's Port.
    /// The pager supplies pages, which the kernel maps and resumes the
    /// faulting thread.
    Pager {
        /// The pager's port (stored as a generic kernel object; downcast to
        /// Port at use site).
        port: Arc<dyn KernelObject>,
    },
    /// Physically contiguous pages (required for DMA).
    Contiguous,
}

/// Options for creating a VMO.
#[derive(Debug, Clone, Copy)]
pub struct VmoCreateOptions {
    /// Size in bytes (rounded up to page boundary).
    pub size: u64,
    /// Whether pages must be physically contiguous.
    pub contiguous: bool,
}

/// A Virtual Memory Object — the fundamental unit of memory in the microkernel.
///
/// VMOs hold physical pages and can be mapped into one or more address spaces
/// via VMAR mappings. They support:
/// - Direct read/write of contents (for small transfers)
/// - Memory-mapped access (zero-copy shared memory between processes)
/// - Copy-on-write cloning (efficient process forking, snapshot)
/// - Pager-backed demand paging (filesystem mmap)
pub struct Vmo {
    /// Unique identifier for this VMO.
    koid: Koid,
    /// Size of the VMO in bytes (always page-aligned).
    size: AtomicU64,
    /// The backing store kind.
    kind: VmoKind,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers for signal notifications.
    observers: ObserverList,
}

impl Vmo {
    /// Create a new paged VMO with the given size.
    ///
    /// The size is rounded up to the nearest page boundary (4 KiB).
    #[must_use]
    pub fn new_paged(size: u64) -> Arc<Self> {
        let aligned_size = (size + 0xFFF) & !0xFFF;
        Arc::new(Self {
            koid: Koid::alloc(),
            size: AtomicU64::new(aligned_size),
            kind: VmoKind::Paged,
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Create a new physically contiguous VMO.
    #[must_use]
    pub fn new_contiguous(size: u64) -> Arc<Self> {
        let aligned_size = (size + 0xFFF) & !0xFFF;
        Arc::new(Self {
            koid: Koid::alloc(),
            size: AtomicU64::new(aligned_size),
            kind: VmoKind::Contiguous,
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Create a copy-on-write child of this VMO.
    ///
    /// The child shares the parent's pages until either side writes, at which
    /// point the written page is copied (COW fault).
    #[must_use]
    pub fn create_cow_child(parent: &Arc<Self>, offset: u64, size: u64) -> Arc<Self> {
        let aligned_size = (size + 0xFFF) & !0xFFF;
        Arc::new(Self {
            koid: Koid::alloc(),
            size: AtomicU64::new(aligned_size),
            kind: VmoKind::Cow {
                parent: Arc::clone(parent),
                offset,
            },
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// The current size of the VMO in bytes.
    #[must_use]
    pub fn size(&self) -> u64 {
        self.size.load(Ordering::Relaxed)
    }

    /// Resize the VMO. Only valid for paged VMOs.
    ///
    /// # Errors
    ///
    /// Returns `Err` if this VMO kind does not support resizing (COW, contiguous).
    pub fn set_size(&self, new_size: u64) -> Result<(), VmoError> {
        match &self.kind {
            VmoKind::Paged => {
                let aligned = (new_size + 0xFFF) & !0xFFF;
                self.size.store(aligned, Ordering::Relaxed);
                Ok(())
            }
            _ => Err(VmoError::NotResizable),
        }
    }

    /// The backing store kind.
    #[must_use]
    pub fn kind(&self) -> &VmoKind {
        &self.kind
    }
}

impl KernelObject for Vmo {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Vmo
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

/// Errors from VMO operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmoError {
    /// The VMO kind does not support resizing.
    NotResizable,
    /// The requested offset or range is out of bounds.
    OutOfRange,
    /// Insufficient physical memory to commit pages.
    NoMemory,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paged_vmo_size_aligned() {
        let vmo = Vmo::new_paged(100);
        assert_eq!(vmo.size(), 4096);
    }

    #[test]
    fn paged_vmo_exact_page() {
        let vmo = Vmo::new_paged(4096);
        assert_eq!(vmo.size(), 4096);
    }

    #[test]
    fn paged_vmo_resize() {
        let vmo = Vmo::new_paged(4096);
        assert!(vmo.set_size(8192).is_ok());
        assert_eq!(vmo.size(), 8192);
    }

    #[test]
    fn contiguous_vmo_not_resizable() {
        let vmo = Vmo::new_contiguous(4096);
        assert_eq!(vmo.set_size(8192), Err(VmoError::NotResizable));
    }

    #[test]
    fn cow_child_has_unique_koid() {
        let parent = Vmo::new_paged(8192);
        let child = Vmo::create_cow_child(&parent, 0, 4096);
        assert_ne!(parent.koid(), child.koid());
        assert_eq!(child.size(), 4096);
    }

    #[test]
    fn vmo_object_type() {
        let vmo = Vmo::new_paged(4096);
        assert_eq!(vmo.object_type(), ObjectType::Vmo);
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    const PAGE_SIZE: u64 = 4096;

    /// The size of a paged VMO is always page-aligned.
    #[kani::proof]
    fn kani_vmo_size_page_aligned() {
        let size: u64 = kani::any();
        // Bound input to avoid overflow in alignment math.
        kani::assume(size <= 1 << 40);
        let vmo = Vmo::new_paged(size);
        assert_eq!(vmo.size() % PAGE_SIZE, 0);
    }

    /// The size of a paged VMO is at least the requested size (when the input
    /// does not cause overflow).
    #[kani::proof]
    fn kani_vmo_size_at_least_input() {
        let size: u64 = kani::any();
        kani::assume(size <= 1 << 40);
        let vmo = Vmo::new_paged(size);
        assert!(vmo.size() >= size);
    }
}
