//! Kernel-side ACPI integration.
//!
//! Provides the [`AcpiHandler`] implementation using HHDM address translation,
//! and stores parsed ACPI information (MADT, HPET, MCFG, FADT, SRAT, SLIT,
//! DMAR, IVRS, BGRT, DSDT/SSDT) for use by kernel subsystems.

use core::sync::atomic::{AtomicU8, AtomicU32, AtomicU64, Ordering};

use hadron_acpi::aml::namespace::NamespaceBuilder;
use hadron_acpi::aml::{self, AmlValue, Namespace};
use hadron_acpi::{AcpiHandler, AcpiTables, SdtHeader, madt};

use crate::addr::{PhysAddr, VirtAddr};
use crate::arch::x86_64::hw::hpet::Hpet;
use crate::arch::x86_64::hw::io_apic::{
    DeliveryMode, DestinationMode, IoApic, Polarity, RedirectionEntry, TriggerMode,
};
use crate::arch::x86_64::hw::local_apic::LocalApic;
use crate::id::IrqVector;
use crate::mm::hhdm;
use crate::sync::IrqSpinLock;

use crate::arch::x86_64::interrupts::dispatch::vectors;

/// HHDM-based ACPI handler — translates physical addresses via the HHDM offset.
struct HhdmAcpiHandler;

// SAFETY: HHDM is initialized before ACPI parsing, so `phys_to_virt` is valid.
// The HHDM maps all physical memory and the mapping is permanent ('static).
unsafe impl AcpiHandler for HhdmAcpiHandler {
    unsafe fn map_physical_region(&self, phys: u64, size: usize) -> &'static [u8] {
        // SAFETY: The HHDM provides a permanent, identity-offset mapping of all
        // physical memory. The caller guarantees `phys` and `size` describe a
        // valid ACPI table region within physical memory.
        unsafe {
            let ptr = hhdm::phys_to_virt(PhysAddr::new(phys)).as_ptr::<u8>();
            core::slice::from_raw_parts(ptr, size)
        }
    }
}

/// Consolidated APIC platform state, initialized once during ACPI init.
struct AcpiPlatformState {
    /// LAPIC virtual base address.
    lapic_base: VirtAddr,
    /// I/O APIC virtual base address.
    io_apic_base: VirtAddr,
    /// I/O APIC GSI base.
    gsi_base: u32,
}

/// APIC platform state: `None` before init, `Some` after.
///
/// Now only used for I/O APIC operations (`with_io_apic`). The LAPIC base
/// address is cached in `LAPIC_BASE` for lock-free access on hot paths.
static PLATFORM: IrqSpinLock<Option<AcpiPlatformState>> =
    IrqSpinLock::leveled("PLATFORM", 11, None);

/// LAPIC timer initial count (ticks per interval), stored after BSP calibration
/// so APs can start their timers with the same configuration.
static LAPIC_TIMER_INITIAL_COUNT: AtomicU32 = AtomicU32::new(0);

/// LAPIC timer divide value, stored after BSP calibration for AP reuse.
static LAPIC_TIMER_DIVIDE: AtomicU8 = AtomicU8::new(0);

/// Cached LAPIC virtual base address, set once during ACPI init.
///
/// All CPUs share the same virtual address for LAPIC MMIO; the hardware
/// routes each access to the requesting CPU's local APIC. This atomic
/// allows lock-free access to the LAPIC for EOI, IPI, and other hot-path
/// operations without acquiring the PLATFORM lock.
static LAPIC_BASE: AtomicU64 = AtomicU64::new(0);

/// ACPI namespace, stored after AML parsing for use by platform device discovery.
///
/// Level 12 avoids nesting with PLATFORM (level 11) since `with_namespace` is
/// only called during boot/driver probe and never while holding PLATFORM.
static ACPI_NAMESPACE: IrqSpinLock<Option<Namespace>> = IrqSpinLock::leveled("ACPI_NS", 12, None);

