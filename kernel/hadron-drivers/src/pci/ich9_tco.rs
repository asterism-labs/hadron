//! ICH9 TCO (Total Cost of Ownership) watchdog timer driver.
//!
//! Replaces the ICH9 LPC stub. Drives the TCO watchdog timer embedded in the
//! Q35 chipset's LPC bridge (`0x8086:0x2918`). On expiry the TCO triggers a
//! system reset — with QEMU's `-no-reboot` flag this causes an immediate exit,
//! which the ktest harness uses for hang detection.

extern crate alloc;

use alloc::sync::Arc;

use hadron_kernel::driver_api::error::DriverError;
use hadron_kernel::driver_api::hw::Watchdog;
use hadron_kernel::driver_api::pci::PciDeviceId;

// ---------------------------------------------------------------------------
// PCI ID table
// ---------------------------------------------------------------------------

/// ICH9 LPC/ISA bridge (present on every Q35 machine).
#[cfg(target_os = "none")]
static ID_TABLE: [PciDeviceId; 1] = [PciDeviceId::new(0x8086, 0x2918)];

// ---------------------------------------------------------------------------
// TCO register offsets (relative to TCOBASE)
// ---------------------------------------------------------------------------

/// Reload register — write any value to pet the watchdog.
const TCO_RLD: u16 = 0x00;
/// TCO1 status — bit 3 = first timeout (write-1-to-clear).
const TCO1_STS: u16 = 0x04;
/// TCO2 status — bit 1 = second timeout / reset (write-1-to-clear).
const TCO2_STS: u16 = 0x06;
/// TCO1 control — bit 11 = `TCO_TMR_HLT` (1 = halted).
const TCO1_CNT: u16 = 0x08;
/// Timer initial value — bits 9:0 = tick count (max 1023).
const TCO_TMR: u16 = 0x12;

/// Bit mask for `TCO_TMR_HLT` in `TCO1_CNT`.
const TCO_TMR_HLT: u16 = 1 << 11;
/// Bit mask for first-timeout status in `TCO1_STS`.
const TCO1_TIMEOUT: u16 = 1 << 3;
/// Bit mask for second-timeout (reset) status in `TCO2_STS`.
const TCO2_TIMEOUT: u16 = 1 << 1;
/// Maximum tick value (10-bit field).
const TCO_TMR_MAX: u32 = 1023;

/// `NO_REBOOT` bit in the GCS register (RCBA + 0x3410).
const GCS_NO_REBOOT: u32 = 1 << 5;

// ---------------------------------------------------------------------------
// Driver struct
// ---------------------------------------------------------------------------

/// ICH9 TCO watchdog state. All register access is via x86 port I/O so the
/// struct only stores the computed I/O base address.
struct Ich9TcoWatchdog {
    tcobase: u16,
}

impl Watchdog for Ich9TcoWatchdog {
    fn arm(&self, timeout_secs: u32) {
        // Each TCO tick is ~0.6 s. The TCO fires a first timeout after `ticks`
        // ticks, then a second timeout (system reset) after another `ticks` ticks.
        // Total time to reset = 2 * ticks * 0.6 s  ⇒  ticks = timeout / 1.2
        let ticks = (timeout_secs * 10 / 12).clamp(2, TCO_TMR_MAX);

        // SAFETY: Port I/O to TCO registers we own.
        unsafe {
            use hadron_kernel::arch::x86_64::Port;

            let cnt = Port::<u16>::new(self.tcobase + TCO1_CNT);
            let tmr = Port::<u16>::new(self.tcobase + TCO_TMR);
            let sts1 = Port::<u16>::new(self.tcobase + TCO1_STS);
            let sts2 = Port::<u16>::new(self.tcobase + TCO2_STS);
            let rld = Port::<u16>::new(self.tcobase + TCO_RLD);

            // 1. Halt the timer.
            let cnt_val = cnt.read();
            cnt.write(cnt_val | TCO_TMR_HLT);

            // 2. Set the tick count.
            tmr.write(ticks as u16);

            // 3. Clear pending status bits (w1c).
            sts1.write(TCO1_TIMEOUT);
            sts2.write(TCO2_TIMEOUT);

            // 4. Reload the countdown.
            rld.write(1);

            // 5. Un-halt — countdown begins.
            let cnt_val = cnt.read();
            cnt.write(cnt_val & !TCO_TMR_HLT);
        }
    }

