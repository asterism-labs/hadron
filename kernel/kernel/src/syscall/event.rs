//! Event, timer, and polling syscall handlers.
//!
//! Implements `event_create`, `event_signal`, `event_wait_many`,
//! `clock_gettime`, `clock_nanosleep`, and `futex`.

extern crate alloc;

use alloc::sync::Arc;

use hadron_objects::handle::Rights;
use hadron_objects::object::{KernelObject, Signals};
use hadron_syscall::constants::*;
use hadron_syscall::types::{PollFd, Timespec};
use hadron_syscall::*;

use super::validate::{UserPtrMut, UserSlice};

/// `SYS_EVENT_WAIT_MANY(fds_ptr, nfds, timeout_ms)` — poll multiple handles.
///
/// Checks each handle's current signals against the requested events.
/// If any handle is ready, fills `revents` and returns the count of ready
/// handles. If none are ready and `timeout_ms == 0`, returns 0 (non-blocking).
///
/// For Phase 2c: only non-blocking poll is implemented (timeout_ms is
/// accepted but blocking is not yet supported). This suffices for the
/// verification test which polls after data has been sent.
#[expect(
    clippy::cast_possible_truncation,
    reason = "poll fd/count values fit in u16/u32"
)]
pub fn sys_event_wait_many(fds_ptr: usize, nfds: usize, _timeout_ms: usize) -> isize {
    if nfds == 0 {
        return 0;
    }
    if nfds > 256 {
        return -EINVAL;
    }

    // Read PollFd array from user memory.
    let poll_size = nfds * core::mem::size_of::<PollFd>();
    let slice = match UserSlice::new(fds_ptr, poll_size) {
        Ok(s) => s,
        Err(e) => return e,
    };
    // SAFETY: User buffer was range-validated.
    let raw_bytes = unsafe { slice.read_to_vec() };

    let mut poll_fds: alloc::vec::Vec<PollFd> = raw_bytes
        .chunks_exact(core::mem::size_of::<PollFd>())
        .map(|chunk| {
            let fd = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let events = u16::from_ne_bytes([chunk[4], chunk[5]]);
            PollFd {
                fd,
                events,
                revents: 0,
            }
        })
        .collect();

    let mut ready_count: isize = 0;

    for pfd in &mut poll_fds {
        let hv = hadron_objects::handle::HandleValue::from_raw(pfd.fd);

        let signals =
            super::with_handle_table(|table| match table.get_with_rights(hv, Rights::WAIT) {
                Ok(entry) => {
                    let obj = entry.object();
                    Ok(obj.get_signals())
                }
                Err(hadron_objects::handle::HandleError::NotFound) => Err(POLLNVAL),
                Err(_) => Err(POLLERR),
            });

        match signals {
            Ok(sig) => {
                let mut revents: u16 = 0;
                if pfd.events & POLLIN != 0 && sig.contains(Signals::READABLE) {
                    revents |= POLLIN;
                }
                if pfd.events & POLLOUT != 0 && sig.contains(Signals::WRITABLE) {
                    revents |= POLLOUT;
                }
                if sig.contains(Signals::PEER_CLOSED) {
                    revents |= POLLHUP;
                }
                pfd.revents = revents;
                if revents != 0 {
                    ready_count += 1;
                }
            }
            Err(err_flag) => {
                pfd.revents = err_flag;
                ready_count += 1;
            }
        }
    }

    // Write back the updated PollFd array with revents filled.
    let out_bytes: alloc::vec::Vec<u8> = poll_fds
        .iter()
        .flat_map(|pfd| {
            let mut buf = [0u8; core::mem::size_of::<PollFd>()];
            buf[0..4].copy_from_slice(&pfd.fd.to_ne_bytes());
            buf[4..6].copy_from_slice(&pfd.events.to_ne_bytes());
            buf[6..8].copy_from_slice(&pfd.revents.to_ne_bytes());
            buf
        })
        .collect();

    // SAFETY: User buffer was range-validated; same size as input.
    unsafe { slice.write_from_slice(&out_bytes) };

    ready_count
}

/// `SYS_EVENT_CREATE()` — create a new Event object.
///
/// Returns a handle to the new Event, or a negative error code.
pub fn sys_event_create() -> isize {
    use hadron_objects::event::Event;
    use hadron_objects::handle::HandleEntry;

    let event = Event::new();
    let entry = HandleEntry::new(event as Arc<dyn KernelObject>, Rights::ALL);

    super::with_handle_table(|table| match table.insert(entry) {
        Ok(hv) => hv.raw() as isize,
        Err(_) => -EMFILE,
    })
}

/// `SYS_EVENT_SIGNAL(fd, set_mask, clear_mask)` — signal an Event object.
pub fn sys_event_signal(fd: usize, set_mask: usize, clear_mask: usize) -> isize {
    use hadron_objects::event::Event;

    let hv = hadron_objects::handle::HandleValue::from_raw(fd as u32);

    super::with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::SIGNAL) {
            Ok(e) => e,
            Err(hadron_objects::handle::HandleError::NotFound) => return -EBADF,
            Err(_) => return -EACCES,
        };

        let event = match entry.object().as_any().downcast_ref::<Event>() {
            Some(e) => e,
            None => return -EINVAL,
        };

        let set = Signals::from_bits_truncate(set_mask as u32);
        let clear = Signals::from_bits_truncate(clear_mask as u32);
        event.signal(set, clear);
        0
    })
}

/// `SYS_EVENT_WAIT(fd, signals)` — stub (not yet implemented).
pub fn sys_event_wait(_fd: usize, _signals: usize) -> isize {
    -ENOSYS
}

/// `SYS_CLOCK_GETTIME(clockid, tp_ptr)` — get current time.
///
/// Only `CLOCK_MONOTONIC` is supported. Returns nanoseconds since boot.
pub fn sys_clock_gettime(clockid: usize, tp_ptr: usize) -> isize {
    if clockid != CLOCK_MONOTONIC as usize {
        return -EINVAL;
    }

    let tp_out = match UserPtrMut::<Timespec>::new(tp_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let nanos = crate::time::nanos_since_boot();
    let ts = Timespec {
        tv_sec: nanos / 1_000_000_000,
        tv_nsec: nanos % 1_000_000_000,
    };

    // SAFETY: Pointer was range-validated.
    unsafe { tp_out.write(ts) };
    0
}

/// `SYS_CLOCK_NANOSLEEP` — stub (not yet implemented).
pub fn sys_clock_nanosleep(
    _clockid: usize,
    _flags: usize,
    _req_ptr: usize,
    _rem_ptr: usize,
) -> isize {
    -ENOSYS
}

/// `SYS_FUTEX` — stub (not yet implemented).
pub fn sys_futex(_addr: usize, _op: usize, _val: usize, _timeout: usize) -> isize {
    -ENOSYS
}
