//! Limine boot stub for Hadron kernel.
//!
//! This crate is the Limine-specific entry point. It declares Limine protocol
//! requests, converts the bootloader responses into the kernel's
//! [`BootInfo`](hadron_kernel::boot::BootInfo) types, builds kernel-owned page
//! tables, switches CR3, and calls [`kernel_init`](hadron_kernel::kernel_init).

#![no_std]
#![no_main]

mod requests;

use requests::REQUESTS;

use hadron_kernel::addr::{PhysAddr, VirtAddr};
use hadron_kernel::arch::x86_64::structures::paging::{PageTable, PageTableEntry, PageTableFlags};
use hadron_kernel::paging::{PhysFrame, Size4KiB};
use hadron_kernel::arch::x86_64::paging::PageTableMapper;
use hadron_kernel::boot::{
    BootInfoData, FramebufferInfo, InitrdInfo, KernelAddressInfo, MAX_FRAMEBUFFERS,
    MAX_MEMORY_REGIONS, MAX_SMP_CPUS, MemoryRegion, MemoryRegionKind, PagingMode, PixelFormat,
    SmpCpuEntry,
};
use noalloc::vec::ArrayVec;

unsafe extern "C" {
    static __text_start: u8;
    static __text_end: u8;
    static __rodata_start: u8;
    static __rodata_end: u8;
    static __data_start: u8;
    static __data_end: u8;
}

// ---------------------------------------------------------------------------
// Bump frame allocator
// ---------------------------------------------------------------------------

struct BumpFrameAllocator {
    /// Current allocation pointer (physical address), decrements by 4K per alloc.
    next: u64,
    /// Lower bound (start of the usable region).
    limit: u64,
    /// HHDM offset for zeroing frames.
    hhdm_offset: u64,
    /// Number of frames allocated.
    count: u64,
}

impl BumpFrameAllocator {
    fn new(region_start: u64, region_end: u64, hhdm_offset: u64) -> Self {
        Self {
            next: region_end,
            limit: region_start,
            hhdm_offset,
            count: 0,
        }
    }

    /// Allocates a zeroed 4 KiB frame. Panics if out of frames.
    fn alloc_frame(&mut self) -> PhysFrame<Size4KiB> {
        assert!(self.next >= self.limit + 0x1000, "out of page table frames");
        self.next -= 0x1000;
        self.count += 1;
        let virt = (self.hhdm_offset + self.next) as *mut u8;
        unsafe { core::ptr::write_bytes(virt, 0, 0x1000) };
        PhysFrame::containing_address(PhysAddr::new(self.next))
    }
}

