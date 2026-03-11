//! EventPair syscall handlers: create, signal_peer.

use hadron_objects::event_pair::EventPair;
use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::Signals;
use hadron_syscall::*;

use super::validate::UserPtrMut;
use super::with_handle_table;

/// `SYS_EVENT_PAIR_CREATE(fds_ptr)` — create a linked event pair.
///
/// Writes `[fd_a, fd_b]` to the user buffer at `fds_ptr`.
pub fn sys_event_pair_create(fds_ptr: usize) -> isize {
    let fds_out = match UserPtrMut::<[usize; 2]>::new(fds_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let (ep0, ep1) = EventPair::create_pair();

    let result = with_handle_table(|table| {
        let hv0 = table.insert(HandleEntry::new(ep0, Rights::EVENT_PAIR_DEFAULT))?;
        match table.insert(HandleEntry::new(ep1, Rights::EVENT_PAIR_DEFAULT)) {
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

/// `SYS_EVENT_PAIR_SIGNAL_PEER(fd, set, clear)` — signal the peer endpoint.
#[expect(
    clippy::cast_possible_truncation,
    reason = "signal bitmasks fit in u32"
)]
pub fn sys_event_pair_signal_peer(fd: usize, set_mask: usize, clear_mask: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::SIGNAL) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let ep = match entry.object().as_any().downcast_ref::<EventPair>() {
            Some(e) => e,
            None => return -EINVAL,
        };

        let set = Signals::from_bits_truncate(set_mask as u32);
        let clear = Signals::from_bits_truncate(clear_mask as u32);

        if ep.signal_peer(set, clear) {
            0
        } else {
            -EPIPE // Peer has been closed
        }
    })
}
