//! IOMMU-related syscall handlers.
//!
//! Provides `sys_bti_create`, `sys_bti_pin`, `sys_bti_release_quarantine`,
//! and `sys_pmt_unpin` for userspace DMA management.
//!
//! Lock ordering: all handlers clone the `Arc<dyn KernelObject>` from the
//! handle table and release the table lock before calling IOMMU operations,
//! which acquire `VTD_UNITS` (level 14).

use alloc::sync::Arc;

use hadron_iommu::hw::{DmaPermission, IommuError, PciBdf};
use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::KernelObject;
use hadron_syscall::*;

use super::validate::UserPtrMut;
use super::with_handle_table;
use crate::iommu_objects::bti::Bti;
use crate::iommu_objects::iommu::Iommu;
use crate::iommu_objects::pmt::Pmt;

/// Create a BTI (Bus Transaction Initiator) for a PCI device.
///
/// # Arguments
/// - `iommu_fd`: Handle to an `Iommu` object (requires `WRITE` right)
/// - `bdf_packed`: Packed BDF: `(bus << 16) | (device << 8) | function`
///
/// # Returns
/// Handle value of the new `Bti`, or negative errno.
pub fn sys_bti_create(iommu_fd: usize, bdf_packed: usize) -> isize {
    // Get Iommu object from handle table (release lock after Arc clone).
    let obj = match clone_object_with_rights(iommu_fd, Rights::WRITE) {
        Ok(obj) => obj,
        Err(e) => return e,
    };
    let iommu = match obj.as_any().downcast_ref::<Iommu>() {
        Some(i) => i,
        None => return -EBADF,
    };

    // Decode packed BDF.
    let bdf = PciBdf {
        bus: ((bdf_packed >> 16) & 0xFF) as u8,
        device: ((bdf_packed >> 8) & 0x1F) as u8,
        function: (bdf_packed & 0x07) as u8,
    };

    // Create the Bti.
    let bti = match Bti::new(iommu.unit_index(), bdf) {
        Ok(bti) => bti,
        Err(e) => return iommu_err_to_errno(e),
    };

    // Insert into handle table.
    with_handle_table(
        |table| match table.insert(HandleEntry::new(bti, Rights::BTI_DEFAULT)) {
            Ok(hv) => hv.raw() as isize,
            Err(_) => -EMFILE,
        },
    )
}

/// Pin physical pages for DMA through a BTI.
///
/// # Arguments
/// - `bti_fd`: Handle to a `Bti` object (requires `WRITE` right)
/// - `page_count`: Number of 4 KiB pages to pin
/// - `perm`: DMA permission (0 = READ, 1 = WRITE, 2 = READ_WRITE)
/// - `phys_out_ptr`: User pointer to write physical addresses (array of u64)
/// - `phys_out_len`: Length of the output array in elements
///
/// # Returns
/// Handle value of the new `Pmt`, or negative errno.
pub fn sys_bti_pin(
    bti_fd: usize,
    page_count: usize,
    perm: usize,
    phys_out_ptr: usize,
    phys_out_len: usize,
) -> isize {
    if page_count == 0 || phys_out_len < page_count {
        return -EINVAL;
    }

    // Get Bti object (clone Arc, release handle table lock).
    let obj = match clone_object_with_rights(bti_fd, Rights::WRITE) {
        Ok(obj) => obj,
        Err(e) => return e,
    };
    let bti = match obj.as_any().downcast_ref::<Bti>() {
        Some(b) => b,
        None => return -EBADF,
    };

    // Decode permission.
    let dma_perm = match perm {
        0 => DmaPermission::READ,
        1 => DmaPermission {
            read: false,
            write: true,
        },
        2 => DmaPermission::READ_WRITE,
        _ => return -EINVAL,
    };

    // Pin pages (calls IOMMU hardware — no handle table lock held).
    // SAFETY: bti is valid because we hold an Arc to it.
    // We need the Arc<Bti> for the pin method's self: &Arc<Self>.
    // Reconstruct it from the dyn object.
    let bti_ptr = bti as *const Bti;
    let bti_arc = {
        // Increment refcount by cloning the dyn Arc, then build Arc<Bti>.
        let _extra_ref = Arc::clone(&obj);
        // SAFETY: The object was verified to be Bti. Arc<dyn KernelObject>
        // stores the Bti at the same address returned by downcast_ref.
        // We incremented the refcount above to compensate for Arc::from_raw.
        unsafe { Arc::from_raw(bti_ptr) }
    };

    let pmt = match bti_arc.pin(page_count, dma_perm) {
        Ok(pmt) => pmt,
        Err(e) => return iommu_err_to_errno(e),
    };

    // Write physical addresses to user buffer.
    let addrs = pmt.phys_addrs();
    for (i, addr) in addrs.iter().enumerate() {
        let user_ptr = match UserPtrMut::<u64>::new(phys_out_ptr + i * 8) {
            Ok(p) => p,
            Err(e) => return e,
        };
        // SAFETY: UserPtrMut::new validated the address is in user space.
        unsafe { core::ptr::write(user_ptr.as_mut_ptr(), *addr) };
    }

    // Insert Pmt into handle table.
    with_handle_table(
        |table| match table.insert(HandleEntry::new(pmt, Rights::PMT_DEFAULT)) {
            Ok(hv) => hv.raw() as isize,
            Err(_) => -EMFILE,
        },
    )
}

