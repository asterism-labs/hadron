//! FIFO syscall handlers: create, write, read.

extern crate alloc;

use hadron_objects::fifo::{Fifo, FifoError};
use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_syscall::*;

use super::validate::{UserPtrMut, UserSlice};
use super::with_handle_table;

/// `SYS_FIFO_CREATE(fds_ptr, elem_count, elem_size)` — create a FIFO pair.
///
/// Writes `[fd_a, fd_b]` to the user buffer at `fds_ptr`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "elem_count and elem_size fit in u32"
)]
pub fn sys_fifo_create(fds_ptr: usize, elem_count: usize, elem_size: usize) -> isize {
    let fds_out = match UserPtrMut::<[usize; 2]>::new(fds_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    if elem_count == 0 || elem_size == 0 {
        return -EINVAL;
    }

    let (f0, f1) = Fifo::create_pair(elem_count as u32, elem_size as u32);

    let result = with_handle_table(|table| {
        let hv0 = table.insert(HandleEntry::new(f0, Rights::FIFO_DEFAULT))?;
        match table.insert(HandleEntry::new(f1, Rights::FIFO_DEFAULT)) {
            Ok(hv1) => Ok((hv0, hv1)),
            Err(e) => {
                let _ = table.remove(hv0);
                Err(e)
            }
        }
    });

    match result {
        Ok((hv0, hv1)) => {
            // SAFETY: fds_ptr was validated by UserPtrMut::new.
            unsafe { fds_out.write([hv0.raw() as usize, hv1.raw() as usize]) };
            0
        }
        Err(_) => -EMFILE,
    }
}

/// `SYS_FIFO_WRITE(fd, buf_ptr, len)` — write elements to a FIFO.
///
/// Returns the number of bytes written, or a negative error code.
pub fn sys_fifo_write(fd: usize, buf_ptr: usize, len: usize) -> isize {
    let slice = match UserSlice::new(buf_ptr, len) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // SAFETY: User buffer was range-validated; pages are mapped (shared CR3).
    let data = unsafe { slice.read_to_vec() };

    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::WRITE) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let fifo = match entry.object().as_any().downcast_ref::<Fifo>() {
            Some(f) => f,
            None => return -EBADF,
        };

        match fifo.write(&data) {
            Ok(n) => n as isize,
            Err(FifoError::PeerClosed) => -EPIPE,
            Err(FifoError::BufferFull) => -EAGAIN,
            Err(_) => -EIO,
        }
    })
}

/// `SYS_FIFO_READ(fd, buf_ptr, len)` — read elements from a FIFO.
///
/// If the FIFO is empty, blocks until data is available.
/// Returns the number of bytes read, or a negative error code.
pub fn sys_fifo_read(fd: usize, buf_ptr: usize, buf_len: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::READ) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let fifo = match entry.object().as_any().downcast_ref::<Fifo>() {
            Some(f) => f,
            None => return -EBADF,
        };

        // Allocate a temporary buffer to read into.
        let mut buf = alloc::vec![0u8; buf_len];

        match fifo.read(&mut buf) {
            Ok(n) => {
                // SAFETY: buf_ptr was validated (will be checked below).
                let out_slice = match UserSlice::new(buf_ptr, n) {
                    Ok(s) => s,
                    Err(e) => return e,
                };
                // SAFETY: User buffer was range-validated.
                unsafe { out_slice.write_from_slice(&buf[..n]) };
                n as isize
            }
            Err(FifoError::ShouldWait) => {
                // Block: set up blocking op and longjmp to process_task.
                crate::process::set_blocking_op(crate::process::BlockingOp::FifoRead {
                    fd,
                    buf_ptr,
                    buf_len,
                });
                let saved_rsp: u64;
                // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
                unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
                // SAFETY: saved_rsp is valid.
                unsafe {
                    crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
                }
            }
            Err(FifoError::PeerClosed) => -EPIPE,
            Err(_) => -EIO,
        }
    })
}