/// Limine entry point. This is called by the bootloader after it has loaded the kernel
/// and populated the `REQUESTS` struct with responses. This function must not return, and should
/// call `kernel_init` to enter the kernel proper.
#[unsafe(no_mangle)]
extern "C" fn _start() -> ! {
    let serial = hadron_drivers::uart16550::Uart16550::new(hadron_drivers::uart16550::COM1);
    unsafe { serial.init(hadron_drivers::uart16550::BaudRate::Baud115200) }
        .expect("COM1 init failed");
    hadron_kernel::log::init_early_serial();

    // 2. Assert base revision is supported.
    assert!(REQUESTS.base_revision.is_supported());

    hadron_kernel::kinfo!("Hadron OS booting with Limine...");

    // 3. Read HHDM offset and memory map from Limine responses.
    let hhdm_offset = REQUESTS
        .hhdm
        .response()
        .expect("HHDM response not available")
        .hhdm_base;

    let memmap_response = REQUESTS
        .memmap
        .response()
        .expect("Memory map response not available");

    let exec_addr = REQUESTS
        .executable_address
        .response()
        .expect("Executable address response not available");
    let kernel_phys_base = PhysAddr::new(exec_addr.phys_base);
    let kernel_virt_base = VirtAddr::new(exec_addr.virt_base);

    // 4. Init bump frame allocator from the largest usable region.
    let mut largest_start = 0u64;
    let mut largest_size = 0u64;
    for entry in memmap_response.entries() {
        if entry.type_ == limine::memmap::MemMapEntryType::Usable && entry.length > largest_size {
            largest_start = entry.base;
            largest_size = entry.length;
        }
    }
    assert!(largest_size >= 0x10_0000, "no large usable memory region");

    let mut alloc =
        BumpFrameAllocator::new(largest_start, largest_start + largest_size, hhdm_offset);

    // 5. Build framebuffer list early (needed for page table mapping).
    let framebuffers = build_framebuffers();

    // 6. Build kernel page tables.
    let pml4_phys = build_page_tables(
        hhdm_offset,
        memmap_response,
        kernel_phys_base,
        kernel_virt_base,
        &framebuffers,
        &mut alloc,
    );

    let frames_used = alloc.count;
    hadron_kernel::kdebug!(
        "Page tables built: PML4 @ {}, {} frames ({} KiB)",
        pml4_phys,
        frames_used,
        frames_used * 4
    );

    // 7. Set CPU control bits: EFER.NXE, CR4.PGE, CR0.WP, PAT.
    hadron_kernel::kdebug!("Setting CPU control bits (EFER.NXE, CR4.PGE, CR0.WP, PAT)...");
    unsafe { set_cpu_control_bits() };
    hadron_kernel::kdebug!("CPU control bits set");

    // 9. Switch CR3 to new PML4.
    hadron_kernel::kdebug!("Switching CR3 to {}...", pml4_phys);
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) pml4_phys.as_u64(), options(nostack, preserves_flags))
    };

    hadron_kernel::kdebug!("CR3 switched to kernel-owned page tables");

    // 10. Extract boot modules by cmdline string.
    let mut initrd = None;
    if let Some(resp) = REQUESTS.modules.response() {
        for file in resp.modules() {
            let module_name = file.name();
            let virt_addr = file.address as u64;
            let phys_addr = PhysAddr::new(virt_addr - hhdm_offset);
            match module_name {
                "initrd" => {
                    initrd = Some(InitrdInfo {
                        phys_addr,
                        size: file.size,
                    });
                }
                _ => {
                    hadron_kernel::kwarn!("Unknown boot module: name={}", module_name);
                }
            }
        }
    }

    // 10b. Build SMP CPU entry list from Limine MP response.
    let (smp_cpus, bsp_lapic_id) = build_smp_cpus();

    // 10c. Park APs on kernel page tables immediately.
    // Limine starts APs in a spin loop using shared page tables (base revision 0).
    // The BSP's kernel init can corrupt the AP's execution environment, so we
    // must park them on the kernel page tables before proceeding.
    hadron_kernel::arch::x86_64::smp::park_aps(smp_cpus.as_slice(), pml4_phys.as_u64());

    // 11. Build BootInfoData (after CR3 switch, using new page tables).
    let boot_info = build_boot_info(
        hhdm_offset,
        kernel_phys_base,
        kernel_virt_base,
        framebuffers,
        pml4_phys,
        largest_start,
        largest_size,
        frames_used,
        initrd,
        smp_cpus,
        bsp_lapic_id,
    );

    // 11. Log detailed boot info to all sinks.
    log_boot_info(&boot_info);

    // 13. Enter the kernel.
    hadron_kernel::kernel_init(&boot_info);
}

// ---------------------------------------------------------------------------
// Page table construction
// ---------------------------------------------------------------------------

