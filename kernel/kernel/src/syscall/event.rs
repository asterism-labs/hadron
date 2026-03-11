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

use super::validate::{UserPtr, UserPtrMut, UserSlice};

/// `SYS_EVENT_WAIT_MANY(fds_ptr, nfds, timeout_ms)` — poll multiple handles.
///
/// Checks each handle's current signals against the requested events.
/// If any handle is ready, fills `revents` and returns the count of ready
/// handles. If none are ready and `timeout_ms == 0`, returns 0 (non-blocking).
/// If none are ready and `timeout_ms != 0`, blocks until an fd becomes ready
/// or the timeout expires.
#[expect(
    clippy::cast_possible_truncation,
    reason = "poll fd/count values fit in u16/u32"
)]
pub fn sys_event_wait_many(fds_ptr: usize, nfds: usize, timeout_ms: usize) -> isize {
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

    // If nothing is ready and caller wants to block, set up blocking op.
    if ready_count == 0 && timeout_ms != 0 {
        crate::process::set_blocking_op(crate::process::BlockingOp::EventWaitMany {
            fds_ptr,
            nfds,
            timeout_ms: timeout_ms as u64,
        });

        // Longjmp back to the process task.
        let saved_rsp: u64;
        // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
        unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
        // SAFETY: saved_rsp is valid.
        unsafe {
            crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
        }
    }

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

/// `SYS_EVENT_WAIT(fd, signals)` — wait for signals on a single object.
///
/// Returns immediately if any of the requested signals are already set.
/// Otherwise, blocks until at least one signal in the mask becomes set.
#[expect(
    clippy::cast_possible_truncation,
    reason = "signal bitmask fits in u32"
)]
pub fn sys_event_wait(fd: usize, signals: usize) -> isize {
    let hv = hadron_objects::handle::HandleValue::from_raw(fd as u32);
    let mask = Signals::from_bits_truncate(signals as u32);

    super::with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::WAIT) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let current = entry.object().get_signals();
        if current.intersects(mask) {
            return current.bits() as isize;
        }

        // Not ready — block.
        crate::process::set_blocking_op(crate::process::BlockingOp::EventWait {
            fd,
            signals: signals as u32,
        });
        let saved_rsp: u64;
        // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
        unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
        // SAFETY: saved_rsp is valid.
        unsafe {
            crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
        }
    })
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

/// `SYS_CLOCK_NANOSLEEP` — sleep until an absolute monotonic deadline.
///
/// `clockid` must be `CLOCK_MONOTONIC`. `flags` is reserved (must be 0).
/// `req_ptr` points to a `Timespec` with the absolute deadline.
/// Returns 0 on success, negative errno on error.
pub fn sys_clock_nanosleep(
    clockid: usize,
    _flags: usize,
    req_ptr: usize,
    _rem_ptr: usize,
) -> isize {
    if clockid != CLOCK_MONOTONIC as usize {
        return -EINVAL;
    }

    // Read the requested sleep time from user memory.
    let ptr = match UserPtr::<Timespec>::new(req_ptr) {
        Ok(ptr) => ptr,
        Err(e) => return e,
    };
    // SAFETY: the pointer was validated to be in the user address range.
    let ts = unsafe { ptr.read() };

    // Validate nanoseconds field.
    if ts.tv_nsec >= 1_000_000_000 {
        return -EINVAL;
    }

    // Compute absolute deadline in nanoseconds since boot.
    let deadline_ns = ts
        .tv_sec
        .saturating_mul(1_000_000_000)
        .saturating_add(ts.tv_nsec);

    // If the deadline has already passed, return immediately.
    if crate::time::nanos_since_boot() >= deadline_ns {
        return 0;
    }

    crate::process::set_blocking_op(crate::process::BlockingOp::ClockNanosleep { deadline_ns });

    // Longjmp back to the process task.
    let saved_rsp: u64;
    // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
    unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
    // SAFETY: saved_rsp is valid.
    unsafe {
        crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
    }
}

/// `SYS_FUTEX(addr, op, val, timeout)` — userspace synchronization primitive.
///
/// `FUTEX_WAIT`: if `*addr == val`, block until woken or timeout expires.
/// `FUTEX_WAKE`: wake up to `val` waiters on `addr`, returns count woken.
#[expect(
    clippy::cast_possible_truncation,
    reason = "futex val and wake count fit in u32"
)]
pub fn sys_futex(addr: usize, op: usize, val: usize, timeout: usize) -> isize {
    // Validate address: must be in user space and 4-byte aligned.
    if addr < 0x1000 || addr >= 0x0000_8000_0000_0000 || addr & 3 != 0 {
        return -EINVAL;
    }

    let koid = crate::process::with_current_process(|p| p.koid().raw()).unwrap_or(0);

    match op as u32 {
        FUTEX_WAIT => {
            // Read the current value at the futex address.
            // SAFETY: addr was validated to be in user range and aligned.
            let current: u32 = unsafe { *(addr as *const u32) };

            if current != val as u32 {
                return -EAGAIN;
            }

            // Compute timeout deadline.
            let timeout_ns = if timeout == 0 || timeout == usize::MAX {
                u64::MAX
            } else {
                crate::time::nanos_since_boot().saturating_add(timeout as u64 * 1_000_000)
            };

            crate::process::set_blocking_op(crate::process::BlockingOp::FutexWait {
                addr: addr as u64,
                val: val as u32,
                timeout_ns,
            });

            // Longjmp back to the process task.
            let saved_rsp: u64;
            // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
            unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
            // SAFETY: saved_rsp is valid.
            unsafe {
                crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
            }
        }
        FUTEX_WAKE => {
            let woken = crate::futex::futex_wake(koid, addr as u64, val as u32);
            woken as isize
        }
        _ => -EINVAL,
    }
}