/// ECAM (Enhanced Configuration Access Mechanism) base info from MCFG.
static ECAM_INFO: IrqSpinLock<Option<EcamInfo>> = IrqSpinLock::leveled("ECAM", 12, None);

/// ECAM region info for PCI Express memory-mapped config access.
#[derive(Clone, Copy)]
pub struct EcamInfo {
    /// Physical base address of the ECAM region.
    pub phys_base: u64,
    /// PCI segment group number.
    pub segment: u16,
    /// First PCI bus number covered.
    pub start_bus: u8,
    /// Last PCI bus number covered.
    pub end_bus: u8,
}

/// Runs a closure with access to the ACPI namespace, if available.
pub fn with_namespace<R>(f: impl FnOnce(&Namespace) -> R) -> Option<R> {
    let lock = ACPI_NAMESPACE.lock();
    let ns = lock.as_ref()?;
    Some(f(ns))
}

/// Runs a closure with access to the ECAM info, if available.
pub fn with_ecam<R>(f: impl FnOnce(&EcamInfo) -> R) -> Option<R> {
    let lock = ECAM_INFO.lock();
    let info = lock.as_ref()?;
    Some(f(info))
}

/// Sends LAPIC EOI if the LAPIC has been initialized.
///
/// Called by the interrupt dispatch subsystem after every hardware interrupt.
/// Reads the cached `LAPIC_BASE` atomic — no lock, EOI is never dropped.
pub fn send_lapic_eoi() {
    let base = LAPIC_BASE.load(Ordering::Acquire);
    if base != 0 {
        // SAFETY: The LAPIC was mapped during init and the mapping is permanent.
        let lapic = unsafe { LocalApic::new(VirtAddr::new(base)) };
        lapic.eoi();
    }
}

/// Returns the LAPIC timer configuration (initial_count, divide) from BSP calibration.
///
/// APs use this to start their periodic timers with the same interval.
/// Returns `(0, 0)` if the timer has not been calibrated yet.
pub fn lapic_timer_config() -> (u32, u8) {
    (
        LAPIC_TIMER_INITIAL_COUNT.load(Ordering::Acquire),
        LAPIC_TIMER_DIVIDE.load(Ordering::Acquire),
    )
}

/// Returns the LAPIC virtual base address, if initialized.
///
/// Reads the cached `LAPIC_BASE` atomic — no lock required.
/// All CPUs share the same virtual address for LAPIC MMIO; the hardware
/// routes each access to the requesting CPU's local APIC.
pub fn lapic_virt() -> Option<VirtAddr> {
    let base = LAPIC_BASE.load(Ordering::Acquire);
    if base != 0 {
        Some(VirtAddr::new(base))
    } else {
        None
    }
}

/// Runs a closure with a reference to the I/O APIC, if initialized.
///
/// Reconstructs the `IoApic` from the stored virtual base address. Drivers use
/// this to unmask their IRQ lines after binding a handler.
pub fn with_io_apic<R>(f: impl FnOnce(&IoApic) -> R) -> Option<R> {
    let lock = PLATFORM.lock();
    let state = lock.as_ref()?;
    // SAFETY: The I/O APIC was mapped during init and the mapping is permanent.
    let ioapic = unsafe { IoApic::new(state.io_apic_base, state.gsi_base) };
    Some(f(&ioapic))
}

/// Combined timer tick + LAPIC EOI for the custom timer preemption stub.
///
/// Called from both ring-0 and ring-3 paths of the naked timer stub.
/// Performs the timer tick logic (increment counter, wake sleepers, set
/// preempt flag) and sends LAPIC EOI.
pub(crate) extern "C" fn timer_tick_and_eoi() {
    timer_handler(vectors::TIMER.as_irq_vector());
    send_lapic_eoi();
}

/// LAPIC timer interrupt handler.
fn timer_handler(_vector: IrqVector) {
    // Wake tasks whose sleep deadline has expired.
    crate::sched::timer::wake_expired(crate::time::timer_ticks());

    // Signal the executor to rotate to the next task.
    crate::sched::set_preempt_pending();
}

