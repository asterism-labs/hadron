//! Host tests for PCI enumeration and capability parsing.

use std::cell::RefCell;
use std::collections::HashMap;

use hadron_driver_api::pci::{PciAddress, PciBar};

use crate::PciConfigAccess;
use crate::caps::{VirtioPciCfgType, walk_capabilities};
use crate::enumerate::enumerate;
use crate::regs;

/// Mock PCI config space backed by a `HashMap<(bus,dev,func,offset), u32>`.
///
/// Supports the BAR sizing protocol: when `0xFFFF_FFFF` is written, the next
/// read returns the sizing mask. Writing anything else restores the stored value.
struct MockPci {
    config: RefCell<HashMap<(u8, u8, u8, u8), u32>>,
    sizing: RefCell<HashMap<(u8, u8, u8, u8), u32>>,
}

impl MockPci {
    fn new() -> Self {
        Self {
            config: RefCell::new(HashMap::new()),
            sizing: RefCell::new(HashMap::new()),
        }
    }

    /// Set a 32-bit register value.
    fn set_u32(&self, bus: u8, dev: u8, func: u8, offset: u8, val: u32) {
        self.config
            .borrow_mut()
            .insert((bus, dev, func, offset & 0xFC), val);
    }

    /// Set the sizing mask for a BAR register (returned when `0xFFFF_FFFF` is written).
    fn set_bar_sizing(&self, bus: u8, dev: u8, func: u8, offset: u8, sizing: u32) {
        self.sizing
            .borrow_mut()
            .insert((bus, dev, func, offset & 0xFC), sizing);
    }

    /// Set a 16-bit register value within a 32-bit dword.
    fn set_u16(&self, bus: u8, dev: u8, func: u8, offset: u8, val: u16) {
        let aligned = offset & 0xFC;
        let shift = ((offset & 2) as u32) * 8;
        let mask = !(0xFFFFu32 << shift);
        let mut config = self.config.borrow_mut();
        let dword = config
            .get(&(bus, dev, func, aligned))
            .copied()
            .unwrap_or(0xFFFF_FFFF);
        config.insert(
            (bus, dev, func, aligned),
            (dword & mask) | ((val as u32) << shift),
        );
    }

    /// Set an 8-bit register value within a 32-bit dword.
    fn set_u8(&self, bus: u8, dev: u8, func: u8, offset: u8, val: u8) {
        let aligned = offset & 0xFC;
        let shift = ((offset & 3) as u32) * 8;
        let mask = !(0xFFu32 << shift);
        let mut config = self.config.borrow_mut();
        let dword = config
            .get(&(bus, dev, func, aligned))
            .copied()
            .unwrap_or(0xFFFF_FFFF);
        config.insert(
            (bus, dev, func, aligned),
            (dword & mask) | ((val as u32) << shift),
        );
    }
}

impl PciConfigAccess for MockPci {
    unsafe fn read_u32(&self, bus: u8, device: u8, function: u8, offset: u8) -> u32 {
        let key = (bus, device, function, offset & 0xFC);
        self.config
            .borrow()
            .get(&key)
            .copied()
            .unwrap_or(0xFFFF_FFFF)
    }

    unsafe fn write_u32(&self, bus: u8, device: u8, function: u8, offset: u8, val: u32) {
        let key = (bus, device, function, offset & 0xFC);
        if val == 0xFFFF_FFFF {
            // BAR sizing: return the sizing mask on next read.
            if let Some(&sizing) = self.sizing.borrow().get(&key) {
                self.config.borrow_mut().insert(key, sizing);
                return;
            }
        }
        self.config.borrow_mut().insert(key, val);
    }
}

// -- class_name tests --------------------------------------------------------

#[test]
fn class_name_known() {
    assert_eq!(crate::class_name(0x02, 0x00), "Ethernet Controller");
    assert_eq!(crate::class_name(0x06, 0x00), "Host Bridge");
    assert_eq!(crate::class_name(0x01, 0x06), "SATA Controller");
}

