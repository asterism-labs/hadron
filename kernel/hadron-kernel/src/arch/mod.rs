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
        unsafe { hadron_core::percpu::init_gs_base() };
        unsafe { hadron_core::arch::x86_64::syscall::init() };
    }
    #[cfg(target_arch = "aarch64")]
    {
        aarch64::cpu_init();
    }
}

/// Architecture-specific platform initialization (ACPI, PCI, interrupt controllers, timers).
pub fn platform_init(boot_info: &impl crate::boot::BootInfo) {
    #[cfg(target_arch = "x86_64")]
    {
        // 1. Initialize ACPI, interrupt controllers, and timers.
        x86_64::acpi::init(boot_info.rsdp_address());

        // 2. PCI enumeration and device tree.
        let pci_devices = hadron_drivers::pci::enumerate::enumerate();
        hadron_core::kinfo!("PCI: found {} devices", pci_devices.len());

        let tree = hadron_drivers::bus::DeviceTree::build(&pci_devices);
        tree.print();

        // 3. Driver discovery and matching.
        let pci_entries = hadron_drivers::registry::pci_driver_entries();
        let platform_entries = hadron_drivers::registry::platform_driver_entries();
        hadron_core::kinfo!(
            "Drivers: {} PCI, {} platform registered",
            pci_entries.len(),
            platform_entries.len()
        );

        hadron_drivers::registry::match_pci_drivers(
            &pci_devices,
            &crate::services::KERNEL_SERVICES,
        );
        let platform_devs: alloc::vec::Vec<_> = tree.platform_devices().collect();
        hadron_drivers::registry::match_platform_drivers(
            &platform_devs,
            &crate::services::KERNEL_SERVICES,
        );
    }
    #[cfg(target_arch = "aarch64")]
    {
        aarch64::platform_init(boot_info);
    }
}

/// Spawn arch-specific async tasks (serial echo, keyboard, etc.).
pub fn spawn_platform_tasks() {
    #[cfg(target_arch = "x86_64")]
    {
        // Serial echo task — reads from COM1 and echoes back.
        crate::sched::spawn_with(
            async {
                use hadron_driver_api::serial::SerialPort;
                use hadron_drivers::uart16550::{COM1, Uart16550};

                let uart = Uart16550::new(COM1);
                let serial = match hadron_drivers::serial_async::AsyncSerial::new(
                    uart,
                    4,
                    &crate::services::KERNEL_SERVICES,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        hadron_core::kerr!("[serial-echo] Failed to init: {}", e);
                        return;
                    }
                };

                hadron_core::kdebug!("[serial-echo] Listening on COM1...");
                loop {
                    match serial.read_byte().await {
                        Ok(byte) => {
                            if byte == b'\r' {
                                let _ = serial.write_byte(b'\r').await;
                                let _ = serial.write_byte(b'\n').await;
                            } else {
                                let _ = serial.write_byte(byte).await;
                            }
                        }
                        Err(e) => {
                            hadron_core::kerr!("[serial-echo] Read error: {}", e);
                        }
                    }
                }
            },
            crate::sched::TaskMeta::new("serial-echo"),
        );

        // Keyboard echo task — reads PS/2 key events and logs them.
        crate::sched::spawn_with(
            async {
                use hadron_driver_api::input::KeyboardDevice;

                let kbd = match hadron_drivers::keyboard_async::AsyncKeyboard::new(
                    &crate::services::KERNEL_SERVICES,
                ) {
                    Ok(k) => k,
                    Err(e) => {
                        hadron_core::kerr!("[kbd] Failed: {}", e);
                        return;
                    }
                };
                hadron_core::kdebug!("[kbd] Listening for key events...");
                loop {
                    match kbd.read_event().await {
                        Ok(event) => {
                            if event.pressed {
                                hadron_core::kdebug!("[kbd] Key {:?} pressed", event.key);
                            }
                        }
                        Err(e) => hadron_core::kerr!("[kbd] Error: {}", e),
                    }
                }
            },
            crate::sched::TaskMeta::new("kbd-echo"),
        );
    }
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
