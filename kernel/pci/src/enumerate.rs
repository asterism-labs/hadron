//! PCI bus enumeration.
//!
//! Walks the PCI bus hierarchy using a [`PciConfigAccess`] implementation,
//! handling multi-function devices and PCI-to-PCI bridges.

use alloc::vec::Vec;

use hadron_driver_api::pci::{PciAddress, PciBar, PciDeviceInfo};

use crate::PciConfigAccess;
use crate::regs;

/// Enumerates all PCI devices across all host-controller buses.
///
/// If the root host controller (0:0.0) is multi-function, each function
/// represents a separate PCI bus domain. Otherwise, only bus 0 is scanned
/// as the root.
pub fn enumerate<C: PciConfigAccess + ?Sized>(cam: &C) -> Vec<PciDeviceInfo> {
    let mut devices = Vec::new();

    // Check if host controller at 0:0.0 is multi-function.
    let header_type = unsafe { cam.read_u8(0, 0, 0, regs::HEADER_TYPE) };
    if header_type & 0x80 == 0 {
        // Single PCI host controller — enumerate bus 0.
        enumerate_bus(cam, 0, &mut devices);
    } else {
        // Multiple PCI host controllers — each function is a separate bus.
        for func in 0..8u8 {
            let vendor = unsafe { cam.read_u16(0, 0, func, regs::VENDOR_ID) };
            if vendor != 0xFFFF {
                enumerate_bus(cam, func, &mut devices);
            }
        }
    }

    devices
}

/// Enumerates all devices on a single PCI bus.
fn enumerate_bus<C: PciConfigAccess + ?Sized>(cam: &C, bus: u8, devices: &mut Vec<PciDeviceInfo>) {
    for device in 0..32u8 {
        enumerate_device(cam, bus, device, devices);
    }
}

/// Probes a single device slot, handling multi-function devices and bridges.
fn enumerate_device<C: PciConfigAccess + ?Sized>(
    cam: &C,
    bus: u8,
    device: u8,
    devices: &mut Vec<PciDeviceInfo>,
) {
    let vendor = unsafe { cam.read_u16(bus, device, 0, regs::VENDOR_ID) };
    if vendor == 0xFFFF {
        return;
    }

    let info = read_device_info(cam, bus, device, 0);
    let is_multi_function = info.header_type & 0x80 != 0;

    // If this is a PCI-to-PCI bridge, recurse into the secondary bus.
    if info.class == 0x06 && info.subclass == 0x04 {
        let secondary = unsafe { cam.read_u8(bus, device, 0, regs::SECONDARY_BUS) };
        if secondary != 0 {
            enumerate_bus(cam, secondary, devices);
        }
    }

    devices.push(info);

    // Scan remaining functions if multi-function device.
    if is_multi_function {
        for func in 1..8u8 {
            let v = unsafe { cam.read_u16(bus, device, func, regs::VENDOR_ID) };
            if v == 0xFFFF {
                continue;
            }
            let func_info = read_device_info(cam, bus, device, func);

            if func_info.class == 0x06 && func_info.subclass == 0x04 {
                let secondary = unsafe { cam.read_u8(bus, device, func, regs::SECONDARY_BUS) };
                if secondary != 0 {
                    enumerate_bus(cam, secondary, devices);
                }
            }

            devices.push(func_info);
        }
    }
}