fn build_page_tables(
    hhdm_offset: u64,
    memmap_response: &limine::MemMapResponse,
    kernel_phys_base: PhysAddr,
    kernel_virt_base: VirtAddr,
    framebuffers: &ArrayVec<FramebufferInfo, MAX_FRAMEBUFFERS>,
    alloc: &mut BumpFrameAllocator,
) -> PhysAddr {
    let mapper = PageTableMapper::new(hhdm_offset);
    let pml4_phys = alloc.alloc_frame().start_address();

    // --- HHDM mappings (2 MiB huge pages, sequential fill) ---
    // TODO: switch to 1 GiB pages (map_1gib) when running on hardware with PDPE1GB support
    let mut max_phys: u64 = 0;
    for entry in memmap_response.entries() {
        let end = entry.base + entry.length;
        if end > max_phys {
            max_phys = end;
        }
    }
    let max_phys = (max_phys + 0x1F_FFFF) & !0x1F_FFFF; // round up to 2 MiB
    let hhdm_pages = max_phys / 0x20_0000;

    hadron_kernel::kdebug!(
        "Mapping HHDM: {} MiB physical address space ({} x 2 MiB pages)",
        max_phys / (1024 * 1024),
        hhdm_pages,
    );

    // Build HHDM by filling page directory tables sequentially instead of
    // re-walking PML4→PDPT→PD on every 2 MiB page. This avoids O(n) pointer
    // chasing and reduces the mapping to O(n/512) table allocations.
    let hhdm_leaf_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::GLOBAL
        | PageTableFlags::NO_EXECUTE
        | PageTableFlags::HUGE_PAGE;
    let intermediate_flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;

    let pml4 = unsafe { &mut *((hhdm_offset + pml4_phys.as_u64()) as *mut PageTable) };

    let mut phys: u64 = 0;
    while phys < max_phys {
        let virt = hhdm_offset + phys;
        let pml4_idx = ((virt >> 39) & 0x1FF) as usize;
        let pdpt_idx = ((virt >> 30) & 0x1FF) as usize;

        // Ensure PDPT exists for this PML4 slot.
        let pdpt_phys = if pml4.entries[pml4_idx].is_present() {
            pml4.entries[pml4_idx].address()
        } else {
            let frame = alloc.alloc_frame().start_address();
            pml4.entries[pml4_idx] = PageTableEntry::new(frame, intermediate_flags);
            frame
        };
        let pdpt = unsafe { &mut *((hhdm_offset + pdpt_phys.as_u64()) as *mut PageTable) };

        // Ensure PD exists for this PDPT slot.
        let pd_phys = if pdpt.entries[pdpt_idx].is_present() {
            pdpt.entries[pdpt_idx].address()
        } else {
            let frame = alloc.alloc_frame().start_address();
            pdpt.entries[pdpt_idx] = PageTableEntry::new(frame, intermediate_flags);
            frame
        };
        let pd = unsafe { &mut *((hhdm_offset + pd_phys.as_u64()) as *mut PageTable) };

        // Fill this entire PD (up to 512 entries = 1 GiB) in one shot.
        let pd_start_idx = ((virt >> 21) & 0x1FF) as usize;
        for pd_idx in pd_start_idx..512 {
            if phys >= max_phys {
                break;
            }
            pd.entries[pd_idx] = PageTableEntry::new(PhysAddr::new(phys), hhdm_leaf_flags);
            phys += 0x20_0000; // 2 MiB
        }
    }

    // --- Kernel image mappings (4 KiB pages for precise permissions) ---
    let text_start = VirtAddr::new(core::ptr::addr_of!(__text_start) as u64);
    let text_end = VirtAddr::new(core::ptr::addr_of!(__text_end) as u64);
    let rodata_start = VirtAddr::new(core::ptr::addr_of!(__rodata_start) as u64);
    let rodata_end = VirtAddr::new(core::ptr::addr_of!(__rodata_end) as u64);
    let data_start = VirtAddr::new(core::ptr::addr_of!(__data_start) as u64);
    let data_end = VirtAddr::new(core::ptr::addr_of!(__data_end) as u64);

    // .text: executable, read-only
    let text_flags = PageTableFlags::PRESENT | PageTableFlags::GLOBAL;
    map_kernel_range(
        &mapper,
        pml4_phys,
        text_start,
        text_end,
        kernel_phys_base,
        kernel_virt_base,
        text_flags,
        alloc,
    );

    // .rodata: read-only, no execute
    let rodata_flags =
        PageTableFlags::PRESENT | PageTableFlags::GLOBAL | PageTableFlags::NO_EXECUTE;
    map_kernel_range(
        &mapper,
        pml4_phys,
        rodata_start,
        rodata_end,
        kernel_phys_base,
        kernel_virt_base,
        rodata_flags,
        alloc,
    );

    // .data + .bss: read-write, no execute
    let data_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::GLOBAL
        | PageTableFlags::NO_EXECUTE;
    map_kernel_range(
        &mapper,
        pml4_phys,
        data_start,
        data_end,
        kernel_phys_base,
        kernel_virt_base,
        data_flags,
        alloc,
    );

    // --- Framebuffer mappings (2 MiB huge pages, write-combine via PAT entry 4) ---
    let fb_flags = PageTableFlags::PRESENT
        | PageTableFlags::WRITABLE
        | PageTableFlags::NO_EXECUTE
        | PageTableFlags::PAT_HUGE; // WC via PAT entry 4

    for fb in framebuffers.iter() {
        let fb_virt = fb.address.as_u64();
        let fb_phys = fb_virt - hhdm_offset;
        let fb_size = fb.pitch as u64 * fb.height as u64;
        let fb_phys_start = fb_phys & !0x1F_FFFF;
        let fb_phys_end = (fb_phys + fb_size + 0x1F_FFFF) & !0x1F_FFFF;

        let mut phys = fb_phys_start;
        while phys < fb_phys_end {
            let virt = VirtAddr::new_truncate(hhdm_offset + phys);
            let phys_addr = PhysAddr::new(phys);
            unsafe {
                mapper.map_2mib(pml4_phys, virt, phys_addr, fb_flags, &mut || {
                    alloc.alloc_frame()
                });
            }
            phys += 0x20_0000;
        }
    }

    // --- Identity map first 2 MiB (for CR3 switch transition) ---
    let identity_flags = PageTableFlags::PRESENT | PageTableFlags::WRITABLE;
    unsafe {
        mapper.map_2mib(
            pml4_phys,
            VirtAddr::zero(),
            PhysAddr::zero(),
            identity_flags,
            &mut || alloc.alloc_frame(),
        );
    }

    pml4_phys
}