#[test]
fn class_name_unknown() {
    assert_eq!(crate::class_name(0xFF, 0xFF), "Unknown");
}

// -- VirtIO cfg type tests ---------------------------------------------------

#[test]
fn virtio_pci_cfg_type_from_u8() {
    assert_eq!(
        VirtioPciCfgType::from_u8(1),
        Some(VirtioPciCfgType::CommonCfg)
    );
    assert_eq!(VirtioPciCfgType::from_u8(5), Some(VirtioPciCfgType::PciCfg));
    assert_eq!(VirtioPciCfgType::from_u8(0), None);
    assert_eq!(VirtioPciCfgType::from_u8(6), None);
}

// -- Enumeration tests -------------------------------------------------------

/// Helper: set up a single device at (bus, dev, func) with given vendor/device/class.
fn setup_device(
    mock: &MockPci,
    bus: u8,
    dev: u8,
    func: u8,
    vendor: u16,
    device_id: u16,
    class: u8,
    subclass: u8,
    header_type: u8,
) {
    mock.set_u16(bus, dev, func, regs::VENDOR_ID, vendor);
    mock.set_u16(bus, dev, func, regs::DEVICE_ID, device_id);
    mock.set_u8(bus, dev, func, regs::CLASS, class);
    mock.set_u8(bus, dev, func, regs::SUBCLASS, subclass);
    mock.set_u8(bus, dev, func, regs::HEADER_TYPE, header_type);
    mock.set_u8(bus, dev, func, regs::REVISION, 0);
    mock.set_u8(bus, dev, func, regs::PROG_IF, 0);
    mock.set_u8(bus, dev, func, regs::INTERRUPT_LINE, 0);
    mock.set_u8(bus, dev, func, regs::INTERRUPT_PIN, 0);
}

#[test]
fn enumerate_empty_bus() {
    let mock = MockPci::new();
    // All reads return 0xFFFF_FFFF (no devices present).
    let devices = enumerate(&mock);
    assert!(devices.is_empty());
}

#[test]
fn enumerate_single_device() {
    let mock = MockPci::new();
    // Host bridge at 0:0.0.
    setup_device(&mock, 0, 0, 0, 0x8086, 0x29C0, 0x06, 0x00, 0x00);
    // Ethernet controller at 0:1.0.
    setup_device(&mock, 0, 1, 0, 0x8086, 0x100E, 0x02, 0x00, 0x00);

    let devices = enumerate(&mock);
    assert_eq!(devices.len(), 2);

    // Host bridge.
    assert_eq!(devices[0].vendor_id, 0x8086);
    assert_eq!(devices[0].device_id, 0x29C0);
    assert_eq!(devices[0].address.bus, 0);
    assert_eq!(devices[0].address.device, 0);

    // Ethernet controller.
    assert_eq!(devices[1].vendor_id, 0x8086);
    assert_eq!(devices[1].device_id, 0x100E);
    assert_eq!(devices[1].class, 0x02);
}

#[test]
fn enumerate_multifunction_device() {
    let mock = MockPci::new();
    // Host bridge at 0:0.0 (NOT multi-function).
    setup_device(&mock, 0, 0, 0, 0x8086, 0x29C0, 0x06, 0x00, 0x00);
    // Multi-function device at 0:1.0 with functions 0 and 1.
    setup_device(&mock, 0, 1, 0, 0x1234, 0x5678, 0x0C, 0x03, 0x80); // header_type bit 7 set
    setup_device(&mock, 0, 1, 1, 0x1234, 0x5679, 0x0C, 0x03, 0x00);

    let devices = enumerate(&mock);
    assert_eq!(devices.len(), 3); // host bridge + func 0 + func 1
    assert_eq!(devices[1].address.device, 1);
    assert_eq!(devices[1].address.function, 0);
    assert_eq!(devices[2].address.device, 1);
    assert_eq!(devices[2].address.function, 1);
}