/// Reads full device information from a single PCI function.
fn read_device_info<C: PciConfigAccess + ?Sized>(
    cam: &C,
    bus: u8,
    dev: u8,
    func: u8,
) -> PciDeviceInfo {
    let vendor_id = unsafe { cam.read_u16(bus, dev, func, regs::VENDOR_ID) };
    let device_id = unsafe { cam.read_u16(bus, dev, func, regs::DEVICE_ID) };
    let revision = unsafe { cam.read_u8(bus, dev, func, regs::REVISION) };
    let prog_if = unsafe { cam.read_u8(bus, dev, func, regs::PROG_IF) };
    let subclass = unsafe { cam.read_u8(bus, dev, func, regs::SUBCLASS) };
    let class = unsafe { cam.read_u8(bus, dev, func, regs::CLASS) };
    let header_type = unsafe { cam.read_u8(bus, dev, func, regs::HEADER_TYPE) };

    let (subsystem_vendor_id, subsystem_device_id) = if header_type & 0x7F == 0 {
        let sv = unsafe { cam.read_u16(bus, dev, func, regs::SUBSYSTEM_VENDOR_ID) };
        let sd = unsafe { cam.read_u16(bus, dev, func, regs::SUBSYSTEM_DEVICE_ID) };
        (sv, sd)
    } else {
        (0, 0)
    };

    let interrupt_line = unsafe { cam.read_u8(bus, dev, func, regs::INTERRUPT_LINE) };
    let interrupt_pin = unsafe { cam.read_u8(bus, dev, func, regs::INTERRUPT_PIN) };

    let bars = decode_bars(cam, bus, dev, func, header_type);

    PciDeviceInfo {
        address: PciAddress {
            bus,
            device: dev,
            function: func,
        },
        vendor_id,
        device_id,
        revision,
        prog_if,
        subclass,
        class,
        header_type,
        subsystem_vendor_id,
        subsystem_device_id,
        interrupt_line,
        interrupt_pin,
        gsi: None,
        bars,
    }
}

/// Decodes Base Address Registers using the standard PCI BAR sizing algorithm.
///
/// Type 0 (general device) headers have 6 BARs; type 1 (bridge) headers have 2.
fn decode_bars<C: PciConfigAccess + ?Sized>(
    cam: &C,
    bus: u8,
    dev: u8,
    func: u8,
    header_type: u8,
) -> [PciBar; 6] {
    let mut bars = [PciBar::Unused; 6];
    let max_bars: usize = if header_type & 0x7F == 1 { 2 } else { 6 };

    let mut i = 0;
    while i < max_bars {
        let offset = regs::BAR0 + (i as u8) * 4;

        let original = unsafe { cam.read_u32(bus, dev, func, offset) };
        unsafe { cam.write_u32(bus, dev, func, offset, 0xFFFF_FFFF) };
        let sizing = unsafe { cam.read_u32(bus, dev, func, offset) };
        unsafe { cam.write_u32(bus, dev, func, offset, original) };

        if sizing == 0 || sizing == 0xFFFF_FFFF {
            i += 1;
            continue;
        }

        if original & 1 != 0 {
            // I/O BAR.
            let mask = sizing & !0x03;
            let size = (!mask).wrapping_add(1) & 0xFFFF;
            if size > 0 {
                bars[i] = PciBar::Io {
                    base: original & !0x03,
                    size,
                };
            }
            i += 1;
        } else {
            // Memory BAR.
            let bar_type = (original >> 1) & 0x03;
            let prefetchable = original & 0x08 != 0;
            let is_64bit = bar_type == 2;

            if is_64bit && i + 1 < max_bars {
                let next_offset = regs::BAR0 + ((i + 1) as u8) * 4;
                let original_high = unsafe { cam.read_u32(bus, dev, func, next_offset) };

                unsafe { cam.write_u32(bus, dev, func, next_offset, 0xFFFF_FFFF) };
                let sizing_high = unsafe { cam.read_u32(bus, dev, func, next_offset) };
                unsafe { cam.write_u32(bus, dev, func, next_offset, original_high) };

                let base = (u64::from(original_high) << 32) | u64::from(original & !0x0F);
                let mask64 = (u64::from(sizing_high) << 32) | u64::from(sizing & !0x0F);
                let size = (!mask64).wrapping_add(1);

                bars[i] = PciBar::Memory {
                    base,
                    size,
                    prefetchable,
                    is_64bit: true,
                };
                i += 2;
            } else {
                let mask = sizing & !0x0F;
                let size = u64::from((!mask).wrapping_add(1));
                bars[i] = PciBar::Memory {
                    base: u64::from(original & !0x0F),
                    size,
                    prefetchable,
                    is_64bit: false,
                };
                i += 1;
            }
        }
    }

    bars
}
