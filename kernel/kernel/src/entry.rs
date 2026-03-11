//! Kernel entry point called by the UEFI boot stub.
//!
//! Executes the full BSP boot sequence: GDT → IDT → TLB registration →
//! PMM → VMM → heap → per-CPU state → SYSCALL init → userboot launch.

use hadron_core::addr::VirtAddr;
use hadron_sched::executor::ArchHalt;

use crate::boot::BootInfo;

/// Kernel entry point.
///
/// The UEFI boot stub jumps here after setting up page tables, loading
/// the kernel ELF, and building the `BootInfo` struct.
///
/// # Safety
///
/// `boot_info` must point to a valid, fully initialized `BootInfo` struct
/// in memory accessible under the kernel's page tables.
#[unsafe(no_mangle)]
pub extern "C" fn kernel_init(boot_info: *const BootInfo) -> ! {
    // SAFETY: boot_info was set up by the UEFI stub and is in HHDM-mapped memory.
    let bi = unsafe { &*boot_info };

    // ── 0. Record boot TSC ──────────────────────────────────────────────
    crate::time::record_boot_tsc();

    // ── 1. Initialize HHDM offset ──────────────────────────────────────
    let hhdm_offset = VirtAddr::new_truncate(bi.hhdm_offset);
    hadron_mm::hhdm::init(hhdm_offset);
    crate::kinfo!("boot", "HHDM initialized at {:#x}", bi.hhdm_offset);

    // ── 2. Boot mapper ─────────────────────────────────────────────────
    // The boot stub maps the first 4 GiB into the HHDM. The boot mapper
    // callback extends the HHDM beyond 4 GiB, but after GDT/IDT init the
    // callback into stub code is no longer safe (the stub was compiled for
    // the UEFI target with different segment assumptions). For systems with
    // <=4 GiB RAM the initial mapping suffices.
    // TODO: Support >4 GiB by extending HHDM from the kernel's own VMM.
    crate::kdebug!("boot", "boot mapper skipped (4 GiB HHDM sufficient)");

    // ── 3. GDT init ────────────────────────────────────────────────────
    // SAFETY: Called exactly once during BSP init. No interrupts yet.
    unsafe { crate::arch::x86_64::gdt::init() };

    // ── 4. IDT init ────────────────────────────────────────────────────
    // SAFETY: Called after GDT init, CS is valid.
    unsafe { crate::arch::x86_64::idt::init() };

    // ── 4b. Calibrate TSC frequency ────────────────────────────────────
    // Uses PIT channel 2 (I/O ports only, available after GDT/IDT).
    // SAFETY: Interrupts are still disabled; PIT is not in use.
    unsafe { crate::time::calibrate_tsc() };

    // ── 5. Register TLB flush callback ─────────────────────────────────
    hadron_mm::mapper::register_tlb_flush(crate::arch::x86_64::instructions::tlb::flush);
    crate::kdebug!("boot", "TLB flush registered");

    // ── 6. Convert UEFI memory map ─────────────────────────────────────
    // SAFETY: bi contains a valid UEFI memory map; HHDM is initialized.
    let mem = unsafe { crate::boot::convert_uefi_memory_map(bi, hhdm_offset) };
    let regions = &mem.regions[..mem.count];
    crate::kinfo!(
        "boot",
        "UEFI memory map: {} regions, max_phys {:#x}",
        mem.count,
        mem.max_phys
    );

    // ── 7. PMM init ────────────────────────────────────────────────────
    let boot_reserved = [
        (bi.kernel_phys, bi.kernel_size),
        (bi.boot_pt_pool_phys, bi.boot_pt_pool_pages * 4096),
    ];
    hadron_mm::pmm::init(regions, hhdm_offset, &boot_reserved);
    crate::kinfo!("boot", "PMM initialized");

    // ── 8. VMM init + heap ─────────────────────────────────────────────
    let root_phys = crate::arch::x86_64::registers::control::Cr3::read();
    let mapper = crate::arch::x86_64::paging::PageTableMapper::new(hhdm_offset);
    let mut vmm = hadron_mm::vmm::Vmm::new(root_phys, mapper, hhdm_offset, mem.max_phys);

    let (heap_base, heap_size) = hadron_mm::pmm::with(|pmm| {
        let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
        vmm.map_initial_heap(&mut alloc)
            .expect("failed to map initial heap")
    });

    // SAFETY: heap_base points to a freshly mapped and zeroed region.
    unsafe { hadron_mm::heap::init_raw(heap_base.as_u64() as usize, heap_size as usize) };
    crate::kinfo!(
        "boot",
        "heap initialized: base {:#x}, size {} KiB",
        heap_base.as_u64(),
        heap_size / 1024
    );

    // ── 9. Store VMM globally ──────────────────────────────────────────
    crate::vmm::init(vmm);

    // ── 10a. Per-CPU phase 1 (PERCPU_BASES[0] = template) ───────────────
    // SAFETY: Called once before init_gs_base, single-threaded BSP init.
    unsafe { crate::percpu::percpu_init_phase1() };

    // ── 10b. Per-CPU init (GS base) ─────────────────────────────────────
    // After this, cpu_is_initialized() returns true and span tracking
    // becomes available for log records.
    // SAFETY: Called once after GDT + phase1, before any code that reads GS-relative data.
    unsafe { crate::percpu::init_gs_base() };
    crate::kinfo!("boot", "BSP per-CPU state initialized");

    // ── 11. Initialize SYSCALL/SYSRET ──────────────────────────────────
    // SAFETY: Called after GDT + per-CPU init, exactly once on BSP.
    unsafe { crate::arch::x86_64::syscall::init() };
    crate::kinfo!("boot", "SYSCALL/SYSRET initialized");

    // ── 11b. ACPI platform init ─────────────────────────────────────────
    #[cfg(hadron_acpi)]
    {
        let rsdp = if bi.rsdp_phys != 0 {
            Some(hadron_core::addr::PhysAddr::new(bi.rsdp_phys))
        } else {
            None
        };
        crate::arch::x86_64::acpi::init(rsdp);
    }

    // ── 11c. IOMMU init ────────────────────────────────────────────────
    #[cfg(hadron_iommu)]
    {
        use crate::arch::x86_64::acpi::Acpi;
        Acpi::with_dmar(|dmar| {
            let drhds: alloc::vec::Vec<hadron_iommu::DrhdEntry> = dmar
                .drhds
                .iter()
                .map(|d| hadron_iommu::DrhdEntry {
                    flags: d.flags,
                    segment: d.segment,
                    register_base_address: d.register_base_address,
                })
                .collect();
            hadron_iommu::init_vtd(dmar.host_address_width, &drhds);
        });
    }

    // ── 11d. Create Iommu kernel objects ──────────────────────────────
    #[cfg(hadron_iommu)]
    {
        let count = hadron_iommu::unit_count();
        for i in 0..count {
            let _iommu_obj = crate::iommu_objects::iommu::Iommu::new(i);
            // Phase 4d: insert into root process handle table for devmgr.
        }
        if count > 0 {
            hadron_log::kinfo!("iommu", "IOMMU: created {} Iommu object(s)", count);
        }
    }

    // ── 11e. Per-CPU phase 2 (allocate AP percpu regions) ───────────────
    #[cfg(hadron_smp)]
    {
        let total_cpus = crate::arch::x86_64::smp::madt_cpu_count();
        crate::percpu::percpu_init_phase2(total_cpus);
    }

    // ── 12. Boot APs (SMP) ──────────────────────────────────────────────
    #[cfg(hadron_smp)]
    {
        // SAFETY: Called once from BSP after ACPI/PMM/heap/per-CPU init.
        unsafe { crate::arch::x86_64::smp::boot_aps() };
    }

    // ── 13. Boot complete ──────────────────────────────────────────────
    crate::kinfo!("boot", "kernel bootstrap complete");

    // ── 14. Load and launch userboot ───────────────────────────────────
    load_and_run_userboot();
}