/// Maps a kernel section range using 4 KiB pages.
fn map_kernel_range(
    mapper: &PageTableMapper,
    pml4_phys: PhysAddr,
    virt_start: VirtAddr,
    virt_end: VirtAddr,
    kernel_phys_base: PhysAddr,
    kernel_virt_base: VirtAddr,
    flags: PageTableFlags,
    alloc: &mut BumpFrameAllocator,
) {
    let start = virt_start.align_down(0x1000);
    let end = virt_end.align_up(0x1000);

    let mut virt = start.as_u64();
    let end_val = end.as_u64();
    while virt < end_val {
        let phys = PhysAddr::new((virt - kernel_virt_base.as_u64()) + kernel_phys_base.as_u64());
        unsafe {
            mapper.map_4k(pml4_phys, VirtAddr::new(virt), phys, flags, &mut || {
                alloc.alloc_frame()
            });
        }
        virt += 0x1000;
    }
}

// ---------------------------------------------------------------------------
// CPU control bits
// ---------------------------------------------------------------------------

unsafe fn set_cpu_control_bits() {
    unsafe {
        // Enable EFER.NXE (bit 11)
        core::arch::asm!(
            "mov ecx, 0xC0000080",  // IA32_EFER MSR
            "rdmsr",
            "or eax, (1 << 11)",    // NXE bit
            "wrmsr",
            out("ecx") _, out("eax") _, out("edx") _,
            options(nomem, nostack),
        );

        // Enable CR4.PGE (bit 7)
        core::arch::asm!(
            "mov {tmp}, cr4",
            "or {tmp}, (1 << 7)",
            "mov cr4, {tmp}",
            tmp = out(reg) _,
            options(nomem, nostack),
        );

        // Enable CR0.WP (bit 16)
        core::arch::asm!(
            "mov {tmp}, cr0",
            "or {tmp}, (1 << 16)",
            "mov cr0, {tmp}",
            tmp = out(reg) _,
            options(nomem, nostack),
        );

        // Program PAT MSR: change entry 4 from WB to WC
        core::arch::asm!(
            "mov ecx, 0x277",   // IA32_PAT MSR
            "rdmsr",
            "and edx, 0xFFFFFF00",
            "or  edx, 0x01",       // PA4 = WC (0x01)
            "wrmsr",
            out("ecx") _, out("eax") _, out("edx") _,
            options(nomem, nostack),
        );
    }
}

// ---------------------------------------------------------------------------
// Boot info construction
// ---------------------------------------------------------------------------