/// Initialize ACPI tables and all interrupt controllers.
///
/// This is the main Phase 5 init function, called from `kernel_setup` after
/// the heap is ready. It:
/// 1. Parses ACPI tables (RSDP -> MADT, HPET, MCFG, FADT, SRAT, SLIT, DMAR, IVRS, BGRT, DSDT/SSDT)
/// 2. Disables legacy PIC
/// 3. Maps and enables Local APIC (BSP)
/// 4. Maps and configures I/O APIC
/// 5. Initializes HPET (if available)
/// 6. Calibrates and starts LAPIC timer
/// 7. Enables interrupts
pub fn init(rsdp_phys: Option<PhysAddr>) {
    let rsdp_phys = match rsdp_phys {
        Some(addr) => addr,
        None => {
            crate::kwarn!("ACPI: No RSDP address available, skipping ACPI init");
            return;
        }
    };

    // --- 1. Parse ACPI tables ---
    let tables = match AcpiTables::new(rsdp_phys.as_u64(), HhdmAcpiHandler) {
        Ok(t) => {
            crate::kinfo!(
                "ACPI: RSDP validated, {} at {:#x}",
                if t.is_xsdt() { "XSDT" } else { "RSDT" },
                t.rsdt_addr()
            );
            t
        }
        Err(e) => {
            crate::kerr!("ACPI: Failed to parse RSDP: {:?}", e);
            return;
        }
    };

    // Parse MADT
    let madt_info = match tables.madt() {
        Ok(m) => {
            let mut cpu_count = 0u32;
            let mut io_apic_count = 0u32;
            for entry in m.entries() {
                match entry {
                    madt::MadtEntry::LocalApic { flags, .. } => {
                        if flags & 1 != 0 {
                            cpu_count += 1;
                        }
                    }
                    madt::MadtEntry::IoApic { .. } => io_apic_count += 1,
                    _ => {}
                }
            }
            crate::kinfo!(
                "ACPI: MADT: {} CPUs, {} I/O APICs, LAPIC at {:#x}",
                cpu_count,
                io_apic_count,
                m.local_apic_address
            );
            Some(m)
        }
        Err(e) => {
            crate::kwarn!("ACPI: MADT not found: {:?}", e);
            None
        }
    };

    // Parse HPET
    let hpet_info = match tables.hpet() {
        Ok(h) => {
            let hpet_addr = h.base_address.address;
            let min_tick = h.minimum_tick;
            crate::kdebug!("ACPI: HPET at {:#x}, minimum tick {}", hpet_addr, min_tick);
            Some(h)
        }
        Err(e) => {
            crate::kwarn!("ACPI: HPET not available: {:?}", e);
            None
        }
    };

    // Parse MCFG and store ECAM info for PCI Express config access.
    match tables.mcfg() {
        Ok(m) => {
            crate::kdebug!("ACPI: MCFG with {} entries", m.entry_count());
            if let Some(entry) = m.entries().next() {
                let info = EcamInfo {
                    phys_base: entry.base_address,
                    segment: entry.segment_group,
                    start_bus: entry.start_bus,
                    end_bus: entry.end_bus,
                };
                *ECAM_INFO.lock() = Some(info);
                crate::kinfo!(
                    "ACPI: ECAM at {:#x}, segment {}, buses {}-{}",
                    info.phys_base, info.segment, info.start_bus, info.end_bus
                );
            }
        }
        Err(_) => {
            crate::kdebug!("ACPI: MCFG not found");
        }
    }

    // Parse FADT
    let fadt = match tables.fadt() {
        Ok(f) => {
            crate::kinfo!(
                "ACPI: FADT: PM timer port {:#x}, boot arch flags {:#x}",
                f.pm_timer_block,
                f.boot_architecture_flags
            );
            if let Some(dsdt) = f.dsdt_address() {
                crate::kdebug!("ACPI: FADT: DSDT at {:#x}", dsdt);
            }
            if let Some(facs) = f.facs_address() {
                crate::kdebug!("ACPI: FADT: FACS at {:#x}", facs);
            }
            Some(f)
        }
        Err(_) => {
            crate::kdebug!("ACPI: FADT not present");
            None
        }
    };

    // Parse SRAT (NUMA topology)
    match tables.srat() {
        Ok(srat) => {
            let mut domains = 0u32;
            let mut mem_regions = 0u32;
            let mut cpu_count = 0u32;
            let mut max_domain = 0u32;

            for entry in srat.entries() {
                match entry {
                    hadron_acpi::SratEntry::ProcessorLocalApicAffinity {
                        flags,
                        proximity_domain_lo,
                        proximity_domain_hi,
                        ..
                    } => {
                        if flags & 1 != 0 {
                            cpu_count += 1;
                            let domain = u32::from(proximity_domain_lo)
                                | (u32::from(proximity_domain_hi[0]) << 8)
                                | (u32::from(proximity_domain_hi[1]) << 16)
                                | (u32::from(proximity_domain_hi[2]) << 24);
                            if domain > max_domain {
                                max_domain = domain;
                            }
                        }
                    }
                    hadron_acpi::SratEntry::MemoryAffinity {
                        proximity_domain,
                        base_address,
                        length,
                        flags,
                    } => {
                        if flags & 1 != 0 {
                            mem_regions += 1;
                            crate::kdebug!(
                                "ACPI: SRAT: memory domain {} base {:#x} length {:#x}",
                                proximity_domain,
                                base_address,
                                length
                            );
                            if proximity_domain > max_domain {
                                max_domain = proximity_domain;
                            }
                        }
                    }
                    hadron_acpi::SratEntry::X2ApicAffinity {
                        flags,
                        proximity_domain,
                        ..
                    } => {
                        if flags & 1 != 0 {
                            cpu_count += 1;
                            if proximity_domain > max_domain {
                                max_domain = proximity_domain;
                            }
                        }
                    }
                    hadron_acpi::SratEntry::Unknown { .. } => {}
                }
            }

            if cpu_count > 0 || mem_regions > 0 {
                domains = max_domain + 1;
            }
            crate::kinfo!(
                "ACPI: SRAT: {} proximity domains, {} CPUs, {} memory regions",
                domains,
                cpu_count,
                mem_regions
            );
        }
        Err(_) => {
            crate::kdebug!("ACPI: SRAT not present");
        }
    }

    // Parse SLIT (NUMA distances)
    match tables.slit() {
        Ok(slit) => {
            let n = slit.num_localities();
            crate::kinfo!("ACPI: SLIT: {} localities", n);
            if n <= 8 {
                for from in 0..n {
                    for to in 0..n {
                        if let Some(d) = slit.distance(from, to) {
                            crate::kdebug!("ACPI: SLIT: [{} -> {}] = {}", from, to, d);
                        }
                    }
                }
            }
        }
        Err(_) => {
            crate::kdebug!("ACPI: SLIT not present");
        }
    }

    // Parse DMAR (Intel VT-d)
    match tables.dmar() {
        Ok(dmar) => {
            let mut drhd_count = 0u32;
            let mut rmrr_count = 0u32;
            crate::kdebug!(
                "ACPI: DMAR: host address width {}, flags {:#x}",
                dmar.host_address_width,
                dmar.flags
            );
            for entry in dmar.entries() {
                match entry {
                    hadron_acpi::DmarEntry::Drhd {
                        flags,
                        segment,
                        register_base_address,
                        ..
                    } => {
                        drhd_count += 1;
                        crate::kdebug!(
                            "ACPI: DMAR: DRHD segment {} base {:#x} flags {:#x}",
                            segment,
                            register_base_address,
                            flags
                        );
                    }
                    hadron_acpi::DmarEntry::Rmrr {
                        segment,
                        base_address,
                        limit_address,
                        ..
                    } => {
                        rmrr_count += 1;
                        crate::kdebug!(
                            "ACPI: DMAR: RMRR segment {} range {:#x}-{:#x}",
                            segment,
                            base_address,
                            limit_address
                        );
                    }
                    hadron_acpi::DmarEntry::Atsr { flags, segment, .. } => {
                        crate::kdebug!("ACPI: DMAR: ATSR segment {} flags {:#x}", segment, flags);
                    }
                    hadron_acpi::DmarEntry::Unknown { .. } => {}
                }
            }
            crate::kinfo!("ACPI: DMAR: {} DRHDs, {} RMRRs", drhd_count, rmrr_count);
        }
        Err(_) => {
            crate::kdebug!("ACPI: DMAR not present");
        }
    }

    // Parse IVRS (AMD-Vi)
    match tables.ivrs() {
        Ok(ivrs) => {
            let mut ivhd_count = 0u32;
            let mut ivmd_count = 0u32;
            crate::kdebug!("ACPI: IVRS: iv_info {:#x}", ivrs.iv_info);
            for entry in ivrs.entries() {
                match entry {
                    hadron_acpi::IvrsEntry::Ivhd {
                        ivhd_type,
                        iommu_base_address,
                        segment_group,
                        device_id,
                        ..
                    } => {
                        ivhd_count += 1;
                        crate::kdebug!(
                            "ACPI: IVRS: IVHD type {:#x} IOMMU base {:#x} segment {} device {:#x}",
                            ivhd_type,
                            iommu_base_address,
                            segment_group,
                            device_id
                        );
                    }
                    hadron_acpi::IvrsEntry::Ivmd {
                        ivmd_type,
                        start_address,
                        memory_block_length,
                        ..
                    } => {
                        ivmd_count += 1;
                        crate::kdebug!(
                            "ACPI: IVRS: IVMD type {:#x} start {:#x} length {:#x}",
                            ivmd_type,
                            start_address,
                            memory_block_length
                        );
                    }
                    hadron_acpi::IvrsEntry::Unknown { .. } => {}
                }
            }
            crate::kinfo!("ACPI: IVRS: {} IVHDs, {} IVMDs", ivhd_count, ivmd_count);
        }
        Err(_) => {
            crate::kdebug!("ACPI: IVRS not present");
        }
    }

    // Parse BGRT
    match tables.bgrt() {
        Ok(bgrt) => {
            crate::kinfo!(
                "ACPI: BGRT: image at {:#x} type {} offset ({}, {})",
                bgrt.image_address,
                bgrt.image_type,
                bgrt.image_offset_x,
                bgrt.image_offset_y
            );
        }
        Err(_) => {
            crate::kdebug!("ACPI: BGRT not present");
        }
    }

    // Walk DSDT/SSDT AML namespace and persist for platform device discovery.
    if fadt.is_some() {
        if let Some(ns) = parse_aml_namespace(&tables) {
            *ACPI_NAMESPACE.lock() = Some(ns);
        }
    }

    // --- 2. Disable legacy PIC ---
    // SAFETY: Interrupts are disabled at this point (CLI from boot).
    unsafe { crate::arch::x86_64::hw::pic::remap_and_disable() };
    crate::kdebug!("PIC: Remapped to vectors 32-47, masked all");

    // --- 3. Map and enable Local APIC ---
    let madt = match madt_info {
        Some(m) => m,
        None => {
            crate::kerr!("ACPI: Cannot initialize APIC without MADT");
            return;
        }
    };

    let lapic_phys = PhysAddr::new(u64::from(madt.local_apic_address));

    // Map LAPIC MMIO region (permanent hardware mapping).
    let mapping = crate::mm::vmm::map_mmio_region(lapic_phys, crate::mm::PAGE_SIZE as u64);
    let lapic_virt = mapping.virt_base();
    core::mem::forget(mapping); // permanent hardware mapping

    // SAFETY: lapic_virt was just mapped to the LAPIC MMIO region.
    let lapic = unsafe { LocalApic::new(lapic_virt) };
    lapic.enable(vectors::SPURIOUS.as_irq_vector());
    lapic.set_tpr(0); // Accept all interrupts

    // Initialize per-CPU state
    let apic_id = lapic.id();
    crate::percpu::current_cpu().init(crate::id::CpuId::new(0), apic_id);
    crate::sched::smp::register_cpu_apic_id(crate::id::CpuId::new(0), apic_id);

    crate::kinfo!(
        "LAPIC: Enabled, ID={}, spurious vector={}",
        apic_id,
        vectors::SPURIOUS
    );

    // --- 4. Map and configure I/O APIC ---
    let mut io_apic_virt = VirtAddr::new(0);
    let mut io_apic_gsi_base = 0u32;

    for entry in madt.entries() {
        if let madt::MadtEntry::IoApic {
            io_apic_address,
            gsi_base,
            ..
        } = entry
        {
            let ioapic_phys = PhysAddr::new(u64::from(io_apic_address));

            // Map I/O APIC MMIO region (permanent hardware mapping).
            let mapping = crate::mm::vmm::map_mmio_region(ioapic_phys, crate::mm::PAGE_SIZE as u64);
            let ioapic_virt = mapping.virt_base();
            core::mem::forget(mapping); // permanent hardware mapping

            // SAFETY: ioapic_virt was just mapped to the I/O APIC MMIO region.
            let ioapic = unsafe { IoApic::new(ioapic_virt, gsi_base) };
            let max_entry = ioapic.max_redirection_entry();

            crate::kdebug!(
                "I/O APIC: ID={}, GSI base={}, {} entries",
                ioapic.id(),
                gsi_base,
                max_entry + 1
            );

            // Mask all entries by default
            for i in 0..=max_entry {
                ioapic.mask(i);
            }

            // Route ISA IRQs (0-15) to BSP with identity mapping (GSI = IRQ + 32)
            // but check for interrupt source overrides from MADT first.
            if gsi_base == 0 {
                setup_isa_irqs(&ioapic, &madt, apic_id);
            }

            // Remember the last I/O APIC for the consolidated state.
            io_apic_virt = ioapic_virt;
            io_apic_gsi_base = gsi_base;
        }
    }

    // Persist the platform state for later use by I/O APIC operations.
    *PLATFORM.lock() = Some(AcpiPlatformState {
        lapic_base: lapic_virt,
        io_apic_base: io_apic_virt,
        gsi_base: io_apic_gsi_base,
    });

    // Cache LAPIC base address for lock-free access by EOI, IPI, etc.
    LAPIC_BASE.store(lapic_virt.as_u64(), Ordering::Release);

    // --- 5. Initialize HPET ---
    let hpet = hpet_info.and_then(|info| {
        let hpet_phys = PhysAddr::new(info.base_address.address);
        // Map HPET MMIO region (permanent hardware mapping).
        let mapping = crate::mm::vmm::map_mmio_region(hpet_phys, crate::mm::PAGE_SIZE as u64);
        let hpet_virt = mapping.virt_base();
        core::mem::forget(mapping); // permanent hardware mapping

        let hpet = unsafe { Hpet::new(hpet_virt) };
        hpet.enable();

        // Initialize global time source from HPET — timestamps become real after this.
        crate::time::init_hpet(hpet_virt, hpet.period_fs());

        crate::kinfo!(
            "HPET: Enabled, {} Hz, {} comparators",
            hpet.frequency_hz(),
            hpet.num_comparators()
        );
        Some(hpet)
    });

    // --- 6. Calibrate and start LAPIC timer ---
    calibrate_and_start_timer(&lapic, hpet.as_ref());

    // --- 6b. Register HPET as global ClockSource ---
    if let Some(hpet) = hpet {
        crate::time::register_hpet(hpet);
    }

    // Note: Interrupts are NOT enabled here. The caller (kernel_init) enables
    // them after AP bootstrap completes, right before entering the executor.
    // Starting the LAPIC timer with interrupts disabled is fine — interrupts
    // are simply held pending until STI.
}

