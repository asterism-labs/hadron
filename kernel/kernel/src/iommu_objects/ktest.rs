//! Kernel tests for IOMMU objects (Bti, Pmt, domain allocation).

use hadron_ktest::kernel_test;

/// Verifies that VT-d IOMMU units were initialized from ACPI DMAR.
#[kernel_test(stage = "before_executor")]
fn test_iommu_vtd_initialized() {
    // With intel-iommu device in QEMU, we expect at least one DRHD.
    let count = hadron_iommu::unit_count();
    assert!(count > 0, "expected at least 1 VT-d unit, got {count}");
}

/// Verifies DMAR info was parsed and stored in the ACPI subsystem.
#[kernel_test(stage = "before_executor")]
fn test_dmar_info_stored() {
    use crate::arch::x86_64::acpi::Acpi;
    let has_dmar = Acpi::with_dmar(|dmar| {
        assert!(!dmar.drhds.is_empty(), "DMAR has no DRHD entries");
        dmar.drhds.len()
    });
    assert!(has_dmar.is_some(), "DMAR info not stored");
}

/// Verifies domain allocation and freeing works.
#[kernel_test(stage = "before_executor")]
fn test_iommu_domain_alloc_free() {
    use hadron_iommu::domain::DomainAllocator;
    let mut alloc = DomainAllocator::new(64);
    let d1 = alloc.alloc().expect("failed to allocate domain");
    assert_eq!(d1.as_u16(), 1); // Domain 0 is reserved
    let d2 = alloc.alloc().expect("failed to allocate domain");
    assert_eq!(d2.as_u16(), 2);
    alloc.free(d1).expect("failed to free domain");
    let d3 = alloc.alloc().expect("failed to allocate domain");
    assert_eq!(d3.as_u16(), 1); // Reused
    alloc.free(d2).expect("failed to free domain");
    alloc.free(d3).expect("failed to free domain");
}

/// Verifies Bti creation allocates a domain and attaches a device.
#[kernel_test(stage = "before_executor")]
fn test_iommu_bti_create() {
    use hadron_iommu::hw::PciBdf;
    let bdf = PciBdf {
        bus: 0,
        device: 1,
        function: 0,
    };
    let bti = super::bti::Bti::new(0, bdf).expect("Bti::new failed");
    // Bti should have a valid domain ID (non-zero, since 0 is reserved).
    assert!(bti.domain_id().as_u16() > 0, "expected non-zero domain ID");
    // Cleanup happens on drop via on_zero_handles.
    drop(bti);
}

/// Verifies pin/unpin round-trip: pin pages, verify IOVAs, then unpin.
#[kernel_test(stage = "before_executor")]
fn test_iommu_bti_pin_unpin() {
    use hadron_iommu::hw::{DmaPermission, PciBdf};
    let bdf = PciBdf {
        bus: 0,
        device: 2,
        function: 0,
    };
    let bti = super::bti::Bti::new(0, bdf).expect("Bti::new failed");

    // Pin 2 pages.
    let pmt = bti
        .pin(2, DmaPermission::READ_WRITE)
        .expect("Bti::pin failed");

    // Verify physical addresses are present.
    let addrs = pmt.phys_addrs();
    assert_eq!(addrs.len(), 2, "expected 2 physical addresses");
    assert!(addrs[0] > 0, "expected non-zero physical address");
    assert!(addrs[1] > 0, "expected non-zero physical address");

    // IOVA base should be 0x1000 (first allocation).
    assert_eq!(pmt.iova_base(), 0x1000, "expected IOVA base at 0x1000");

    // Unpin should succeed.
    pmt.unpin().expect("Pmt::unpin failed");

    // Physical addresses should be empty after unpin.
    let addrs_after = pmt.phys_addrs();
    assert!(addrs_after.is_empty(), "expected empty addrs after unpin");

    drop(bti);
}

/// Verifies that dropping a Pmt auto-unpins (safety net).
#[kernel_test(stage = "before_executor")]
fn test_iommu_pmt_drop_auto_unpins() {
    use hadron_iommu::hw::{DmaPermission, PciBdf};
    let bdf = PciBdf {
        bus: 0,
        device: 3,
        function: 0,
    };
    let bti = super::bti::Bti::new(0, bdf).expect("Bti::new failed");

    // Pin 1 page and immediately drop — should not panic.
    let pmt = bti.pin(1, DmaPermission::READ).expect("Bti::pin failed");
    drop(pmt);
    // If we get here, auto-unpin in Drop worked.

    drop(bti);
}

/// Verifies that dropping a Bti frees its domain.
#[kernel_test(stage = "before_executor")]
fn test_iommu_bti_drop_frees_domain() {
    use hadron_iommu::hw::PciBdf;
    let bdf = PciBdf {
        bus: 0,
        device: 4,
        function: 0,
    };
    let bti = super::bti::Bti::new(0, bdf).expect("Bti::new failed");
    let domain_id = bti.domain_id();

    // Drop the Bti — on_zero_handles should free the domain.
    use hadron_objects::object::KernelObject;
    bti.on_zero_handles();
    drop(bti);

    // Allocating a new domain should reuse the freed ID.
    let bti2 = super::bti::Bti::new(
        0,
        PciBdf {
            bus: 0,
            device: 5,
            function: 0,
        },
    )
    .expect("Bti::new failed for second BTI");

    // The freed domain ID should have been reused.
    assert_eq!(
        bti2.domain_id().as_u16(),
        domain_id.as_u16(),
        "expected domain ID to be reused"
    );

    bti2.on_zero_handles();
    drop(bti2);
}