fn build_boot_info(
    hhdm_offset: u64,
    kernel_phys_base: PhysAddr,
    kernel_virt_base: VirtAddr,
    framebuffers: ArrayVec<FramebufferInfo, MAX_FRAMEBUFFERS>,
    page_table_root: PhysAddr,
    alloc_region_start: u64,
    alloc_region_size: u64,
    frames_used: u64,
    initrd: Option<InitrdInfo>,
    smp_cpus: ArrayVec<SmpCpuEntry, MAX_SMP_CPUS>,
    bsp_lapic_id: u32,
) -> BootInfoData {
    let memory_map = build_memory_map(alloc_region_start, alloc_region_size, frames_used);

    let kernel_address = KernelAddressInfo {
        physical_base: kernel_phys_base,
        virtual_base: kernel_virt_base,
    };

    let paging_mode = convert_paging_mode(
        REQUESTS
            .paging_mode
            .response()
            .expect("Paging mode response not available")
            .paging_mode,
    );

    let rsdp_address = REQUESTS
        .rsdp
        .response()
        .map(|r| PhysAddr::new(r.rsdp_addr - hhdm_offset));
    let dtb_address = REQUESTS
        .dtb
        .response()
        .map(|r| PhysAddr::new(r.dtb_addr - hhdm_offset));
    let command_line = REQUESTS.cmdline.response().map(|r| r.cmdline());

    let (smbios_32, smbios_64) = REQUESTS
        .smbios
        .response()
        .map(|r| {
            let s32 = if r.entry_32_addr != 0 {
                Some(PhysAddr::new(u64::from(r.entry_32_addr)))
            } else {
                None
            };
            let s64 = if r.entry_64_addr != 0 {
                Some(PhysAddr::new(r.entry_64_addr))
            } else {
                None
            };
            (s32, s64)
        })
        .unwrap_or((None, None));

    BootInfoData {
        memory_map,
        hhdm_offset,
        kernel_address,
        paging_mode,
        framebuffers,
        rsdp_address,
        dtb_address,
        command_line,
        smbios_32,
        smbios_64,
        page_table_root,
        initrd,
        smp_cpus,
        bsp_lapic_id,
    }
}

// ---------------------------------------------------------------------------
// Conversion helpers
// ---------------------------------------------------------------------------

fn build_memory_map(
    alloc_region_start: u64,
    alloc_region_size: u64,
    frames_used: u64,
) -> ArrayVec<MemoryRegion, MAX_MEMORY_REGIONS> {
    let response = REQUESTS
        .memmap
        .response()
        .expect("Memory map response not available");

    let consumed_bytes = frames_used * 0x1000;

    let mut regions = ArrayVec::new();
    for entry in response.entries() {
        let start = entry.base;
        let mut size = entry.length;

        if entry.type_ == limine::memmap::MemMapEntryType::Usable
            && start == alloc_region_start
            && entry.length == alloc_region_size
        {
            size -= consumed_bytes;
        }

        regions.push(MemoryRegion {
            start: PhysAddr::new(start),
            size,
            kind: convert_memory_kind(entry.type_),
        });
    }
    regions
}

fn convert_memory_kind(kind: limine::memmap::MemMapEntryType) -> MemoryRegionKind {
    use limine::memmap::MemMapEntryType;
    match kind {
        MemMapEntryType::Usable => MemoryRegionKind::Usable,
        MemMapEntryType::Reserved => MemoryRegionKind::Reserved,
        MemMapEntryType::AcpiReclaimable | MemMapEntryType::AcpiTables => {
            MemoryRegionKind::AcpiReclaimable
        }
        MemMapEntryType::AcpiNvs => MemoryRegionKind::AcpiNvs,
        MemMapEntryType::BadMemory => MemoryRegionKind::BadMemory,
        MemMapEntryType::BootloaderReclaimable => MemoryRegionKind::BootloaderReclaimable,
        MemMapEntryType::KernelAndModules => MemoryRegionKind::KernelAndModules,
        MemMapEntryType::Framebuffer => MemoryRegionKind::Framebuffer,
    }
}

fn convert_paging_mode(mode: limine::paging::PagingMode) -> PagingMode {
    use limine::paging::PagingMode as LiminePaging;
    match mode {
        LiminePaging::Paging4Level => PagingMode::Level4,
        LiminePaging::Paging5Level => PagingMode::Level5,
        _ => panic!("unsupported paging mode"),
    }
}

