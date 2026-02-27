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

/// Serializes a [`MouseEventPacket`] into a byte array using safe field-level
/// conversion. Avoids `transmute` by writing each field at its known offset.
fn packet_to_bytes(evt: &hadron_syscall::MouseEventPacket) -> [u8; PACKET_SIZE] {
    let mut bytes = [0u8; PACKET_SIZE];
    bytes[0..2].copy_from_slice(&evt.dx.to_ne_bytes());
    bytes[2..4].copy_from_slice(&evt.dy.to_ne_bytes());
    bytes[4] = evt.buttons;
    // bytes[5..8] remain zero (padding).
    bytes
}

/// PS/2 i8042 data port.
const DATA_PORT: u16 = 0x60;

/// PS/2 i8042 status/command port.
const STATUS_PORT: u16 = 0x64;

/// i8042 command: route the next data-port write to the auxiliary (mouse) device.
const CMD_WRITE_MOUSE: u8 = 0xD4;

/// PS/2 device command: enable data reporting (mouse starts sending packets).
const MOUSE_ENABLE_DATA_REPORTING: u8 = 0xF4;

/// PS/2 acknowledge byte.
const PS2_ACK: u8 = 0xFA;

/// Spin-loop iteration limit for waiting on i8042 ports.
const SPIN_TIMEOUT: u32 = 100_000;

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

/// Wait until the i8042 input buffer is empty (ready for a command/data write).
fn wait_input_ready() {
    use crate::arch::x86_64::Port;
    for _ in 0..SPIN_TIMEOUT {
        // SAFETY: Reading the i8042 status port is a standard operation.
        let status = unsafe { Port::<u8>::new(STATUS_PORT).read() };
        // Bit 1: input buffer full — must be clear before writing.
        if status & 0x02 == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

/// Wait until the i8042 output buffer has data to read.
fn wait_output_ready() -> bool {
    use crate::arch::x86_64::Port;
    for _ in 0..SPIN_TIMEOUT {
        // SAFETY: Reading the i8042 status port is a standard operation.
        let status = unsafe { Port::<u8>::new(STATUS_PORT).read() };
        // Bit 0: output buffer full — data available.
        if status & 0x01 != 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Send the "enable data reporting" command (0xF4) to the PS/2 mouse.
///
/// Uses the 0xD4 controller command to route the next data-port byte
/// to the auxiliary (mouse) device, then writes 0xF4 and waits for ACK.
fn enable_mouse_data_reporting() {
    use crate::arch::x86_64::Port;

    // Tell the i8042 that the next data write goes to the mouse.
    wait_input_ready();
    // SAFETY: Writing the i8042 command port is a standard PS/2 operation.
    unsafe { Port::<u8>::new(STATUS_PORT).write(CMD_WRITE_MOUSE) };

    // Send the "enable data reporting" command to the mouse.
    wait_input_ready();
    // SAFETY: Writing to the i8042 data port is a standard PS/2 operation.
    unsafe { Port::<u8>::new(DATA_PORT).write(MOUSE_ENABLE_DATA_REPORTING) };

    // Wait for the mouse to ACK (0xFA).
    if wait_output_ready() {
        // SAFETY: Reading the i8042 data port after output-ready is standard.
        let ack = unsafe { Port::<u8>::new(DATA_PORT).read() };
        if ack != PS2_ACK {
            crate::kwarn!("DevMouse: expected ACK (0xFA), got 0x{:02X}", ack);
        }
    } else {
        crate::kwarn!("DevMouse: timeout waiting for mouse ACK");
    }
}

/// Initialize the PS/2 mouse IRQ handler and enable data reporting.
///
/// Registers the IRQ 12 handler, unmasks it in the I/O APIC, and sends
/// the 0xF4 command to the mouse device so it begins generating packets.
pub fn init() {
    use crate::arch::x86_64::interrupts::dispatch;

    let vector = dispatch::vectors::isa_irq_vector(12);
    dispatch::register_handler(vector, mouse_irq_handler)
        .expect("dev_mouse: failed to register mouse IRQ handler");

    #[cfg(hadron_apic)]
    crate::arch::x86_64::acpi::Acpi::with_io_apic(|ioapic| ioapic.unmask(12));

    #[cfg(not(hadron_apic))]
    // SAFETY: Unmasking IRQ 12 (PS/2 mouse) on the PIC from driver init.
    unsafe {
        crate::arch::x86_64::hw::pic::unmask(12);
    };

    // Enable the mouse device itself — without this, the hardware stays silent.
    enable_mouse_data_reporting();

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

    fn dev_number(&self) -> hadron_fs::DevNumber {
        // Major 13 (input devices), minor 63 (mice).
        hadron_fs::DevNumber::new(13, 63)
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
                        buf[..PACKET_SIZE].copy_from_slice(&packet_to_bytes(&evt));

                        // Copy additional buffered events if space permits.
                        let mut count = 1;
                        while count < max_events {
                            if let Some(extra) = events.pop() {
                                let off = count * PACKET_SIZE;
                                buf[off..off + PACKET_SIZE]
                                    .copy_from_slice(&packet_to_bytes(&extra));
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
        // Register waker first, then check — an event arriving between these
        // two steps will wake the waker we just registered, so no wakeup is lost.
        if let Some(w) = waker {
            *READ_WAKER.lock() = Some(w.clone());
        }
        let ready = !EVENT_BUF.lock().is_empty();
        if ready {
            // Data is already available. Wake immediately in case the interrupt
            // handler already consumed our waker before we checked the buffer.
            if let Some(w) = READ_WAKER.lock().take() {
                w.wake();
            }
            hadron_syscall::POLLIN
        } else {
            0
        }
    }
}
