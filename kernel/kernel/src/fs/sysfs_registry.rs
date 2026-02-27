//! Global sysfs population API.
//!
//! Provides two entry points:
//!
//! - [`populate_pci`] — called after PCI enumeration to populate
//!   `/sys/bus/pci/devices/<addr>/` with standard sysfs attributes.
//! - [`register_drm`] — called by GPU drivers to add DRM symlinks under
//!   `/sys/class/drm/`.
//!
//! [`set_root`] must be called once during boot with the root [`SysDir`].

extern crate alloc;

use alloc::format;
use alloc::sync::Arc;

use hadron_core::sync::SpinLock;
use hadron_driver_api::pci::PciDeviceInfo;

use crate::fs::sysfs::{SysAttrFile, SysDir, SysSymlink};

// ── Global root ──────────────────────────────────────────────────────────────

/// The global sysfs root directory (set once at boot).
static SYSFS_ROOT: SpinLock<Option<Arc<SysDir>>> = SpinLock::leveled("sysfs_root", 4, None);

/// Set the global sysfs root.
///
/// Must be called exactly once during boot, before any calls to
/// [`populate_pci`] or [`register_drm`].
///
/// # Panics
///
/// Panics if the root has already been set.
pub fn set_root(root: Arc<SysDir>) {
    let mut guard = SYSFS_ROOT.lock();
    assert!(guard.is_none(), "sysfs_registry: root already set");
    *guard = Some(root);
}

// ── PCI population ───────────────────────────────────────────────────────────

/// Populate `/sys/bus/pci/devices/` with one directory per PCI device.
///
/// Each device directory is named `<domain>:<bus>:<device>.<function>` (Linux
/// format, e.g. `0000:00:02.0`) and contains the standard sysfs attributes:
/// `vendor`, `device`, `class`, `irq`, `resource`, `enable`.
///
/// # Panics
///
/// Panics if the sysfs root has not been set.
pub fn populate_pci(devices: &[PciDeviceInfo]) {
    let root = root_dir();

    // Navigate to /sys/bus/pci/devices/ (skeleton created by SysFs::new()).
    let bus_pci_devices = root
        .get_or_create_dir("bus")
        .get_or_create_dir("pci")
        .get_or_create_dir("devices");

    for dev in devices {
        let addr = dev.address;
        // Linux sysfs device address format: 0000:BB:DD.F
        let addr_name = format!(
            "0000:{:02x}:{:02x}.{:x}",
            addr.bus, addr.device, addr.function
        );

        let dev_dir = bus_pci_devices.get_or_create_dir(&addr_name);

        // vendor: e.g. "0x1234\n"
        dev_dir.insert(
            "vendor".into(),
            SysAttrFile::new(format!("0x{:04x}", dev.vendor_id)),
        );

        // device: e.g. "0x1111\n"
        dev_dir.insert(
            "device".into(),
            SysAttrFile::new(format!("0x{:04x}", dev.device_id)),
        );

        // class: e.g. "0x030000\n" (class|subclass|progif)
        let class_val =
            (u32::from(dev.class) << 16) | (u32::from(dev.subclass) << 8) | u32::from(dev.prog_if);
        dev_dir.insert(
            "class".into(),
            SysAttrFile::new(format!("0x{class_val:06x}")),
        );

        // irq: decimal interrupt line
        dev_dir.insert(
            "irq".into(),
            SysAttrFile::new(format!("{}", dev.interrupt_line)),
        );

        // enable: "1\n" (we assume devices are enabled after enumeration)
        dev_dir.insert("enable".into(), SysAttrFile::new("1"));

        // resource: one line per BAR in Linux format
        // Format: start end flags (all hex, 64-bit)
        let mut resource = alloc::string::String::new();
        for bar in &dev.bars {
            let (start, end, flags) = match bar {
                hadron_driver_api::pci::PciBar::Memory { base, size, .. } => {
                    let s = *base;
                    let e = if *size > 0 { s + size - 1 } else { s };
                    (s, e, 0x0000_0200u64) // IORESOURCE_MEM
                }
                hadron_driver_api::pci::PciBar::Io { base, size } => {
                    let s = *base as u64;
                    let e = if *size > 0 { s + *size as u64 - 1 } else { s };
                    (s, e, 0x0000_0100u64) // IORESOURCE_IO
                }
                hadron_driver_api::pci::PciBar::Unused => (0u64, 0u64, 0u64),
            };
            resource.push_str(&format!("0x{start:016x} 0x{end:016x} 0x{flags:016x}\n"));
        }
        if !resource.is_empty() {
            dev_dir.insert("resource".into(), SysAttrFile::new(resource));
        }
    }
}

// ── DRM registration ─────────────────────────────────────────────────────────

/// Register a DRM device under `/sys/class/drm/`.
///
/// Creates two symlinks: `card<N>` and `renderD<M>` pointing to the PCI
/// device address under `/sys/bus/pci/devices/`.
///
/// `name` is the display name, e.g. `"card0"`. `pci_addr` is the PCI address
/// string, e.g. `"0000:00:02.0"`.
///
/// # Panics
///
/// Panics if the sysfs root has not been set.
pub fn register_drm(card_name: &str, render_name: &str, pci_addr: &str) {
    let root = root_dir();
    let drm_dir = root.get_or_create_dir("class").get_or_create_dir("drm");

    let target = format!("/sys/bus/pci/devices/{pci_addr}");

    drm_dir.insert(
        card_name.into(),
        SysSymlink::new(alloc::format!("{target}")),
    );
    drm_dir.insert(
        render_name.into(),
        SysSymlink::new(alloc::format!("{target}")),
    );
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn root_dir() -> Arc<SysDir> {
    SYSFS_ROOT
        .lock()
        .clone()
        .expect("sysfs_registry: root not set")
}
