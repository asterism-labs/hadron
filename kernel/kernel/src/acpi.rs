//! ACPI: finds the tables through the RSDP and copies what the interrupt
//! code needs from the MADT into [`Madt`].
//!
//! [`init`] reads the tables once, at boot, through the kernel's direct map.
//! Firmware places them in ACPI-reclaimable or ACPI NVS memory, which the
//! direct map covers. A table outside it stops boot.

use alloc::vec::Vec;

use ::acpi::{AcpiHandler, AcpiTables, MadtEntry};
use arch_x86_64::PhysAddr;
use arch_x86_64::paging::PAGE_SIZE;

use crate::memory::{self, DIRECT_MAP};
use crate::sync::WriteOnce;

/// MADT flags bit 0: the board has the two legacy 8259 PICs.
const PCAT_COMPAT: u32 = 1 << 0;
/// Local APIC flags bit 0: the processor is enabled.
const LAPIC_ENABLED: u32 = 1 << 0;

/// What the kernel keeps from the MADT.
#[derive(Debug)]
pub(crate) struct Madt {
    /// Physical address of the local APIC registers, after any address
    /// override. Every CPU sees its own local APIC here.
    pub(crate) local_apic: PhysAddr,
    /// Whether the legacy 8259 PICs are present and must be masked.
    pub(crate) has_8259: bool,
    /// The enabled processors, in MADT order. The firmware lists the boot
    /// CPU first.
    pub(crate) cpus: Vec<Cpu>,
    pub(crate) io_apics: Vec<IoApic>,
    /// ISA IRQs that do not reach the I/O APIC input of the same number.
    pub(crate) isa_overrides: Vec<IsaOverride>,
    /// Local APIC pins wired to NMI.
    pub(crate) local_apic_nmis: Vec<LocalApicNmi>,
}

/// An enabled processor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cpu {
    /// The ACPI processor UID, which [`LocalApicNmi`] refers to.
    pub(crate) acpi_id: u8,
    pub(crate) apic_id: u8,
}

/// An I/O APIC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IoApic {
    pub(crate) id: u8,
    /// Physical address of its registers.
    pub(crate) address: PhysAddr,
    /// The first global system interrupt (GSI) it handles.
    pub(crate) gsi_base: u32,
}

/// An ISA IRQ routed to a different GSI, or with a different polarity or
/// trigger mode than ISA's active-high, edge-triggered default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IsaOverride {
    pub(crate) irq: u8,
    pub(crate) gsi: u32,
    /// MPS INTI flags: polarity in bits 0-1, trigger mode in bits 2-3.
    pub(crate) flags: u16,
}

/// A local APIC pin that receives NMIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LocalApicNmi {
    /// The ACPI processor UID, or `None` for every processor.
    pub(crate) acpi_id: Option<u8>,
    /// LINT0 or LINT1.
    pub(crate) lint: u8,
    /// MPS INTI flags: polarity in bits 0-1, trigger mode in bits 2-3.
    pub(crate) flags: u16,
}

static MADT: WriteOnce<Madt> = WriteOnce::new();

/// What the kernel keeps from the MADT.
///
/// # Panics
///
/// Panics before [`init`].
pub(crate) fn madt() -> &'static Madt {
    MADT.get().expect("acpi::init ran")
}

/// Reads the ACPI tables from the RSDP at `rsdp`. Needs the heap.
///
/// # Panics
///
/// Panics if the RSDP or the MADT is missing or invalid, if a table lies
/// outside the direct map, or if called twice.
pub(crate) fn init(rsdp: Option<PhysAddr>) -> &'static Madt {
    let rsdp = rsdp.expect("the bootloader found the ACPI RSDP");
    let tables = AcpiTables::new(rsdp.as_u64(), DirectMapHandler).expect("the ACPI RSDP is valid");
    let madt = tables.madt().expect("ACPI has a valid MADT");
    MADT.init(Madt::read(&madt))
}