/// Parses the embedded userboot ELF, maps it into the shared address space,
/// creates Process/Thread objects, spawns a `process_task` on the executor,
/// and starts the executor loop (which never returns).
#[expect(
    clippy::cast_possible_truncation,
    reason = "ELF vaddr fits in usize on x86_64"
)]
fn load_and_run_userboot() -> ! {
    use alloc::string::ToString;
    use alloc::sync::Arc;

    use hadron_core::addr::VirtAddr;
    use hadron_core::paging::{Page, Size4KiB};
    use hadron_mm::mapper::MapFlags;
    use hadron_objects::object::KernelObject;
    use hadron_objects::process::Process;
    use hadron_objects::thread::Thread;
    use hadron_objects::vmar::Vmar;
    /// Page size in bytes.
    const PAGE_SIZE: u64 = 4096;
    /// Page offset mask.
    const PAGE_MASK: u64 = PAGE_SIZE - 1;
    /// ELF segment flag: executable.
    const PF_X: u32 = 1;
    /// ELF segment flag: writable.
    const PF_W: u32 = 2;
    /// User stack top address (well within canonical user range).
    const USER_STACK_TOP: u64 = 0x0000_7FFF_FFFF_0000;
    /// Number of pages for the user stack (64 KiB).
    const USER_STACK_PAGES: u64 = 16;
    /// User address space base and size for VMAR.
    const USER_BASE: u64 = 0x0000_0010_0000_0000;
    const USER_SIZE: u64 = 0x0000_7FEF_0000_0000;

    crate::kinfo!("boot", "loading userboot ELF");

    // ── Parse the ELF ────────────────────────────────────────────────
    let elf =
        hadron_elf::ElfFile::parse(crate::userboot::elf_bytes()).expect("invalid userboot ELF");
    let entry = elf.entry_point();

    // ── Map PT_LOAD segments ─────────────────────────────────────────
    // Phase 2b: still uses the kernel's shared CR3 for userboot.
    // Per-process address spaces are used for spawned children.
    for seg in elf.load_segments() {
        let seg_vaddr = seg.vaddr;
        let seg_memsz = seg.memsz;
        let page_start = seg_vaddr & !PAGE_MASK;
        let page_end = (seg_vaddr + seg_memsz + PAGE_MASK) & !PAGE_MASK;

        let mut flags = MapFlags::USER;
        if seg.flags & PF_W != 0 {
            flags |= MapFlags::WRITABLE;
        }
        if seg.flags & PF_X != 0 {
            flags |= MapFlags::EXECUTABLE;
        }

        let mut page_addr = page_start;
        while page_addr < page_end {
            let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));
            let frame = hadron_mm::pmm::with(|pmm| {
                pmm.allocate_frame()
                    .expect("PMM: out of frames for userboot")
            });

            let hhdm_ptr = hadron_mm::hhdm::phys_to_virt(frame.start_address());
            // SAFETY: Frame was just allocated and is HHDM-mapped.
            let page_slice = unsafe {
                core::slice::from_raw_parts_mut(hhdm_ptr.as_u64() as *mut u8, PAGE_SIZE as usize)
            };
            page_slice.fill(0);

            let copy_start = page_addr.max(seg_vaddr);
            let data_end = seg_vaddr + seg.data.len() as u64;
            let copy_end = (page_addr + PAGE_SIZE).min(data_end);
            if copy_start < copy_end {
                let dst_offset = (copy_start - page_addr) as usize;
                let src_offset = (copy_start - seg_vaddr) as usize;
                let len = (copy_end - copy_start) as usize;
                page_slice[dst_offset..dst_offset + len]
                    .copy_from_slice(&seg.data[src_offset..src_offset + len]);
            }

            crate::vmm::with(|vmm| {
                hadron_mm::pmm::with(|pmm| {
                    let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
                    vmm.map_page(page, frame, flags, &mut alloc)
                        .expect("failed to map userboot page")
                        .flush();
                });
            });

            page_addr += PAGE_SIZE;
        }
    }

    // ── Allocate user stack ──────────────────────────────────────────
    let stack_bottom = USER_STACK_TOP - USER_STACK_PAGES * PAGE_SIZE;
    for i in 0..USER_STACK_PAGES {
        let page_addr = stack_bottom + i * PAGE_SIZE;
        let page = Page::<Size4KiB>::containing_address(VirtAddr::new(page_addr));
        let frame = hadron_mm::pmm::with(|pmm| {
            pmm.allocate_frame()
                .expect("PMM: out of frames for user stack")
        });

        let hhdm_ptr = hadron_mm::hhdm::phys_to_virt(frame.start_address());
        // SAFETY: Frame was just allocated and is HHDM-mapped.
        let page_slice = unsafe {
            core::slice::from_raw_parts_mut(hhdm_ptr.as_u64() as *mut u8, PAGE_SIZE as usize)
        };
        page_slice.fill(0);

        crate::vmm::with(|vmm| {
            hadron_mm::pmm::with(|pmm| {
                let mut alloc = hadron_mm::pmm::BitmapFrameAllocRef(pmm);
                let flags = MapFlags::USER | MapFlags::WRITABLE;
                vmm.map_page(page, frame, flags, &mut alloc)
                    .expect("failed to map user stack page")
                    .flush();
            });
        });
    }

    // ── Create Process and Thread objects ─────────────────────────────
    let root_vmar = Vmar::new_root(USER_BASE, USER_SIZE);
    let process = Process::new("userboot".to_string(), root_vmar);
    let thread = Thread::new("main".to_string(), &process);
    process.add_thread(Arc::clone(&thread));

    // Register in the global process table.
    crate::process::register_process(&process);

    crate::kinfo!(
        "boot",
        "userboot process created (pid={}), entering executor",
        process.koid().raw()
    );

    // ── Spawn process_task on the executor ───────────────────────────
    // Userboot uses the shared CR3 (no per-process address space).
    hadron_sched::spawn(crate::process::process_task(
        process,
        thread,
        None, // shared CR3
        entry,
        USER_STACK_TOP,
    ));

    // ── Start the executor (never returns) ───────────────────────────
    hadron_sched::executor().run(&HltHalt, make_steal_fn());
}

