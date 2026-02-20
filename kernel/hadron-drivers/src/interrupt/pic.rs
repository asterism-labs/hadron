//! 8259 PIC (Programmable Interrupt Controller) driver.
//!
//! Provides just enough functionality to remap the PIC to vectors 32-47
//! and then mask all IRQs so the APIC can take over.

use hadron_kernel::arch::x86_64::Port;

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

/// Small I/O delay by writing to an unused port.
#[inline]
fn io_wait() {
    let port = Port::<u8>::new(0x80);
    // SAFETY: Port 0x80 is the POST diagnostic port, writing 0 is harmless
    // and serves as an I/O delay.
    unsafe { port.write(0) };
}