impl Madt {
    fn read(table: &::acpi::Madt) -> Self {
        let mut madt = Self {
            local_apic: PhysAddr::new(u64::from(table.local_apic_address)),
            has_8259: table.flags & PCAT_COMPAT != 0,
            cpus: Vec::new(),
            io_apics: Vec::new(),
            isa_overrides: Vec::new(),
            local_apic_nmis: Vec::new(),
        };
        for entry in table.entries() {
            match entry {
                MadtEntry::LocalApic {
                    acpi_processor_id,
                    apic_id,
                    flags,
                } if flags & LAPIC_ENABLED != 0 => madt.cpus.push(Cpu {
                    acpi_id: acpi_processor_id,
                    apic_id,
                }),
                MadtEntry::IoApic {
                    io_apic_id,
                    io_apic_address,
                    gsi_base,
                } => madt.io_apics.push(IoApic {
                    id: io_apic_id,
                    address: PhysAddr::new(u64::from(io_apic_address)),
                    gsi_base,
                }),
                // Bus 0 is ISA, the only bus ACPI defines here.
                MadtEntry::InterruptSourceOverride {
                    bus: 0,
                    source,
                    gsi,
                    flags,
                } => madt.isa_overrides.push(IsaOverride {
                    irq: source,
                    gsi,
                    flags,
                }),
                MadtEntry::LocalApicNmi {
                    acpi_processor_id,
                    flags,
                    lint,
                } => madt.local_apic_nmis.push(LocalApicNmi {
                    acpi_id: (acpi_processor_id != 0xff).then_some(acpi_processor_id),
                    lint,
                    flags,
                }),
                MadtEntry::LocalApicAddressOverride { address } => {
                    madt.local_apic = PhysAddr::new(address);
                }
                _ => {}
            }
        }
        madt
    }
}

/// Reads tables through the kernel's direct map.
struct DirectMapHandler;

// SAFETY: `DIRECT_MAP` never changes once `memory::init` has run, so a
// slice into it stays valid for good, and `map_physical_region` checks that
// every page of the slice is mapped.
unsafe impl AcpiHandler for DirectMapHandler {
    unsafe fn map_physical_region(&self, phys: u64, size: usize) -> &'static [u8] {
        let start = PhysAddr::new(phys);
        let virt = DIRECT_MAP.phys_to_virt(start);
        let end = virt + size as u64;
        let mut page = virt.align_down(PAGE_SIZE);
        while page < end {
            assert!(
                memory::translate(page).is_some(),
                "ACPI data at {start:?} lies outside the direct map"
            );
            page = page + PAGE_SIZE;
        }
        // SAFETY: every page of the range is mapped, read-write, in the
        // direct map, and firmware does not change ACPI tables.
        unsafe { core::slice::from_raw_parts(virt.as_ptr::<u8>(), size) }
    }
}

ktest::ktest! {
    fn qemu_madt_has_one_cpu_and_one_io_apic() {
        let madt = madt();
        assert_eq!(madt.local_apic, PhysAddr::new(0xfee0_0000));
        assert!(madt.has_8259);
        assert_eq!(madt.cpus, [Cpu { acpi_id: 0, apic_id: 0 }]);
        assert_eq!(madt.io_apics.len(), 1);
        assert_eq!(madt.io_apics[0].address, PhysAddr::new(0xfec0_0000));
        assert_eq!(madt.io_apics[0].gsi_base, 0);
    }
}

ktest::ktest! {
    fn qemu_madt_routes_the_pit_to_gsi_2() {
        let pit = madt().isa_overrides.iter().find(|o| o.irq == 0);
        assert_eq!(pit.map(|o| o.gsi), Some(2));
    }
}

ktest::ktest! {
    fn qemu_madt_wires_lint1_to_nmi() {
        let nmi = LocalApicNmi { acpi_id: None, lint: 1, flags: 0 };
        assert_eq!(madt().local_apic_nmis, [nmi]);
    }
}