/// Builds the SMP CPU entry list from the Limine MP response.
///
/// Returns `(cpu_entries, bsp_lapic_id)`. The entries include only non-BSP
/// CPUs (APs) since the BSP does not need to be started.
fn build_smp_cpus() -> (ArrayVec<SmpCpuEntry, MAX_SMP_CPUS>, u32) {
    let Some(mp_response) = REQUESTS.mp.response() else {
        return (ArrayVec::new(), 0);
    };

    // Debug: print raw response fields to diagnose potential layout issues.
    hadron_kernel::kdebug!(
        "MP response: revision={}, flags={:#x}, bsp_lapic_id={}, cpu_count={}",
        mp_response.revision,
        mp_response.flags,
        mp_response.bsp_lapic_id,
        mp_response.cpu_count,
    );

    let bsp_lapic_id = mp_response.bsp_lapic_id;
    let mut cpus = ArrayVec::new();

    for cpu_info in mp_response.cpus() {
        // Skip the BSP — it's already running.
        if cpu_info.lapic_id == bsp_lapic_id {
            continue;
        }

        // Compute pointers to the goto_address and extra_argument fields.
        let info_ptr = cpu_info as *const limine::mp::MpInfo;
        // SAFETY: MpInfo is #[repr(C)]. goto_address is at offset 16 (after
        // processor_id: u32 + lapic_id: u32 + _reserved: u64 = 16 bytes).
        // extra_argument is at offset 24 (goto_address is 8 bytes).
        let goto_ptr = unsafe { (info_ptr as *mut u8).add(16) as *mut u64 };
        let extra_ptr = unsafe { (info_ptr as *mut u8).add(24) as *mut u64 };

        cpus.push(SmpCpuEntry {
            processor_id: cpu_info.processor_id,
            lapic_id: cpu_info.lapic_id,
            goto_address_ptr: goto_ptr,
            extra_argument_ptr: extra_ptr,
        });
    }

    hadron_kernel::kinfo!(
        "MP: {} CPUs detected (BSP LAPIC ID={}), {} APs to boot",
        mp_response.cpu_count,
        bsp_lapic_id,
        cpus.len()
    );

    (cpus, bsp_lapic_id)
}

fn build_framebuffers() -> ArrayVec<FramebufferInfo, MAX_FRAMEBUFFERS> {
    let mut fbs = ArrayVec::new();
    let Some(response) = REQUESTS.framebuffer.response() else {
        return fbs;
    };

    for fb in response.framebuffers() {
        if fbs.len() >= MAX_FRAMEBUFFERS {
            break;
        }
        let mode = &fb.default_mode;
        let pixel_format = PixelFormat::Bitmask {
            red_size: mode.red_mask_size,
            red_shift: mode.red_mask_shift,
            green_size: mode.green_mask_size,
            green_shift: mode.green_mask_shift,
            blue_size: mode.blue_mask_size,
            blue_shift: mode.blue_mask_shift,
        };

        fbs.push(FramebufferInfo {
            address: VirtAddr::new(fb.addr.as_ptr() as u64),
            width: mode.width as u32,
            height: mode.height as u32,
            pitch: mode.pitch as u32,
            bpp: mode.bpp as u8,
            pixel_format,
        });
    }
    fbs
}

// ---------------------------------------------------------------------------
// Detailed boot info logging
// ---------------------------------------------------------------------------

