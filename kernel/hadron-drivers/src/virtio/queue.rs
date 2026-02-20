//! VirtIO split virtqueue implementation.
//!
//! Provides [`Virtqueue`] which manages the descriptor table, available ring,
//! and used ring for VirtIO split-queue I/O.

use core::sync::atomic::{AtomicU16, Ordering};

use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::services::KernelServices;

use super::pci::VirtioPciTransport;

// -- Descriptor flags ---------------------------------------------------------

/// Descriptor continues via the `next` field.
pub const VIRTQ_DESC_F_NEXT: u16 = 1;
/// Buffer is device-writable (otherwise device-readable).
pub const VIRTQ_DESC_F_WRITE: u16 = 2;

// -- Descriptor table entry (16 bytes) ----------------------------------------

/// A single virtqueue descriptor.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VirtqDesc {
    /// Physical address of the buffer.
    pub addr: u64,
    /// Length of the buffer in bytes.
    pub len: u32,
    /// Descriptor flags (`VIRTQ_DESC_F_NEXT`, `VIRTQ_DESC_F_WRITE`).
    pub flags: u16,
    /// Index of the next descriptor if `VIRTQ_DESC_F_NEXT` is set.
    pub next: u16,
}

/// Size of a single descriptor in bytes.
const DESC_SIZE: usize = 16;

// -- Available ring layout ----------------------------------------------------
// struct virtq_avail {
//     flags: u16,        // offset 0
//     idx: u16,          // offset 2
//     ring: [u16; N],    // offset 4
//     used_event: u16,   // offset 4 + 2*N (optional, not used here)
// }

/// Byte offset of the `idx` field in the available ring.
const AVAIL_IDX_OFFSET: usize = 2;
/// Byte offset of the `ring` array in the available ring.
const AVAIL_RING_OFFSET: usize = 4;

// -- Used ring layout ---------------------------------------------------------
// struct virtq_used {
//     flags: u16,          // offset 0
//     idx: u16,            // offset 2
//     ring: [UsedElem; N], // offset 4
//     avail_event: u16,    // (optional, not used here)
// }
// struct UsedElem { id: u32, len: u32 } — 8 bytes each

/// Byte offset of the `idx` field in the used ring.
const USED_IDX_OFFSET: usize = 2;
/// Byte offset of the `ring` array in the used ring.
const USED_RING_OFFSET: usize = 4;
/// Size of a used ring element in bytes.
const USED_ELEM_SIZE: usize = 8;

/// Page size for DMA allocations.
const PAGE_SIZE: usize = 4096;

/// A VirtIO split virtqueue.
///
/// Manages descriptor table, available ring, and used ring backed by
/// DMA-allocated memory. Provides methods to submit descriptor chains
/// and poll for completions.
pub struct Virtqueue {
    /// Virtual address of the descriptor table.
    desc_virt: *mut VirtqDesc,
    /// Virtual address of the available ring.
    avail_virt: *mut u8,
    /// Virtual address of the used ring.
    used_virt: *mut u8,
    /// Physical address of the descriptor table (for device programming).
    desc_phys: u64,
    /// Physical address of the available ring.
    avail_phys: u64,
    /// Physical address of the used ring.
    used_phys: u64,
    /// Queue size (number of descriptors).
    queue_size: u16,
    /// Head of the free descriptor list (linked via `next` fields).
    free_head: u16,
    /// Number of free descriptors.
    num_free: u16,
    /// Last seen used ring index (for polling completions).
    last_used_idx: u16,
    /// Next available ring index to write.
    avail_idx: AtomicU16,
}

// SAFETY: The virtqueue's DMA buffers are accessed only through this struct,
// and the struct itself is protected by a SpinLock in the driver.
unsafe impl Send for Virtqueue {}
unsafe impl Sync for Virtqueue {}

/// Rounds `size` up to the next multiple of `align`.
const fn align_up(size: usize, align: usize) -> usize {
    (size + align - 1) & !(align - 1)
}

