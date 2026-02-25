//! Architecture-specific modules and uniform facade.

#[cfg(target_arch = "aarch64")]
pub mod aarch64;
#[cfg(target_arch = "x86_64")]
pub mod x86_64;

// --- Arch facade: uniform API re-exported from the active arch ---

/// Architecture-specific CPU initialization (GDT+IDT on x86_64, exception vectors on aarch64).
pub fn cpu_init() {
    #[cfg(target_arch = "x86_64")]
    {
        unsafe { x86_64::gdt::init() };
        unsafe { x86_64::idt::init() };
        unsafe { crate::percpu::init_gs_base() };
        unsafe { crate::arch::x86_64::syscall::init() };
        x86_64::cpuid::init();
        #[cfg(hadron_kernel_fpu)]
        unsafe {
            x86_64::fpu::enable_fpu_support();
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        aarch64::cpu_init();
    }
}

/// Architecture-specific platform initialization (ACPI, PCI, interrupt controllers, timers).
#[allow(unused_variables)]
pub fn platform_init(boot_info: &impl crate::boot::BootInfo) {
    #[cfg(target_arch = "x86_64")]
    {
        // 1. Platform initialization (ACPI or legacy).
        #[cfg(hadron_acpi)]
        x86_64::acpi::init(boot_info.rsdp_address());

        #[cfg(not(hadron_acpi))]
        x86_64::legacy::init();

        // 2. PCI enumeration + driver matching (requires PCI).
        #[cfg(hadron_pci)]
        {
            let mut pci_devices = crate::pci::enumerate::enumerate();
            crate::kinfo!("PCI: found {} devices", pci_devices.len());
            crate::pci::enumerate::apply_prt_routing(&mut pci_devices);

            // ACPI platform device enumeration (if ACPI is available).
            #[cfg(hadron_acpi)]
            let acpi_devices = acpi_platform_devices();
            #[cfg(not(hadron_acpi))]
            let acpi_devices = alloc::vec::Vec::new();

            let tree = crate::bus::DeviceTree::build(&pci_devices, &acpi_devices);
            tree.print();

            let pci_entries = crate::drivers::registry::pci_driver_entries();
            let platform_entries = crate::drivers::registry::platform_driver_entries();
            crate::kinfo!(
                "Drivers: {} PCI, {} platform registered",
                pci_entries.len(),
                platform_entries.len()
            );

            crate::drivers::registry::match_pci_drivers(&pci_devices);
            crate::drivers::registry::match_platform_drivers(&acpi_devices);
        }

        // Platform-only drivers when PCI is off but ACPI is on.
        #[cfg(all(not(hadron_pci), hadron_acpi))]
        {
            let acpi_devices = acpi_platform_devices();
            crate::drivers::registry::match_platform_drivers(&acpi_devices);
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        aarch64::platform_init(boot_info);
    }
}

/// Builds the list of ACPI platform devices from the namespace.
#[cfg(all(target_arch = "x86_64", hadron_acpi))]
fn acpi_platform_devices() -> alloc::vec::Vec<crate::driver_api::acpi_device::AcpiDeviceInfo> {
    use crate::driver_api::acpi_device::AcpiDeviceInfo;
    use hadron_acpi::aml::value::AmlValue;

    let devices: alloc::vec::Vec<AcpiDeviceInfo> = x86_64::acpi::Acpi::with_namespace(|ns| {
        ns.devices()
            .filter(|d| d.hid.is_some())
            .filter(|d| !matches!(&d.hid, Some(AmlValue::Unresolved)))
            .map(|d| AcpiDeviceInfo {
                path: d.path,
                hid: d.hid.unwrap(),
                cid: d.cid,
                uid: match d.uid {
                    Some(AmlValue::Integer(v)) => Some(v),
                    _ => None,
                },
                resources: d.resources.clone(),
            })
            .collect()
    })
    .unwrap_or_default();
    crate::kinfo!("Platform: {} ACPI devices with _HID", devices.len());
    devices
}

/// Spawn arch-specific async tasks.
///
/// The serial echo task is now spawned by the serial driver during probe
/// via the [`TaskSpawner`](crate::driver_api::TaskSpawner) capability. This
/// function handles any remaining arch-specific platform tasks.
pub fn spawn_platform_tasks() {
    #[cfg(target_arch = "aarch64")]
    {
        aarch64::spawn_platform_tasks();
    }
}

/// Arch-uniform interrupt facade.
pub mod interrupts {
    #[cfg(target_arch = "aarch64")]
    pub use super::aarch64::interrupts::*;
    #[cfg(target_arch = "x86_64")]
    pub use super::x86_64::interrupts::*;
}