/// Sets up ISA IRQ routing through the I/O APIC, respecting MADT overrides.
fn setup_isa_irqs(ioapic: &IoApic, madt_data: &hadron_acpi::madt::Madt, bsp_apic_id: u8) {
    // Build override table for ISA IRQs 0-15.
    for irq in 0u8..16 {
        let mut gsi = u32::from(irq);
        let mut polarity = Polarity::ActiveHigh;
        let mut trigger = TriggerMode::Edge;

        /// MADT Interrupt Source Override flag bit masks.
        const ISO_POLARITY_MASK: u16 = 0x03;
        const ISO_TRIGGER_SHIFT: u16 = 2;
        const ISO_TRIGGER_MASK: u16 = 0x03;
        const ISO_ACTIVE_HIGH: u16 = 0b10;
        const ISO_ACTIVE_LOW: u16 = 0b11;
        const ISO_EDGE_TRIGGERED: u16 = 0b10;
        const ISO_LEVEL_TRIGGERED: u16 = 0b11;

        // Check for interrupt source overrides.
        for entry in madt_data.entries() {
            if let madt::MadtEntry::InterruptSourceOverride {
                source,
                gsi: override_gsi,
                flags: iso_flags,
                ..
            } = entry
            {
                if source == irq {
                    gsi = override_gsi;
                    // Bits 0-1: polarity
                    match iso_flags & ISO_POLARITY_MASK {
                        ISO_ACTIVE_HIGH => polarity = Polarity::ActiveHigh,
                        ISO_ACTIVE_LOW => polarity = Polarity::ActiveLow,
                        _ => {} // Conforming or reserved — keep default
                    }
                    // Bits 2-3: trigger mode
                    match (iso_flags >> ISO_TRIGGER_SHIFT) & ISO_TRIGGER_MASK {
                        ISO_EDGE_TRIGGERED => trigger = TriggerMode::Edge,
                        ISO_LEVEL_TRIGGERED => trigger = TriggerMode::Level,
                        _ => {} // Conforming or reserved — keep default
                    }
                    break;
                }
            }
        }

        let vector = IrqVector::new(32 + gsi as u8);
        let entry = RedirectionEntry {
            vector,
            delivery_mode: DeliveryMode::Fixed,
            destination_mode: DestinationMode::Physical,
            polarity,
            trigger_mode: trigger,
            masked: true, // Leave masked — drivers unmask as needed
            destination: bsp_apic_id,
        };

        // Only set entries within this I/O APIC's range.
        if gsi < u32::from(ioapic.max_redirection_entry()) + 1 {
            #[expect(
                clippy::cast_possible_truncation,
                reason = "GSI fits in u8 for ISA range"
            )]
            ioapic.set_entry(gsi as u8, entry);
        }
    }
}

