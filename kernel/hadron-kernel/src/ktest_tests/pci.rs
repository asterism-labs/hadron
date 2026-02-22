//! PCI enumeration tests — device discovery on QEMU q35 machine.

extern crate alloc;

use hadron_ktest::kernel_test;
use crate::driver_api::pci::PciBar;

// ── Before executor stage ───────────────────────────────────────────────

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_pci_enumerate_finds_devices() {
    let devices = crate::pci::enumerate::enumerate();
    assert!(
        !devices.is_empty(),
        "QEMU q35 should have at least one PCI device"
    );
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_pci_host_bridge_present() {
    let devices = crate::pci::enumerate::enumerate();
    let has_host_bridge = devices
        .iter()
        .any(|d| d.class == 0x06 && d.subclass == 0x00);
    assert!(
        has_host_bridge,
        "q35 should have a Host Bridge (class=0x06, subclass=0x00)"
    );
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_pci_isa_bridge_present() {
    let devices = crate::pci::enumerate::enumerate();
    let has_isa_bridge = devices
        .iter()
        .any(|d| d.class == 0x06 && d.subclass == 0x01);
    assert!(
        has_isa_bridge,
        "q35 should have an ISA Bridge (class=0x06, subclass=0x01)"
    );
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_pci_device_ids_valid() {
    let devices = crate::pci::enumerate::enumerate();
    for dev in &devices {
        assert_ne!(
            dev.vendor_id, 0xFFFF,
            "device at {:?} has invalid vendor_id 0xFFFF",
            dev.address
        );
    }
}

#[kernel_test(stage = "before_executor", timeout = 5)]
fn test_pci_bar_alignment() {
    let devices = crate::pci::enumerate::enumerate();
    for dev in &devices {
        for (i, bar) in dev.bars.iter().enumerate() {
            if let PciBar::Memory { base, size, .. } = bar {
                if *size > 0 {
                    assert_eq!(
                        base % size,
                        0,
                        "device {:?} BAR{} memory base {:#x} not aligned to size {:#x}",
                        dev.address,
                        i,
                        base,
                        size
                    );
                }
            }
        }
    }
}
