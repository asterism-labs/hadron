//! AML namespace visitor trait.
//!
//! Callers implement [`AmlVisitor`] and override only the callbacks they need.
//! All methods have default empty bodies.

use super::path::{AmlPath, NameSeg};
use super::value::AmlValue;

/// Visitor trait for walking the AML namespace.
///
/// The parser calls these methods as it encounters namespace objects. All
/// methods have default empty implementations so callers only override what
/// they need.
#[allow(unused_variables)]
pub trait AmlVisitor {
    /// Called when entering a new scope (DefScope, DefDevice, etc.).
    fn enter_scope(&mut self, path: &AmlPath) {}

    /// Called when leaving the current scope.
    fn exit_scope(&mut self) {}

    /// Called when a DefDevice is encountered.
    fn device(&mut self, path: &AmlPath, name: NameSeg) {}

    /// Called when a DefName object is encountered with a resolved value.
    fn name_object(&mut self, path: &AmlPath, name: NameSeg, value: &AmlValue) {}

    /// Called when a DefName contains a resource template buffer (e.g., `_CRS`).
    ///
    /// `data` is the raw resource descriptor bytes suitable for parsing with
    /// [`crate::resource::parse_resource_template`].
    fn resource_template(&mut self, path: &AmlPath, name: NameSeg, data: &[u8]) {}

    /// Called when a `_PRT` (PCI Routing Table) package is parsed.
    ///
    /// Each entry is an `(address, pin, gsi)` tuple representing a PCI interrupt
    /// routing from the ACPI namespace.
    fn prt_entry(&mut self, path: &AmlPath, address: u64, pin: u8, gsi: u32) {}

    /// Called when a DefMethod is encountered.
    fn method(&mut self, path: &AmlPath, name: NameSeg, arg_count: u8, serialized: bool) {}

    /// Called when a DefThermalZone is encountered.
    fn thermal_zone(&mut self, path: &AmlPath, name: NameSeg) {}

    /// Called when a DefProcessor is encountered.
    fn processor(&mut self, path: &AmlPath, name: NameSeg, id: u8) {}

    /// Called when a DefPowerRes is encountered.
    fn power_resource(&mut self, path: &AmlPath, name: NameSeg) {}
}