/// Calibrates the LAPIC timer and starts it in periodic mode.
fn calibrate_and_start_timer(lapic: &LocalApic, hpet: Option<&Hpet>) {
    // Register the timer handler.
    crate::arch::x86_64::interrupts::dispatch::register_handler(vectors::TIMER, timer_handler)
        .expect("Failed to register timer handler");

    // Calibration: measure how many LAPIC timer ticks occur in 10ms.
    let divide = 16u8;
    lapic.start_timer_oneshot(vectors::TIMER.as_irq_vector(), u32::MAX, divide);

    // Wait 10ms using HPET or PIT.
    if let Some(hpet) = hpet {
        hpet.busy_wait_ms(10);
    } else {
        // SAFETY: PIT is available, interrupts are disabled.
        unsafe { crate::arch::x86_64::hw::pit::busy_wait_ms(10) };
    }

    let elapsed = u32::MAX - lapic.timer_current_count();
    lapic.stop_timer();

    // Calculate ticks per second: elapsed in 10ms, so * 100.
    let ticks_per_second = u64::from(elapsed) * 100;
    let ticks_per_ms = ticks_per_second / 1000;

    crate::kinfo!(
        "Timer: LAPIC calibrated at {} MHz ({} ticks/ms, divide={})",
        ticks_per_second / 1_000_000,
        ticks_per_ms,
        divide
    );

    // Start periodic timer at ~1000 Hz (1ms interval).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "calibrated tick count fits in u32"
    )]
    let initial_count = ticks_per_ms as u32;
    if initial_count > 0 {
        // Store calibration for AP reuse before starting timer.
        LAPIC_TIMER_INITIAL_COUNT.store(initial_count, Ordering::Release);
        LAPIC_TIMER_DIVIDE.store(divide, Ordering::Release);

        lapic.start_timer_periodic(vectors::TIMER.as_irq_vector(), initial_count, divide);
        crate::kinfo!("Timer: LAPIC periodic timer started (1ms interval)");
    } else {
        crate::kwarn!("Timer: Calibration returned 0 ticks, timer not started");
    }
}