/// Computes the number of pages needed for `size` bytes.
const fn pages_for(size: usize) -> usize {
    align_up(size, PAGE_SIZE) / PAGE_SIZE
}

impl Virtqueue {
    /// Allocates and initializes a new virtqueue with the given size.
    ///
    /// Allocates DMA memory for the descriptor table, available ring, and
    /// used ring. Initializes the free list through descriptor `next` fields.
    pub fn new(
        queue_size: u16,
        services: &'static dyn KernelServices,
    ) -> Result<Self, DriverError> {
        let qs = queue_size as usize;

        // Compute sizes.
        let desc_bytes = qs * DESC_SIZE;
        let avail_bytes = 6 + 2 * qs; // flags(2) + idx(2) + ring(2*N) + used_event(2)
        let used_bytes = 6 + USED_ELEM_SIZE * qs; // flags(2) + idx(2) + ring(8*N) + avail_event(2)

        // Allocate DMA pages for each structure.
        let desc_pages = pages_for(desc_bytes);
        let avail_pages = pages_for(avail_bytes);
        let used_pages = pages_for(used_bytes);

        let desc_phys = services.alloc_dma_frames(desc_pages)?;
        let avail_phys = services.alloc_dma_frames(avail_pages)?;
        let used_phys = services.alloc_dma_frames(used_pages)?;

        let desc_virt = services.phys_to_virt(desc_phys) as *mut VirtqDesc;
        let avail_virt = services.phys_to_virt(avail_phys) as *mut u8;
        let used_virt = services.phys_to_virt(used_phys) as *mut u8;

        // Zero all DMA memory.
        // SAFETY: Freshly allocated DMA pages.
        unsafe {
            core::ptr::write_bytes(desc_virt.cast::<u8>(), 0, desc_pages * PAGE_SIZE);
            core::ptr::write_bytes(avail_virt, 0, avail_pages * PAGE_SIZE);
            core::ptr::write_bytes(used_virt, 0, used_pages * PAGE_SIZE);
        }

        // Initialize free list: each descriptor points to the next.
        for i in 0..queue_size {
            // SAFETY: i is within [0, queue_size) and desc_virt has queue_size entries.
            unsafe {
                let desc = &mut *desc_virt.add(i as usize);
                desc.next = if i + 1 < queue_size { i + 1 } else { 0 };
            }
        }

        Ok(Self {
            desc_virt,
            avail_virt,
            used_virt,
            desc_phys,
            avail_phys,
            used_phys,
            queue_size,
            free_head: 0,
            num_free: queue_size,
            last_used_idx: 0,
            avail_idx: AtomicU16::new(0),
        })
    }

    /// Returns the physical address of the descriptor table.
    #[must_use]
    pub fn desc_phys(&self) -> u64 {
        self.desc_phys
    }

    /// Returns the physical address of the available ring.
    #[must_use]
    pub fn avail_phys(&self) -> u64 {
        self.avail_phys
    }

    /// Returns the physical address of the used ring.
    #[must_use]
    pub fn used_phys(&self) -> u64 {
        self.used_phys
    }

