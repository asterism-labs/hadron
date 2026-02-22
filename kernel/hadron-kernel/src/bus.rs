//! Hierarchical device tree built during boot.
//!
//! Organizes discovered PCI devices, ACPI-enumerated platform devices, and
//! bus placeholders into a tree structure for logging and driver matching.

use alloc::string::String;
use alloc::vec::Vec;

use crate::driver_api::acpi_device::AcpiDeviceInfo;
use crate::driver_api::pci::PciDeviceInfo;

use crate::pci::enumerate::class_name;

/// Information about a device in the tree.
pub enum DeviceInfo {
    /// Virtual root node.
    Root,
    /// PCI bus (one per unique bus number found during enumeration).
    PciBus {
        /// PCI bus number.
        bus_number: u8,
    },
    /// Discovered PCI device/function.
    PciDevice(PciDeviceInfo),
    /// ACPI-discovered platform device.
    AcpiDevice {
        /// Human-readable HID string for display.
        hid_string: String,
    },
    /// Platform bus grouping node for firmware-described devices.
    PlatformBus,
    /// USB bus placeholder for future expansion.
    UsbBus,
}

/// A node in the device tree.
pub struct DeviceNode {
    /// Display name of this node.
    pub name: String,
    /// Device-specific information.
    pub info: DeviceInfo,
    /// Name of the matched driver, if any.
    pub driver_name: Option<&'static str>,
    /// Child nodes.
    pub children: Vec<DeviceNode>,
}

/// Hierarchical device tree built once during boot.
pub struct DeviceTree {
    root: DeviceNode,
}

impl DeviceTree {
    /// Builds the device tree from enumerated PCI devices and ACPI
    /// namespace platform devices.
    #[must_use]
    pub fn build(pci_devices: &[PciDeviceInfo], acpi_devices: &[AcpiDeviceInfo]) -> Self {
        let mut root = DeviceNode {
            name: String::from("root"),
            info: DeviceInfo::Root,
            driver_name: None,
            children: Vec::new(),
        };

        // Group PCI devices by bus number.
        let mut bus_numbers: Vec<u8> = pci_devices.iter().map(|d| d.address.bus).collect();
        bus_numbers.sort_unstable();
        bus_numbers.dedup();

        for bus_num in bus_numbers {
            let children: Vec<DeviceNode> = pci_devices
                .iter()
                .filter(|d| d.address.bus == bus_num)
                .map(|d| DeviceNode {
                    name: alloc::format!("{}", d.address),
                    info: DeviceInfo::PciDevice(*d),
                    driver_name: None,
                    children: Vec::new(),
                })
                .collect();

            root.children.push(DeviceNode {
                name: alloc::format!("pci{bus_num}"),
                info: DeviceInfo::PciBus {
                    bus_number: bus_num,
                },
                driver_name: None,
                children,
            });
        }

        // Add platform devices from ACPI namespace.
        let platform_children: Vec<DeviceNode> = acpi_devices
            .iter()
            .map(|dev| {
                let hid_string = format_hid(&dev.hid);
                DeviceNode {
                    name: alloc::format!("{}", dev.path),
                    info: DeviceInfo::AcpiDevice { hid_string },
                    driver_name: None,
                    children: Vec::new(),
                }
            })
            .collect();

        root.children.push(DeviceNode {
            name: String::from("platform"),
            info: DeviceInfo::PlatformBus,
            driver_name: None,
            children: platform_children,
        });

        // USB bus placeholder.
        root.children.push(DeviceNode {
            name: String::from("usb"),
            info: DeviceInfo::UsbBus,
            driver_name: None,
            children: Vec::new(),
        });

        Self { root }
    }

    /// Prints the device tree to the kernel log.
    pub fn print(&self) {
        crate::kprintln!("Device Tree:");
        print_children(&self.root.children, "");
    }

    /// Iterates all PCI devices in the tree.
    pub fn pci_devices(&self) -> impl Iterator<Item = &PciDeviceInfo> {
        self.root
            .children
            .iter()
            .flat_map(|bus_node| bus_node.children.iter())
            .filter_map(|node| match &node.info {
                DeviceInfo::PciDevice(info) => Some(info),
                _ => None,
            })
    }
}

/// Format an AML value as a human-readable HID string.
///
/// ACPI `_HID` values can be EISA IDs (Buffer or Integer) or plain strings.
/// Integer `_HID` values are compressed EISA IDs per ACPI spec §6.1.5.
fn format_hid(value: &hadron_acpi::aml::value::AmlValue) -> String {
    use hadron_acpi::aml::value::{AmlValue, EisaId};
    match value {
        AmlValue::EisaId(id) => {
            let decoded = id.decode();
            String::from(core::str::from_utf8(&decoded).unwrap_or("?"))
        }
        AmlValue::Integer(v) => {
            let id = EisaId { raw: *v as u32 };
            let decoded = id.decode();
            String::from(core::str::from_utf8(&decoded).unwrap_or("?"))
        }
        AmlValue::String(s) => String::from(s.as_str()),
        _ => String::from("?"),
    }
}

/// Recursively prints tree children with box-drawing indentation.
fn print_children(children: &[DeviceNode], prefix: &str) {
    let count = children.len();
    for (i, child) in children.iter().enumerate() {
        let is_last = i == count - 1;
        let connector = if is_last { "└── " } else { "├── " };
        let child_prefix = if is_last {
            alloc::format!("{prefix}    ")
        } else {
            alloc::format!("{prefix}│   ")
        };

        match &child.info {
            DeviceInfo::PciDevice(dev) => {
                crate::kprintln!(
                    "{prefix}{connector}{} {} [{:04x}:{:04x}]",
                    child.name,
                    class_name(dev.class, dev.subclass),
                    dev.vendor_id,
                    dev.device_id,
                );
            }
            DeviceInfo::AcpiDevice { hid_string } => {
                crate::kprintln!("{prefix}{connector}{} ({hid_string})", child.name);
            }
            DeviceInfo::PlatformBus | DeviceInfo::UsbBus => {
                if child.children.is_empty() {
                    crate::kprintln!("{prefix}{connector}{} (no devices)", child.name);
                } else {
                    crate::kprintln!("{prefix}{connector}{}", child.name);
                }
            }
            _ => {
                crate::kprintln!("{prefix}{connector}{}", child.name);
            }
        }

        if !child.children.is_empty() {
            print_children(&child.children, &child_prefix);
        }
    }
}