/// Walks the DSDT and any SSDTs to extract the AML namespace.
///
/// Returns the namespace for persistence. The caller stores it in
/// `ACPI_NAMESPACE` for later use by platform device discovery.
fn parse_aml_namespace(tables: &AcpiTables<HhdmAcpiHandler>) -> Option<Namespace> {
    let dsdt = match tables.dsdt() {
        Ok(d) => d,
        Err(_) => {
            crate::kdebug!("ACPI: DSDT not available");
            return None;
        }
    };

    let mut builder = NamespaceBuilder::new();

    // Walk DSDT AML (skip the SDT header to get raw AML bytecode).
    let aml_data = match dsdt.data.get(SdtHeader::SIZE..) {
        Some(d) if !d.is_empty() => d,
        _ => {
            crate::kdebug!("ACPI: DSDT has no AML data");
            return None;
        }
    };

    if let Err(e) = aml::walk_aml(aml_data, &mut builder) {
        crate::kdebug!("ACPI: DSDT AML walk error: {:?}", e);
    }

    // Walk any SSDTs and merge into the same namespace.
    let mut ssdt_count = 0u32;
    for ssdt_phys in tables.ssdts() {
        match hadron_acpi::sdt::load_table(tables.handler(), ssdt_phys, b"SSDT") {
            Ok(ssdt) => {
                if let Some(aml) = ssdt.data.get(SdtHeader::SIZE..) {
                    if let Err(e) = aml::walk_aml(aml, &mut builder) {
                        crate::kdebug!("ACPI: SSDT AML walk error: {:?}", e);
                    }
                    ssdt_count += 1;
                }
            }
            Err(e) => {
                crate::kdebug!("ACPI: Failed to load SSDT at {:#x}: {:?}", ssdt_phys, e);
            }
        }
    }

    let ns = builder.build();
    let device_count = ns.devices().count();

    if ssdt_count > 0 {
        crate::kinfo!(
            "ACPI: AML namespace: {} devices (DSDT + {} SSDTs)",
            device_count,
            ssdt_count
        );
    } else {
        crate::kinfo!("ACPI: AML namespace: {} devices (DSDT)", device_count);
    }

    for dev in ns.devices() {
        let resource_count = dev.resources.len();
        let prt_count = dev.prt.len();
        match &dev.hid {
            Some(AmlValue::EisaId(id)) => {
                let decoded = id.decode();
                let hid_str = core::str::from_utf8(&decoded).unwrap_or("?");
                if resource_count > 0 {
                    crate::kdebug!("ACPI: AML: {} _HID={} ({} resources)", dev.path, hid_str, resource_count);
                } else {
                    crate::kdebug!("ACPI: AML: {} _HID={}", dev.path, hid_str);
                }
            }
            Some(AmlValue::String(s)) => {
                if resource_count > 0 {
                    crate::kdebug!("ACPI: AML: {} _HID=\"{}\" ({} resources)", dev.path, s.as_str(), resource_count);
                } else {
                    crate::kdebug!("ACPI: AML: {} _HID=\"{}\"", dev.path, s.as_str());
                }
            }
            Some(AmlValue::Integer(v)) => {
                crate::kdebug!("ACPI: AML: {} _HID={:#x}", dev.path, v);
            }
            _ => {}
        }
        if prt_count > 0 {
            crate::kdebug!("ACPI: AML: {} has {} _PRT entries", dev.path, prt_count);
        }
    }

    Some(ns)
}
