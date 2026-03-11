//! Handle syscall handlers: close, dup, pipe.

use hadron_syscall::*;

use super::validate::UserPtrMut;
use super::with_handle_table;

/// `SYS_HANDLE_CLOSE(fd)` — close a handle.
pub fn sys_handle_close(fd: usize) -> isize {
    let hv = hadron_objects::handle::HandleValue::from_raw(fd as u32);

    let result = with_handle_table(|table| table.remove(hv));

    match result {
        Ok(entry) => {
            // Notify the object that a handle was closed.
            entry.object().on_zero_handles();
            0
        }
        Err(_) => -EBADF,
    }
}

/// `SYS_HANDLE_DUP(old_fd, new_fd)` — duplicate a handle to a specific slot.
///
/// If `new_fd` is already open, it is silently closed first (dup2 semantics).
pub fn sys_handle_dup(old_fd: usize, new_fd: usize) -> isize {
    let old_hv = hadron_objects::handle::HandleValue::from_raw(old_fd as u32);
    let new_hv = hadron_objects::handle::HandleValue::from_raw(new_fd as u32);

    with_handle_table(|table| {
        // Look up the source entry.
        let entry = match table.get(old_hv) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let new_entry = hadron_objects::handle::HandleEntry::new(
            alloc::sync::Arc::clone(entry.object()),
            entry.rights(),
        );

        // Close new_fd if it exists (dup2 semantics).
        let _ = table.remove(new_hv);

        match table.insert(new_entry) {
            Ok(hv) => hv.raw() as isize,
            Err(_) => -EMFILE,
        }
    })
}

/// `SYS_HANDLE_DUP_LOWEST(fd)` — duplicate to the lowest available slot.
pub fn sys_handle_dup_lowest(fd: usize) -> isize {
    let hv = hadron_objects::handle::HandleValue::from_raw(fd as u32);

    with_handle_table(
        |table| match table.duplicate(hv, hadron_objects::handle::Rights::ALL) {
            Ok(new_hv) => new_hv.raw() as isize,
            Err(hadron_objects::handle::HandleError::NotFound) => -EBADF,
            Err(hadron_objects::handle::HandleError::AccessDenied) => -EACCES,
            Err(hadron_objects::handle::HandleError::TableFull) => -EMFILE,
        },
    )
}

/// `SYS_HANDLE_PIPE(fds_ptr)` — create a socket pair.
///
/// Phase 2a stub: returns ENOSYS (sockets not yet implemented).
pub fn sys_handle_pipe(fds_ptr: usize) -> isize {
    let _fds_out = match UserPtrMut::<[usize; 2]>::new(fds_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };
    -ENOSYS // Phase 2b: implement with Socket::create_pair()
}

/// `SYS_HANDLE_TCSETPGRP` — stub (not yet implemented).
pub fn sys_handle_tcsetpgrp(_fd: usize, _pgid: usize) -> isize {
    -ENOSYS
}

/// `SYS_HANDLE_TCGETPGRP` — stub (not yet implemented).
pub fn sys_handle_tcgetpgrp(_fd: usize) -> isize {
    -ENOSYS
}