    /// Adds a descriptor chain to the queue.
    ///
    /// `bufs` is a slice of `(phys_addr, length, flags)` tuples describing
    /// each buffer in the chain. Returns the head descriptor index.
    ///
    /// After calling this, use [`notify`](Self::notify) to kick the device.
    pub fn add_buf(&mut self, bufs: &[(u64, u32, u16)]) -> Result<u16, DriverError> {
        if bufs.is_empty() || bufs.len() as u16 > self.num_free {
            return Err(DriverError::IoError);
        }

        let head = self.free_head;
        let mut idx = head;

        for (i, &(addr, len, flags)) in bufs.iter().enumerate() {
            // SAFETY: idx is within [0, queue_size) maintained by the free list.
            let desc = unsafe { &mut *self.desc_virt.add(idx as usize) };
            desc.addr = addr;
            desc.len = len;

            if i + 1 < bufs.len() {
                desc.flags = flags | VIRTQ_DESC_F_NEXT;
                idx = desc.next;
            } else {
                desc.flags = flags & !VIRTQ_DESC_F_NEXT;
                self.free_head = desc.next;
            }
        }

        self.num_free -= bufs.len() as u16;

        // Write the head index to the available ring.
        let avail_idx = self.avail_idx.load(Ordering::Relaxed);
        let ring_slot = (avail_idx % self.queue_size) as usize;

        // SAFETY: Writing to the available ring within bounds.
        unsafe {
            let ring_ptr = self
                .avail_virt
                .add(AVAIL_RING_OFFSET + ring_slot * 2)
                .cast::<u16>();
            core::ptr::write_volatile(ring_ptr, head);
        }

        // Memory barrier: ensure descriptor writes are visible before updating idx.
        core::sync::atomic::fence(Ordering::Release);

        // Increment available ring index.
        let new_idx = avail_idx.wrapping_add(1);
        self.avail_idx.store(new_idx, Ordering::Relaxed);

        // Write the updated index to the available ring header.
        // SAFETY: Writing to the avail idx field.
        unsafe {
            let idx_ptr = self.avail_virt.add(AVAIL_IDX_OFFSET).cast::<u16>();
            core::ptr::write_volatile(idx_ptr, new_idx);
        }

        Ok(head)
    }

    /// Notifies the device that new buffers are available on the given queue.
    pub fn notify(&self, transport: &VirtioPciTransport, queue_index: u16) {
        // Memory barrier: ensure all writes are visible to the device.
        core::sync::atomic::fence(Ordering::SeqCst);
        transport.notify_queue(queue_index);
    }

    /// Polls the used ring for a completion.
    ///
    /// Returns `Some((head_index, bytes_written))` if a completion is
    /// available, or `None` if the used ring has not advanced.
    pub fn poll_used(&mut self) -> Option<(u16, u32)> {
        // Memory barrier: ensure we see device writes.
        core::sync::atomic::fence(Ordering::Acquire);

        // SAFETY: Reading the used ring idx field.
        let used_idx = unsafe {
            let idx_ptr = self.used_virt.add(USED_IDX_OFFSET).cast::<u16>();
            core::ptr::read_volatile(idx_ptr)
        };

        if self.last_used_idx == used_idx {
            return None;
        }

        let ring_slot = (self.last_used_idx % self.queue_size) as usize;

        // SAFETY: Reading a used ring element within bounds.
        let (id, len) = unsafe {
            let elem_ptr = self
                .used_virt
                .add(USED_RING_OFFSET + ring_slot * USED_ELEM_SIZE);
            let id = core::ptr::read_volatile(elem_ptr.cast::<u32>());
            let len = core::ptr::read_volatile(elem_ptr.add(4).cast::<u32>());
            (id, len)
        };

        self.last_used_idx = self.last_used_idx.wrapping_add(1);

        // Free the descriptor chain.
        self.free_chain(id as u16);

        Some((id as u16, len))
    }

    /// Frees a descriptor chain starting at `head` back to the free list.
    fn free_chain(&mut self, head: u16) {
        let mut idx = head;
        let mut count = 0u16;

        loop {
            // SAFETY: idx is a valid descriptor index (was previously allocated).
            let desc = unsafe { &*self.desc_virt.add(idx as usize) };
            let has_next = desc.flags & VIRTQ_DESC_F_NEXT != 0;
            let next = desc.next;
            count += 1;

            if !has_next {
                // Link the end of this chain to the current free head.
                // SAFETY: idx is a valid descriptor index.
                unsafe {
                    (*self.desc_virt.add(idx as usize)).next = self.free_head;
                }
                break;
            }
            idx = next;
        }

        self.free_head = head;
        self.num_free += count;
    }
}
