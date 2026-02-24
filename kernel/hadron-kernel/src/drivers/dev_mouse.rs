//! `/dev/mouse` device inode.
//!
//! Exposes PS/2 mouse events to userspace via a simple read interface.
//! IRQ 12 fires on mouse movement/button changes; the handler reads
//! 3-byte PS/2 packets from the i8042 data port and queues parsed
//! [`MouseEventPacket`]s into a ring buffer for userspace consumption.

extern crate alloc;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::future::Future;
use core::pin::Pin;
use core::sync::atomic::{AtomicU8, Ordering};
use core::task::Waker;

use crate::fs::{DirEntry, FsError, Inode, InodeType, Permissions};
use crate::sync::IrqSpinLock;
use planck_noalloc::ringbuf::RingBuf;

/// Maximum number of queued mouse event packets.
const EVENT_BUF_SIZE: usize = 64;

/// Size of a serialized [`MouseEventPacket`] in bytes.
const PACKET_SIZE: usize = core::mem::size_of::<hadron_syscall::MouseEventPacket>();

/// PS/2 i8042 data port.
const DATA_PORT: u16 = 0x60;

/// PS/2 i8042 status/command port.
const STATUS_PORT: u16 = 0x64;

/// Ring buffer of parsed mouse event packets, filled by the IRQ handler.
static EVENT_BUF: IrqSpinLock<RingBuf<hadron_syscall::MouseEventPacket, EVENT_BUF_SIZE>> =
    IrqSpinLock::leveled("MOUSE_EVENT", 10, RingBuf::new());

/// Waker for the task blocked in `DevMouse::read()`.
static READ_WAKER: IrqSpinLock<Option<Waker>> = IrqSpinLock::named("MOUSE_WAKER", None);

/// PS/2 packet assembly state: counts bytes 0, 1, 2 of the current packet.
static PACKET_PHASE: AtomicU8 = AtomicU8::new(0);

/// Partial packet bytes being assembled by the IRQ handler.
static PACKET_BYTES: IrqSpinLock<[u8; 3]> = IrqSpinLock::leveled("MOUSE_PKT", 10, [0u8; 3]);

/// Try to read one byte of mouse data from the i8042 controller.
///
/// Returns `Some(byte)` if the output buffer is full and bit 5 (mouse data)
/// is set in the status register, indicating this is mouse data rather than
/// keyboard data.
fn try_read_mouse_byte() -> Option<u8> {
    use crate::arch::x86_64::Port;
    // SAFETY: Reading status and data ports is a standard PS/2 operation.
    let status = unsafe { Port::<u8>::new(STATUS_PORT).read() };
    // Bit 0: output buffer full, bit 5: mouse data.
    if status & 0x01 != 0 && status & 0x20 != 0 {
        Some(unsafe { Port::<u8>::new(DATA_PORT).read() })
    } else {
        None
    }
}

/// Parse a 3-byte PS/2 mouse packet into a [`MouseEventPacket`].
fn parse_packet(bytes: [u8; 3]) -> hadron_syscall::MouseEventPacket {
    let status = bytes[0];
    let x_sign = status & 0x10 != 0;
    let y_sign = status & 0x20 != 0;

    let dx = if x_sign {
        i16::from(bytes[1]) - 256
    } else {
        i16::from(bytes[1])
    };
    let dy = if y_sign {
        i16::from(bytes[2]) - 256
    } else {
        i16::from(bytes[2])
    };

    let buttons = status & 0x07; // bits 0-2: left, right, middle

    hadron_syscall::MouseEventPacket {
        dx,
        dy,
        buttons,
        _pad: [0; 3],
    }
}

/// IRQ 12 handler — assembles 3-byte PS/2 packets and queues parsed events.
fn mouse_irq_handler(_vector: crate::id::IrqVector) {
    while let Some(byte) = try_read_mouse_byte() {
        let phase = PACKET_PHASE.load(Ordering::Relaxed);

        // Byte 0 must have bit 3 set (always-one bit in PS/2 protocol).
        // If it doesn't, resync by discarding and waiting for a valid byte 0.
        if phase == 0 && byte & 0x08 == 0 {
            continue;
        }

        {
            let mut pkt = PACKET_BYTES.lock();
            pkt[phase as usize] = byte;
        }

        if phase == 2 {
            // Complete packet — parse and queue.
            let bytes = *PACKET_BYTES.lock();
            let event = parse_packet(bytes);
            let _ = EVENT_BUF.lock().try_push(event);
            PACKET_PHASE.store(0, Ordering::Relaxed);

            // Wake reader — take waker out of lock before invoking to
            // prevent lock ordering violations with the executor.
            let waker = READ_WAKER.lock().take();
            if let Some(w) = waker {
                w.wake();
            }
        } else {
            PACKET_PHASE.store(phase + 1, Ordering::Relaxed);
        }
    }
}

