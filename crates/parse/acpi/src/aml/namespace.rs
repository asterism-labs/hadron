//! AML namespace tree builder (requires `alloc` feature).
//!
//! The [`NamespaceBuilder`] implements [`AmlVisitor`] to collect namespace
//! nodes into a [`Namespace`] tree. This is useful for discovering devices
//! by their `_HID`, `_CID`, `_ADR`, or `_UID` objects.

extern crate alloc;

use alloc::vec::Vec;

use super::path::{AmlPath, NameSeg};
use super::value::AmlValue;
use super::visitor::AmlVisitor;
use crate::resource::{self, AcpiResource};

/// Kind of a namespace node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A DefScope.
    Scope,
    /// A DefDevice.
    Device,
    /// A DefMethod.
    Method,
    /// A DefThermalZone.
    ThermalZone,
    /// A DefProcessor.
    Processor,
    /// A DefPowerRes.
    PowerResource,
    /// A DefName.
    Name,
}

/// A single node in the ACPI namespace.
#[derive(Debug, Clone)]
pub struct NamespaceNode {
    /// Full path to this node.
    pub path: AmlPath,
    /// Local name of this node.
    pub name: NameSeg,
    /// Kind of namespace object.
    pub kind: NodeKind,
    /// `_HID` value, if this is a device with a hardware ID.
    pub hid: Option<AmlValue>,
    /// `_CID` value, if this is a device with a compatible ID.
    pub cid: Option<AmlValue>,
    /// `_ADR` value, if this is a device with an address.
    pub adr: Option<AmlValue>,
    /// `_UID` value, if this is a device with a unique ID.
    pub uid: Option<AmlValue>,
    /// Decoded resources from `_CRS`.
    pub resources: Vec<AcpiResource>,
    /// PCI interrupt routing entries from `_PRT`.
    pub prt: Vec<PrtEntry>,
}

/// The collected ACPI namespace.
pub struct Namespace {
    nodes: Vec<NamespaceNode>,
}

impl Namespace {
    /// Returns an iterator over all device nodes in the namespace.
    pub fn devices(&self) -> impl Iterator<Item = &NamespaceNode> {
        self.nodes.iter().filter(|n| n.kind == NodeKind::Device)
    }

    /// Find a device by its `_HID` EISA ID.
    ///
    /// Compares the raw 32-bit EISA ID value. Returns the first match.
    #[must_use]
    pub fn find_device_by_hid(&self, eisa_raw: u32) -> Option<&NamespaceNode> {
        self.devices().find(|n| {
            matches!(
                n.hid,
                Some(AmlValue::EisaId(ref id)) if id.raw == eisa_raw
            )
        })
    }

    /// Find a device by its `_HID` string value.
    ///
    /// Compares the inline string contents. Returns the first match.
    #[must_use]
    pub fn find_device_by_hid_string(&self, hid: &str) -> Option<&NamespaceNode> {
        self.devices().find(|n| {
            matches!(
                n.hid,
                Some(AmlValue::String(ref s)) if s.as_str() == hid
            )
        })
    }

    /// Returns all nodes in the namespace.
    #[must_use]
    pub fn nodes(&self) -> &[NamespaceNode] {
        &self.nodes
    }
}

/// A PCI interrupt routing entry from `_PRT`.
#[derive(Debug, Clone, Copy)]
pub struct PrtEntry {
    /// Device address (high word = device number, low word = function or 0xFFFF).
    pub address: u64,
    /// Interrupt pin: 0 = INTA, 1 = INTB, 2 = INTC, 3 = INTD.
    pub pin: u8,
    /// Global System Interrupt number.
    pub gsi: u32,
}

/// `_HID` name segment.
const HID_SEG: NameSeg = NameSeg(*b"_HID");
/// `_CID` name segment.
const CID_SEG: NameSeg = NameSeg(*b"_CID");
/// `_ADR` name segment.
const ADR_SEG: NameSeg = NameSeg(*b"_ADR");
/// `_UID` name segment.
const UID_SEG: NameSeg = NameSeg(*b"_UID");
/// `_CRS` name segment.
const CRS_SEG: NameSeg = NameSeg(*b"_CRS");

/// Visitor that builds a [`Namespace`] from an AML walk.
pub struct NamespaceBuilder {
    nodes: Vec<NamespaceNode>,
}

impl NamespaceBuilder {
    /// Create a new namespace builder.
    #[must_use]
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Consume the builder and return the completed namespace.
    #[must_use]
    pub fn build(self) -> Namespace {
        Namespace { nodes: self.nodes }
    }
}

impl Default for NamespaceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl AmlVisitor for NamespaceBuilder {
    fn device(&mut self, path: &AmlPath, name: NameSeg) {
        self.nodes.push(NamespaceNode {
            path: *path,
            name,
            kind: NodeKind::Device,
            hid: None,
            cid: None,
            adr: None,
            uid: None,
            resources: Vec::new(),
            prt: Vec::new(),
        });
    }

    fn name_object(&mut self, _path: &AmlPath, name: NameSeg, value: &AmlValue) {
        // Attach well-known name objects to the most recent device.
        if let Some(device) = self.nodes.last_mut() {
            if device.kind == NodeKind::Device {
                if name == HID_SEG {
                    device.hid = Some(*value);
                } else if name == CID_SEG {
                    device.cid = Some(*value);
                } else if name == ADR_SEG {
                    device.adr = Some(*value);
                } else if name == UID_SEG {
                    device.uid = Some(*value);
                }
            }
        }
    }

    fn resource_template(&mut self, _path: &AmlPath, name: NameSeg, data: &[u8]) {
        // Attach _CRS resources to the most recent device.
        if name == CRS_SEG {
            if let Some(device) = self.nodes.last_mut() {
                if device.kind == NodeKind::Device {
                    device.resources = resource::parse_resource_template(data).collect();
                }
            }
        }
    }

    fn prt_entry(&mut self, _path: &AmlPath, address: u64, pin: u8, gsi: u32) {
        // Attach _PRT entries to the most recent device.
        if let Some(device) = self.nodes.last_mut() {
            if device.kind == NodeKind::Device {
                device.prt.push(PrtEntry { address, pin, gsi });
            }
        }
    }

    fn method(&mut self, path: &AmlPath, name: NameSeg, _arg_count: u8, _serialized: bool) {
        self.nodes.push(NamespaceNode {
            path: *path,
            name,
            kind: NodeKind::Method,
            hid: None,
            cid: None,
            adr: None,
            uid: None,
            resources: Vec::new(),
            prt: Vec::new(),
        });
    }

    fn thermal_zone(&mut self, path: &AmlPath, name: NameSeg) {
        self.nodes.push(NamespaceNode {
            path: *path,
            name,
            kind: NodeKind::ThermalZone,
            hid: None,
            cid: None,
            adr: None,
            uid: None,
            resources: Vec::new(),
            prt: Vec::new(),
        });
    }

    fn processor(&mut self, path: &AmlPath, name: NameSeg, _id: u8) {
        self.nodes.push(NamespaceNode {
            path: *path,
            name,
            kind: NodeKind::Processor,
            hid: None,
            cid: None,
            adr: None,
            uid: None,
            resources: Vec::new(),
            prt: Vec::new(),
        });
    }

    fn power_resource(&mut self, path: &AmlPath, name: NameSeg) {
        self.nodes.push(NamespaceNode {
            path: *path,
            name,
            kind: NodeKind::PowerResource,
            hid: None,
            cid: None,
            adr: None,
            uid: None,
            resources: Vec::new(),
            prt: Vec::new(),
        });
    }
}