/// Release BTI quarantine after error recovery.
///
/// # Arguments
/// - `bti_fd`: Handle to a `Bti` object (requires `WRITE` right)
///
/// # Returns
/// 0 on success, or negative errno.
pub fn sys_bti_release_quarantine(bti_fd: usize) -> isize {
    let obj = match clone_object_with_rights(bti_fd, Rights::WRITE) {
        Ok(obj) => obj,
        Err(e) => return e,
    };
    match obj.as_any().downcast_ref::<Bti>() {
        Some(bti) => {
            bti.release_quarantine();
            0
        }
        None => -EBADF,
    }
}

/// Unpin a PMT, freeing the DMA mapping and returning frames to PMM.
///
/// This consumes the handle — the PMT is removed from the handle table.
///
/// # Arguments
/// - `pmt_fd`: Handle to a `Pmt` object
///
/// # Returns
/// 0 on success, or negative errno.
pub fn sys_pmt_unpin(pmt_fd: usize) -> isize {
    let hv = HandleValue::from_raw(pmt_fd as u32);

    // Remove the handle from the table (consumes it).
    let entry = match with_handle_table(|table| table.remove(hv)) {
        Ok(entry) => entry,
        Err(_) => return -EBADF,
    };

    // Verify it's a Pmt and unpin.
    match entry.object().as_any().downcast_ref::<Pmt>() {
        Some(pmt) => {
            let _ = pmt.unpin();
            // HandleEntry dropped here — triggers on_zero_handles if last ref.
            0
        }
        None => {
            // Not a Pmt — re-insert the handle (undo removal).
            with_handle_table(|table| {
                let _ = table.insert(entry);
            });
            -EBADF
        }
    }
}

/// Clone an `Arc<dyn KernelObject>` from the handle table with rights check.
///
/// Returns the cloned Arc after releasing the handle table lock, which is
/// critical for avoiding lock ordering violations with `VTD_UNITS`.
fn clone_object_with_rights(fd: usize, required: Rights) -> Result<Arc<dyn KernelObject>, isize> {
    let hv = HandleValue::from_raw(fd as u32);
    with_handle_table(|table| {
        let entry = table.get_with_rights(hv, required).map_err(|_| -EBADF)?;
        Ok(Arc::clone(entry.object()))
    })
}

/// Convert an `IommuError` to a negative errno value.
fn iommu_err_to_errno(e: IommuError) -> isize {
    match e {
        IommuError::DomainExhausted | IommuError::OutOfMemory => -ENOMEM,
        IommuError::InvalidDomain | IommuError::InvalidIova => -EINVAL,
        IommuError::DeviceNotAttached | IommuError::NotInitialized => -ENODEV,
        IommuError::HardwareFault => -EIO,
    }
}