/// Initialize the PS/2 mouse IRQ handler.
///
/// Registers the IRQ 12 handler and unmasks it in the I/O APIC.
pub fn init() {
    use crate::arch::x86_64::interrupts::dispatch;

    let vector = dispatch::vectors::isa_irq_vector(12);
    dispatch::register_handler(vector, mouse_irq_handler)
        .expect("dev_mouse: failed to register mouse IRQ handler");

    crate::arch::x86_64::acpi::Acpi::with_io_apic(|ioapic| ioapic.unmask(12));

    crate::kinfo!(
        "DevMouse: IRQ12 enabled (vector {}), /dev/mouse ready",
        vector
    );
}

/// `/dev/mouse` device inode.
///
/// Reads return serialized [`MouseEventPacket`] structs (8 bytes each).
/// The read blocks (async) until at least one event is available.
pub struct DevMouse;

impl DevMouse {
    /// Returns the global `DevMouse` inode.
    pub const fn global() -> Self {
        Self
    }
}

impl Inode for DevMouse {
    fn inode_type(&self) -> InodeType {
        InodeType::CharDevice
    }

    fn size(&self) -> usize {
        0
    }

    fn permissions(&self) -> Permissions {
        Permissions::read_only()
    }

    fn read<'a>(
        &'a self,
        _offset: usize,
        buf: &'a mut [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async move {
            if buf.len() < PACKET_SIZE {
                return Err(FsError::InvalidArgument);
            }

            // Wait until at least one event is buffered.
            loop {
                {
                    let mut events = EVENT_BUF.lock();
                    if let Some(evt) = events.pop() {
                        // Serialize the packet into the user buffer.
                        let max_events = buf.len() / PACKET_SIZE;
                        let bytes: [u8; PACKET_SIZE] = unsafe { core::mem::transmute(evt) };
                        buf[..PACKET_SIZE].copy_from_slice(&bytes);

                        // Copy additional buffered events if space permits.
                        let mut count = 1;
                        while count < max_events {
                            if let Some(extra) = events.pop() {
                                let off = count * PACKET_SIZE;
                                let bytes: [u8; PACKET_SIZE] =
                                    unsafe { core::mem::transmute(extra) };
                                buf[off..off + PACKET_SIZE].copy_from_slice(&bytes);
                                count += 1;
                            } else {
                                break;
                            }
                        }
                        return Ok(count * PACKET_SIZE);
                    }
                }

                // No events — register waker and yield.
                core::future::poll_fn(|cx| {
                    // Check again under lock to avoid lost wakeup.
                    let has_events = !EVENT_BUF.lock().is_empty();
                    if has_events {
                        core::task::Poll::Ready(())
                    } else {
                        *READ_WAKER.lock() = Some(cx.waker().clone());
                        core::task::Poll::Pending
                    }
                })
                .await;
            }
        })
    }

    fn write<'a>(
        &'a self,
        _offset: usize,
        _buf: &'a [u8],
    ) -> Pin<Box<dyn Future<Output = Result<usize, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotSupported) })
    }

    fn lookup<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn readdir(&self) -> Pin<Box<dyn Future<Output = Result<Vec<DirEntry>, FsError>> + Send + '_>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn create<'a>(
        &'a self,
        _name: &'a str,
        _itype: InodeType,
        _perms: Permissions,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<dyn Inode>, FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn unlink<'a>(
        &'a self,
        _name: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), FsError>> + Send + 'a>> {
        Box::pin(async { Err(FsError::NotADirectory) })
    }

    fn poll_readiness(&self, waker: Option<&core::task::Waker>) -> u16 {
        if let Some(w) = waker {
            *READ_WAKER.lock() = Some(w.clone());
        }
        if EVENT_BUF.lock().is_empty() {
            0
        } else {
            hadron_syscall::POLLIN
        }
    }
}
