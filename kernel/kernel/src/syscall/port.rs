//! Port syscall handlers: create, wait, queue.

extern crate alloc;

use alloc::sync::Arc;

use hadron_objects::handle::{HandleEntry, HandleValue, Rights};
use hadron_objects::object::{KernelObject, Signals};
use hadron_objects::port::Port;
use hadron_syscall::types::UserPortPacket;
use hadron_syscall::*;

use super::validate::UserPtrMut;
use super::with_handle_table;

/// `SYS_PORT_CREATE()` — create a new port (async event aggregator).
///
/// Returns a handle to the new Port, or a negative error code.
pub fn sys_port_create() -> isize {
    let port = Port::new();
    let entry = HandleEntry::new(port as Arc<dyn KernelObject>, Rights::PORT_DEFAULT);

    with_handle_table(|table| match table.insert(entry) {
        Ok(hv) => hv.raw() as isize,
        Err(_) => -EMFILE,
    })
}

/// `SYS_PORT_WAIT(fd, packet_ptr)` — dequeue a packet from a port.
///
/// If the port has a pending packet, writes it to `packet_ptr` and returns 0.
/// If the port is empty, blocks until a packet is available.
#[expect(
    clippy::cast_possible_truncation,
    reason = "signal bitmasks and koid fit in their respective types"
)]
pub fn sys_port_wait(fd: usize, packet_ptr: usize) -> isize {
    let packet_out = match UserPtrMut::<UserPortPacket>::new(packet_ptr) {
        Ok(p) => p,
        Err(e) => return e,
    };

    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::READ) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let port = match entry.object().as_any().downcast_ref::<Port>() {
            Some(p) => p,
            None => return -EINVAL,
        };

        match port.try_wait() {
            Ok(pkt) => {
                let user_pkt = UserPortPacket {
                    key: pkt.key,
                    signals: pkt.signals.bits(),
                    koid: pkt.koid.raw(),
                    packet_type: match pkt.packet_type {
                        hadron_objects::port_packet::PacketType::SignalOne => 0,
                        hadron_objects::port_packet::PacketType::User => 1,
                    },
                };
                // SAFETY: packet_ptr was validated by UserPtrMut::new.
                unsafe { packet_out.write(user_pkt) };
                0
            }
            Err(_) => {
                // Port is empty — block.
                crate::process::set_blocking_op(crate::process::BlockingOp::PortWait {
                    fd,
                    packet_out_ptr: packet_ptr,
                });
                let saved_rsp: u64;
                // SAFETY: GS is kernel; gs:[8] was set by enter_userspace_save.
                unsafe { core::arch::asm!("mov {}, gs:[8]", out(reg) saved_rsp) };
                // SAFETY: saved_rsp is valid.
                unsafe {
                    crate::arch::x86_64::userspace::restore_kernel_context(saved_rsp);
                }
            }
        }
    })
}

/// `SYS_PORT_QUEUE(fd, key, signals)` — queue a user packet on a port.
#[expect(
    clippy::cast_possible_truncation,
    reason = "signal bitmask fits in u32"
)]
pub fn sys_port_queue(fd: usize, key: usize, signals: usize) -> isize {
    let hv = HandleValue::from_raw(fd as u32);

    with_handle_table(|table| {
        let entry = match table.get_with_rights(hv, Rights::WRITE) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };

        let port = match entry.object().as_any().downcast_ref::<Port>() {
            Some(p) => p,
            None => return -EINVAL,
        };

        port.queue_user_packet(key as u64, Signals::from_bits_truncate(signals as u32));
        0
    })
}

/// `SYS_OBJECT_WAIT_ASYNC(object_fd, port_fd, key, signals)` — register an
/// async signal observer on `object_fd` that delivers a packet to `port_fd`
/// when any of the specified `signals` become asserted.
///
/// Uses one-shot semantics: the observer fires at most once, then is removed.
/// If the signals are already asserted at registration time, the packet is
/// delivered immediately.
#[expect(
    clippy::cast_possible_truncation,
    reason = "signal bitmask fits in u32"
)]
pub fn sys_object_wait_async(
    object_fd: usize,
    port_fd: usize,
    key: usize,
    signals: usize,
) -> isize {
    use hadron_objects::observer::PortDispatch;

    let obj_hv = HandleValue::from_raw(object_fd as u32);
    let port_hv = HandleValue::from_raw(port_fd as u32);
    let mask = Signals::from_bits_truncate(signals as u32);

    with_handle_table(|table| {
        // Look up the target object with WAIT rights.
        let obj_entry = match table.get_with_rights(obj_hv, Rights::WAIT) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };
        let object = obj_entry.object().clone();

        // Look up the port with WRITE rights.
        let port_entry = match table.get_with_rights(port_hv, Rights::WRITE) {
            Ok(e) => e,
            Err(_) => return -EBADF,
        };
        let port = match port_entry.object().as_any().downcast_ref::<Port>() {
            Some(_) => port_entry.object().clone(),
            None => return -EINVAL,
        };

        // SAFETY: We verified the downcast succeeds above.
        let port_dispatch: Arc<dyn PortDispatch> =
            unsafe { Arc::from_raw(Arc::into_raw(port).cast::<Port>()) };

        // Register the observer with immediate-check semantics.
        object.add_observer_checked(port_dispatch, key as u64, mask);
        0
    })
}
