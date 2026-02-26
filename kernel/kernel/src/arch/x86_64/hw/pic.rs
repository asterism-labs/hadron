//! 8259 PIC (Programmable Interrupt Controller) driver.
//!
//! Provides remapping to vectors 32-47. In APIC mode, all IRQs are masked
//! and the APIC takes over. In legacy mode, specific IRQs are enabled via
//! [`remap_and_enable`] for PIC-based interrupt delivery.

use crate::arch::x86_64::Port;

const PIC1_CMD: u16 = 0x20;
const PIC1_DATA: u16 = 0x21;
const PIC2_CMD: u16 = 0xA0;
const PIC2_DATA: u16 = 0xA1;

/// ICW1: Initialize + ICW4 needed
const ICW1_INIT: u8 = 0x11;
/// ICW4: 8086 mode
const ICW4_8086: u8 = 0x01;

/// Remaps the 8259 PIC to vectors 32-47, then masks all IRQ lines.
///
/// After this call, the PIC will not generate any interrupts. The APIC
/// should be used instead.
///
/// # Safety
///
/// Must be called with interrupts disabled. Must only be called once.
pub unsafe fn remap_and_disable() {
    let pic1_cmd = Port::<u8>::new(PIC1_CMD);
    let pic1_data = Port::<u8>::new(PIC1_DATA);
    let pic2_cmd = Port::<u8>::new(PIC2_CMD);
    let pic2_data = Port::<u8>::new(PIC2_DATA);

    // ICW1: Start initialization sequence
    // SAFETY: Writing PIC command ports during initialization is safe.
    unsafe {
        pic1_cmd.write(ICW1_INIT);
        io_wait();
        pic2_cmd.write(ICW1_INIT);
        io_wait();
    }

    // ICW2: Vector offsets
    // SAFETY: Writing PIC data ports during initialization is safe.
    unsafe {
        pic1_data.write(32); // Master: vectors 32-39
        io_wait();
        pic2_data.write(40); // Slave: vectors 40-47
        io_wait();
    }

    // ICW3: Cascading
    // SAFETY: Writing PIC data ports during initialization is safe.
    unsafe {
        pic1_data.write(4); // Slave on IRQ2
        io_wait();
        pic2_data.write(2); // Cascade identity
        io_wait();
    }

    // ICW4: 8086 mode
    // SAFETY: Writing PIC data ports during initialization is safe.
    unsafe {
        pic1_data.write(ICW4_8086);
        io_wait();
        pic2_data.write(ICW4_8086);
        io_wait();
    }

    // Mask all IRQ lines on both PICs
    // SAFETY: Masking all IRQs disables PIC interrupts so the APIC can take over.
    unsafe {
        pic1_data.write(0xFF);
        pic2_data.write(0xFF);
    }
}

/// Remaps the 8259 PIC to vectors 32-47 and applies the given IRQ mask.
///
/// Bits set to 0 = IRQ enabled, bits set to 1 = IRQ masked.
/// Low 8 bits control master (IRQs 0-7), high 8 bits control slave (IRQs 8-15).
///
/// # Safety
///
/// Must be called with interrupts disabled. Must only be called once.
pub unsafe fn remap_and_enable(mask: u16) {
    let pic1_cmd = Port::<u8>::new(PIC1_CMD);
    let pic1_data = Port::<u8>::new(PIC1_DATA);
    let pic2_cmd = Port::<u8>::new(PIC2_CMD);
    let pic2_data = Port::<u8>::new(PIC2_DATA);

    // SAFETY: Writing PIC command/data ports during initialization is safe.
    unsafe {
        // ICW1: Start initialization sequence.
        pic1_cmd.write(ICW1_INIT);
        io_wait();
        pic2_cmd.write(ICW1_INIT);
        io_wait();

        // ICW2: Vector offsets.
        pic1_data.write(32); // Master: vectors 32-39
        io_wait();
        pic2_data.write(40); // Slave: vectors 40-47
        io_wait();

        // ICW3: Cascading.
        pic1_data.write(4); // Slave on IRQ2
        io_wait();
        pic2_data.write(2); // Cascade identity
        io_wait();

        // ICW4: 8086 mode.
        pic1_data.write(ICW4_8086);
        io_wait();
        pic2_data.write(ICW4_8086);
        io_wait();

        // Apply caller-specified mask.
        pic1_data.write(mask as u8);
        pic2_data.write((mask >> 8) as u8);
    }
}

/// Sends End-of-Interrupt to the PIC for the given IRQ line.
///
/// For IRQs 8-15 (slave PIC), EOI must be sent to both the slave and master.
///
/// # Safety
///
/// Must only be called from an interrupt handler for a PIC-delivered IRQ.
pub unsafe fn send_eoi(irq: u8) {
    // SAFETY: Writing the EOI command byte to the PIC command port is safe
    // when called from a legitimate interrupt context.
    unsafe {
        if irq >= 8 {
            Port::<u8>::new(PIC2_CMD).write(0x20); // EOI to slave
        }
        Port::<u8>::new(PIC1_CMD).write(0x20); // EOI to master
    }
}

/// Unmasks (enables) a single IRQ line on the PIC.
///
/// # Safety
///
/// Must be called with interrupts disabled or from a context where PIC
/// data port access is safe.
pub unsafe fn unmask(irq: u8) {
    // SAFETY: Reading and writing PIC data ports to adjust the IRQ mask.
    unsafe {
        if irq < 8 {
            let port = Port::<u8>::new(PIC1_DATA);
            let val = port.read();
            port.write(val & !(1 << irq));
        } else {
            let port = Port::<u8>::new(PIC2_DATA);
            let val = port.read();
            port.write(val & !(1 << (irq - 8)));
        }
    }
}

/// Masks (disables) a single IRQ line on the PIC.
///
/// # Safety
///
/// Must be called with interrupts disabled or from a context where PIC
/// data port access is safe.
pub unsafe fn mask(irq: u8) {
    // SAFETY: Reading and writing PIC data ports to adjust the IRQ mask.
    unsafe {
        if irq < 8 {
            let port = Port::<u8>::new(PIC1_DATA);
            let val = port.read();
            port.write(val | (1 << irq));
        } else {
            let port = Port::<u8>::new(PIC2_DATA);
            let val = port.read();
            port.write(val | (1 << (irq - 8)));
        }
    }
}

/// Small I/O delay by writing to an unused port.
#[inline]
fn io_wait() {
    let port = Port::<u8>::new(0x80);
    // SAFETY: Port 0x80 is the POST diagnostic port, writing 0 is harmless
    // and serves as an I/O delay.
    unsafe { port.write(0) };
}