#[test]
fn enumerate_32bit_memory_bar() {
    let mock = MockPci::new();
    setup_device(&mock, 0, 0, 0, 0x8086, 0x29C0, 0x06, 0x00, 0x00);
    setup_device(&mock, 0, 1, 0, 0x1234, 0x5678, 0x02, 0x00, 0x00);

    // BAR0: 32-bit memory at 0xFEB00000, size 64K.
    let bar0_offset = regs::BAR0;
    mock.set_u32(0, 1, 0, bar0_offset, 0xFEB0_0000); // base address
    mock.set_bar_sizing(0, 1, 0, bar0_offset, 0xFFFF_0000); // sizing mask

    let devices = enumerate(&mock);
    assert_eq!(devices.len(), 2);

    match devices[1].bars[0] {
        PciBar::Memory {
            base,
            size,
            prefetchable,
            is_64bit,
        } => {
            assert_eq!(base, 0xFEB0_0000);
            assert_eq!(size, 0x0001_0000); // 64 KiB
            assert!(!prefetchable);
            assert!(!is_64bit);
        }
        other => panic!("expected Memory BAR, got {other:?}"),
    }
}

// -- Capability walking tests ------------------------------------------------

#[test]
fn walk_capabilities_empty() {
    let mock = MockPci::new();
    setup_device(&mock, 0, 0, 0, 0x8086, 0x29C0, 0x06, 0x00, 0x00);
    // Status register bit 4 (cap list) clear → no capabilities.
    mock.set_u16(0, 0, 0, regs::STATUS, 0x0000);

    let addr = PciAddress {
        bus: 0,
        device: 0,
        function: 0,
    };
    assert!(walk_capabilities(&mock, &addr).is_none());
}

#[test]
fn walk_capabilities_single_cap() {
    let mock = MockPci::new();
    setup_device(&mock, 0, 0, 0, 0x8086, 0x29C0, 0x06, 0x00, 0x00);
    // Set capabilities list bit.
    mock.set_u16(0, 0, 0, regs::STATUS, regs::STATUS_CAPABILITIES_LIST);
    // Capabilities pointer → offset 0x40.
    mock.set_u8(0, 0, 0, regs::CAPABILITIES_PTR, 0x40);
    // Cap at 0x40: ID=0x11 (MSI-X), next=0 (end of list).
    mock.set_u8(0, 0, 0, 0x40, regs::CAP_ID_MSIX);
    mock.set_u8(0, 0, 0, 0x41, 0x00);

    let addr = PciAddress {
        bus: 0,
        device: 0,
        function: 0,
    };
    let caps: Vec<_> = walk_capabilities(&mock, &addr).unwrap().collect();
    assert_eq!(caps.len(), 1);
    assert_eq!(caps[0].id, regs::CAP_ID_MSIX);
    assert_eq!(caps[0].offset, 0x40);
}

#[test]
fn walk_capabilities_linked_list() {
    let mock = MockPci::new();
    setup_device(&mock, 0, 0, 0, 0x8086, 0x29C0, 0x06, 0x00, 0x00);
    mock.set_u16(0, 0, 0, regs::STATUS, regs::STATUS_CAPABILITIES_LIST);
    mock.set_u8(0, 0, 0, regs::CAPABILITIES_PTR, 0x40);
    // Cap at 0x40: ID=0x09 (vendor), next=0x50.
    mock.set_u8(0, 0, 0, 0x40, regs::CAP_ID_VENDOR);
    mock.set_u8(0, 0, 0, 0x41, 0x50);
    // Cap at 0x50: ID=0x11 (MSI-X), next=0.
    mock.set_u8(0, 0, 0, 0x50, regs::CAP_ID_MSIX);
    mock.set_u8(0, 0, 0, 0x51, 0x00);

    let addr = PciAddress {
        bus: 0,
        device: 0,
        function: 0,
    };
    let caps: Vec<_> = walk_capabilities(&mock, &addr).unwrap().collect();
    assert_eq!(caps.len(), 2);
    assert_eq!(caps[0].id, regs::CAP_ID_VENDOR);
    assert_eq!(caps[1].id, regs::CAP_ID_MSIX);
}