    fn pet(&self) {
        // SAFETY: Port I/O to our TCO reload register.
        unsafe {
            use hadron_kernel::arch::x86_64::Port;
            let rld = Port::<u16>::new(self.tcobase + TCO_RLD);
            rld.write(1);
        }
    }

    fn disarm(&self) {
        // SAFETY: Port I/O to TCO registers we own.
        unsafe {
            use hadron_kernel::arch::x86_64::Port;

            let cnt = Port::<u16>::new(self.tcobase + TCO1_CNT);
            let sts1 = Port::<u16>::new(self.tcobase + TCO1_STS);
            let sts2 = Port::<u16>::new(self.tcobase + TCO2_STS);

            // Halt timer.
            let cnt_val = cnt.read();
            cnt.write(cnt_val | TCO_TMR_HLT);

            // Clear status (w1c).
            sts1.write(TCO1_TIMEOUT);
            sts2.write(TCO2_TIMEOUT);
        }
    }
}

// ---------------------------------------------------------------------------
// Driver registration
// ---------------------------------------------------------------------------

struct Ich9TcoDriver;

#[hadron_driver_macros::hadron_driver(
    name = "ich9-tco",
    kind = pci,
    capabilities = [PciConfig, Mmio],
    pci_ids = &ID_TABLE,
)]
impl Ich9TcoDriver {
    fn probe(
        ctx: DriverContext,
    ) -> Result<hadron_kernel::driver_api::registration::PciDriverRegistration, DriverError> {
        use hadron_kernel::driver_api::capability::{
            CapabilityAccess, MmioCapability, PciConfigCapability,
        };
        use hadron_kernel::driver_api::device_path::DevicePath;
        use hadron_kernel::driver_api::registration::{DeviceSet, PciDriverRegistration};

        let info = ctx.device();
        let pci_config = ctx.capability::<PciConfigCapability>();
        let mmio_cap = ctx.capability::<MmioCapability>();

        // 1. Read PMBASE from LPC PCI config offset 0x40.
        let pmbase = pci_config.read_config_u32(0x40) & 0xFF80;
        if pmbase == 0 {
            hadron_kernel::kwarn!("ich9-tco: PMBASE is zero, cannot initialize TCO watchdog");
            return Ok(PciDriverRegistration {
                devices: DeviceSet::new(),
                lifecycle: None,
            });
        }
        let tcobase = (pmbase as u16) + 0x60;

        // 2. Read RCBA from LPC PCI config offset 0xF0.
        let rcba_raw = pci_config.read_config_u32(0xF0);
        let rcba_base = (rcba_raw & 0xFFFFC000) as u64;

        // 3. Clear NO_REBOOT bit in GCS (RCBA + 0x3410) via MMIO.
        if rcba_base != 0 {
            let gcs_phys = rcba_base + 0x3410;
            let gcs_virt = mmio_cap.phys_to_virt(gcs_phys);
            // SAFETY: GCS is a 32-bit MMIO register at the computed virtual address.
            unsafe {
                let gcs_ptr = gcs_virt as *mut u32;
                let gcs_val = core::ptr::read_volatile(gcs_ptr);
                if gcs_val & GCS_NO_REBOOT != 0 {
                    core::ptr::write_volatile(gcs_ptr, gcs_val & !GCS_NO_REBOOT);
                }
            }
        }

        // 4. Halt timer and clear pending status.
        // SAFETY: Port I/O to TCO registers at the base we just computed.
        unsafe {
            use hadron_kernel::arch::x86_64::Port;

            let cnt = Port::<u16>::new(tcobase + TCO1_CNT);
            let cnt_val = cnt.read();
            cnt.write(cnt_val | TCO_TMR_HLT);

            let sts1 = Port::<u16>::new(tcobase + TCO1_STS);
            let sts2 = Port::<u16>::new(tcobase + TCO2_STS);
            sts1.write(TCO1_TIMEOUT);
            sts2.write(TCO2_TIMEOUT);
        }

        let wd = Arc::new(Ich9TcoWatchdog { tcobase });

        let mut devices = DeviceSet::new();
        let path = DevicePath::pci(
            info.address.bus,
            info.address.device,
            info.address.function,
            "ich9-tco",
            0,
        );
        devices.add_watchdog(path, wd);

        hadron_kernel::kinfo!(
            "ich9-tco: TCO watchdog at TCOBASE={:#06x} (PMBASE={:#06x})",
            tcobase,
            pmbase
        );

        Ok(PciDriverRegistration {
            devices,
            lifecycle: None,
        })
    }
}