/// Halt implementation: enables interrupts and halts the CPU until an
/// interrupt arrives.
pub(crate) struct HltHalt;
impl ArchHalt for HltHalt {
    fn enable_interrupts_and_halt(&self) {
        // SAFETY: sti + hlt is safe; an interrupt will resume execution.
        unsafe { core::arch::asm!("sti", "hlt", "cli") };
    }
}

/// Creates a work-stealing closure for the executor.
pub(crate) fn make_steal_fn() -> fn() -> Option<(
    hadron_core::task::TaskId,
    hadron_core::task::Priority,
    hadron_sched::executor::TaskEntry,
)> {
    steal_from_other_cpus
}

/// Attempts to steal a task from another CPU's executor.
fn steal_from_other_cpus() -> Option<(
    hadron_core::task::TaskId,
    hadron_core::task::Priority,
    hadron_sched::executor::TaskEntry,
)> {
    #[cfg(hadron_smp)]
    {
        let my_cpu = hadron_core::cpu_local::current_cpu_id();
        let total = crate::arch::x86_64::smp::cpu_count();
        if total <= 1 {
            return None;
        }
        // Start from a pseudo-random CPU to avoid thundering herd.
        let start = (crate::time::nanos_since_boot() as u32) % total;
        for i in 0..total {
            let target = (start + i) % total;
            if target == my_cpu {
                continue;
            }
            let target_id = hadron_core::id::CpuId::new(target);
            if let Some(stolen) = hadron_sched::executor::for_cpu(target_id).steal_task() {
                return Some(stolen);
            }
        }
    }
    None
}

// ── Kernel integration tests ────────────────────────────────────────

#[cfg(ktest)]
mod ktest {
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
        let bti = crate::iommu_objects::bti::Bti::new(0, bdf).expect("Bti::new failed");
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
        let bti = crate::iommu_objects::bti::Bti::new(0, bdf).expect("Bti::new failed");

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
        let bti = crate::iommu_objects::bti::Bti::new(0, bdf).expect("Bti::new failed");

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
        let bti = crate::iommu_objects::bti::Bti::new(0, bdf).expect("Bti::new failed");
        let domain_id = bti.domain_id();

        // Drop the Bti — on_zero_handles should free the domain.
        use hadron_objects::object::KernelObject;
        bti.on_zero_handles();
        drop(bti);

        // Allocating a new domain should reuse the freed ID.
        let bti2 = crate::iommu_objects::bti::Bti::new(
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
}