fn log_boot_info(boot_info: &BootInfoData) {
    hadron_kernel::kinfo!("=== Hadron OS Boot Info ===");

    if let Some(bl) = REQUESTS.bootloader_info.response() {
        hadron_kernel::kinfo!("Bootloader: {} {}", bl.name(), bl.version());
    }

    if let Some(fw) = REQUESTS.firmware_type.response() {
        let fw_name = match fw.firmware_type {
            limine::FirmwareType::Bios => "BIOS",
            limine::FirmwareType::Efi32 => "UEFI 32-bit",
            limine::FirmwareType::Efi64 => "UEFI 64-bit",
            limine::FirmwareType::Sbi => "SBI",
            _ => "Unknown",
        };
        hadron_kernel::kinfo!("Firmware: {}", fw_name);
    }

    hadron_kernel::kdebug!("HHDM offset: {:#x}", boot_info.hhdm_offset);
    hadron_kernel::ktrace!("Page table root (CR3): {}", boot_info.page_table_root);
    hadron_kernel::ktrace!(
        "Kernel phys base: {}, virt base: {}",
        boot_info.kernel_address.physical_base,
        boot_info.kernel_address.virtual_base
    );

    let paging_str = match boot_info.paging_mode {
        PagingMode::Level4 => "4-level",
        PagingMode::Level5 => "5-level",
    };
    hadron_kernel::kinfo!("Paging mode: {}", paging_str);

    hadron_kernel::kinfo!("Memory map ({} regions):", boot_info.memory_map.len());
    let mut usable_kib = 0u64;
    let mut reserved_kib = 0u64;
    let mut reclaimable_kib = 0u64;
    for region in boot_info.memory_map.iter() {
        let end = region.start + region.size;
        let size_kib = region.size / 1024;
        let kind_str = match region.kind {
            MemoryRegionKind::Usable => {
                usable_kib += size_kib;
                "Usable"
            }
            MemoryRegionKind::Reserved => {
                reserved_kib += size_kib;
                "Reserved"
            }
            MemoryRegionKind::AcpiReclaimable => {
                reclaimable_kib += size_kib;
                "ACPI Reclaimable"
            }
            MemoryRegionKind::AcpiNvs => {
                reserved_kib += size_kib;
                "ACPI NVS"
            }
            MemoryRegionKind::BadMemory => "Bad Memory",
            MemoryRegionKind::BootloaderReclaimable => {
                reclaimable_kib += size_kib;
                "Bootloader Reclaimable"
            }
            MemoryRegionKind::KernelAndModules => {
                reserved_kib += size_kib;
                "Kernel+Modules"
            }
            MemoryRegionKind::Framebuffer => {
                reserved_kib += size_kib;
                "Framebuffer"
            }
        };
        hadron_kernel::kdebug!(
            "  {}..{} {:>8} KiB  {}",
            region.start,
            end,
            size_kib,
            kind_str
        );
    }
    hadron_kernel::kinfo!(
        "Memory totals: {} MiB usable, {} MiB reserved, {} MiB reclaimable",
        usable_kib / 1024,
        reserved_kib / 1024,
        reclaimable_kib / 1024
    );

    for (i, fb) in boot_info.framebuffers.iter().enumerate() {
        let fmt_str = match fb.pixel_format {
            PixelFormat::Rgb32 => "RGB32",
            PixelFormat::Bgr32 => "BGR32",
            PixelFormat::Bitmask { .. } => "Bitmask",
        };
        hadron_kernel::kinfo!(
            "Framebuffer[{}]: {}x{} @ {}, pitch={}, bpp={}, format={}",
            i,
            fb.width,
            fb.height,
            fb.address,
            fb.pitch,
            fb.bpp,
            fmt_str
        );
    }

    if let Some(addr) = boot_info.rsdp_address {
        hadron_kernel::kdebug!("RSDP: {}", addr);
    }
    if let Some(addr) = boot_info.smbios_32 {
        hadron_kernel::ktrace!("SMBIOS 32-bit: {}", addr);
    }
    if let Some(addr) = boot_info.smbios_64 {
        hadron_kernel::ktrace!("SMBIOS 64-bit: {}", addr);
    }
    if let Some(cmdline) = boot_info.command_line {
        hadron_kernel::kinfo!("Command line: {}", cmdline);
    }
    if let Some(addr) = boot_info.dtb_address {
        hadron_kernel::ktrace!("DTB: {}", addr);
    }
    if let Some(ref initrd) = boot_info.initrd {
        hadron_kernel::kinfo!("Initrd: {} ({} bytes)", initrd.phys_addr, initrd.size);
    }

    if let Some(date) = REQUESTS.date_at_boot.response() {
        hadron_kernel::kinfo!("Boot timestamp: {} (UNIX)", date.timestamp);
    }

    if let Some(perf) = REQUESTS.bootloader_performance.response() {
        hadron_kernel::kdebug!(
            "Boot perf: reset={}us, init={}us, exec={}us",
            perf.reset_us,
            perf.init_us,
            perf.exec_us
        );
    }

    hadron_kernel::kinfo!("===========================");
}

// ---------------------------------------------------------------------------
// Panic handler
// ---------------------------------------------------------------------------

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    hadron_kernel::log::panic_serial(info);
    loop {
        core::hint::spin_loop();
    }
}
