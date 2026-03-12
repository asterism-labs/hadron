//! UEFI boot stub for Hadron.
//!
//! Embeds the kernel ELF at build time, loads it into higher-half virtual memory,
//! and transfers control to `kernel_init` with a populated `BootInfo` struct.

#![no_std]
#![no_main]

use core::fmt::Write;

use hadron_boot_info::{BootInfo, BootMapFlags, BootServices, FramebufferInfo, PixelFormat};
use hadron_elf::{
    Elf64SectionHeader, ElfFile, R_X86_64_RELATIVE, RelaIter, RelocValue, SHT_RELA,
    compute_x86_64_reloc,
};
use uefi::EfiGuid;
use uefi::EfiHandle;
use uefi::EfiStatus;
use uefi::api::gop::Gop;
use uefi::api::{Boot, GraphicsOutputId, SystemTable};
use uefi::memory::{EfiAllocateType, EfiMemoryType};
use uefi::protocol::gop::PixelFormat as GopPixelFormat;
use uefi::table;

// ── Embedded kernel ELF ──────────────────────────────────────────────

static KERNEL_ELF: &[u8] =
    include_bytes!("../../../../build/kernel/x86_64-unknown-hadron/debug/hadron_kernel_image");

// ── Constants ────────────────────────────────────────────────────────

/// Virtual base of the kernel (must match the linker script).
const KERNEL_VADDR: u64 = 0xFFFF_FFFF_8000_0000;

/// Higher-half direct map base (maps all physical memory).
const HHDM_OFFSET: u64 = 0xFFFF_8000_0000_0000;

/// 4 KiB page size.
const PAGE_SIZE: u64 = 0x1000;

/// 2 MiB huge page size.
const HUGE_PAGE_SIZE: u64 = 0x20_0000;

/// Amount of physical memory to identity-map and HHDM-map (4 GiB).
const MAPPED_PHYS: u64 = 4 * 1024 * 1024 * 1024;

// Page table entry flags.
const PTE_PRESENT: u64 = 1 << 0;
const PTE_WRITABLE: u64 = 1 << 1;
const PTE_HUGE: u64 = 1 << 7;

/// Size of the kernel boot stack (64 KiB).
const BOOT_STACK_PAGES: usize = 16;

// ── Serial output (COM1) ─────────────────────────────────────────────

/// Write a byte to COM1 (port 0x3F8).
fn serial_byte(b: u8) {
    unsafe {
        core::arch::asm!(
            "out dx, al",
            in("dx") 0x3F8u16,
            in("al") b,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Write a string to COM1.
fn serial_str(s: &str) {
    for b in s.bytes() {
        serial_byte(b);
    }
}

/// Write a u64 as hex to COM1.
fn serial_hex(val: u64) {
    serial_str("0x");
    if val == 0 {
        serial_byte(b'0');
        return;
    }
    // Find highest non-zero nibble
    let mut started = false;
    for shift in (0..16).rev() {
        let nibble = ((val >> (shift * 4)) & 0xF) as u8;
        if nibble != 0 {
            started = true;
        }
        if started {
            serial_byte(if nibble < 10 {
                b'0' + nibble
            } else {
                b'a' + nibble - 10
            });
        }
    }
}

// ── Page table helpers ───────────────────────────────────────────────

/// A simple bump allocator for page-table pages from a pre-allocated pool.
struct PagePool {
    base: u64,
    next: u64,
    end: u64,
}

impl PagePool {
    fn new(base: u64, pages: usize) -> Self {
        Self {
            base,
            next: base,
            end: base + (pages as u64) * PAGE_SIZE,
        }
    }

    /// Returns the number of pages used from this pool.
    fn pages_used(&self) -> u64 {
        (self.next - self.base) / PAGE_SIZE
    }

    /// Allocate one zeroed 4 KiB page. Panics if the pool is exhausted.
    fn alloc_page(&mut self) -> u64 {
        if self.next >= self.end {
            serial_str("FATAL: page table pool exhausted\n");
            halt();
        }
        let addr = self.next;
        self.next += PAGE_SIZE;
        // SAFETY: UEFI allocated these pages, they are valid writable memory.
        unsafe {
            core::ptr::write_bytes(addr as *mut u8, 0, PAGE_SIZE as usize);
        }
        addr
    }
}

/// Get a mutable reference to a page table (512 × u64 entries) at a physical address.
///
/// # Safety
///
/// `phys` must point to a valid, 4 KiB-aligned, writable page.
unsafe fn pt_at(phys: u64) -> &'static mut [u64; 512] {
    // SAFETY: Caller guarantees `phys` points to a valid, 4 KiB-aligned, writable page.
    unsafe { &mut *(phys as *mut [u64; 512]) }
}

/// Ensure a page-table entry exists at `table[index]`, allocating from `pool` if absent.
/// Returns the physical address of the next-level table.
fn ensure_entry(table: &mut [u64; 512], index: usize, pool: &mut PagePool) -> u64 {
    if table[index] & PTE_PRESENT == 0 {
        let page = pool.alloc_page();
        table[index] = page | PTE_PRESENT | PTE_WRITABLE;
    }
    table[index] & !0xFFF
}

/// Map a single 4 KiB page at `vaddr` → `paddr` in the PML4 at `pml4`.
///
/// # Safety
///
/// `pml4` must point to a valid PML4 table. `pool` must have capacity.
unsafe fn map_4k_page(pml4: &mut [u64; 512], vaddr: u64, paddr: u64, pool: &mut PagePool) {
    let pml4_idx = ((vaddr >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((vaddr >> 30) & 0x1FF) as usize;
    let pd_idx = ((vaddr >> 21) & 0x1FF) as usize;
    let pt_idx = ((vaddr >> 12) & 0x1FF) as usize;

    // SAFETY: ensure_entry returns valid page table physical addresses from the pool.
    let pdpt_phys = ensure_entry(pml4, pml4_idx, pool);
    let pdpt = unsafe { pt_at(pdpt_phys) };
    let pd_phys = ensure_entry(pdpt, pdpt_idx, pool);
    let pd = unsafe { pt_at(pd_phys) };
    let pt_phys = ensure_entry(pd, pd_idx, pool);
    let pt = unsafe { pt_at(pt_phys) };

    pt[pt_idx] = paddr | PTE_PRESENT | PTE_WRITABLE;
}

/// Check whether a virtual address is already mapped in the given PML4.
///
/// # Safety
///
/// `pml4_phys` must point to a valid PML4 table.
unsafe fn is_mapped(pml4_phys: u64, vaddr: u64) -> bool {
    let pml4_idx = ((vaddr >> 39) & 0x1FF) as usize;
    let pdpt_idx = ((vaddr >> 30) & 0x1FF) as usize;
    let pd_idx = ((vaddr >> 21) & 0x1FF) as usize;
    let pt_idx = ((vaddr >> 12) & 0x1FF) as usize;

    // SAFETY: Caller guarantees pml4_phys is valid.
    let pml4 = unsafe { pt_at(pml4_phys) };
    if pml4[pml4_idx] & PTE_PRESENT == 0 {
        return false;
    }
    let pdpt = unsafe { pt_at(pml4[pml4_idx] & !0xFFF) };
    if pdpt[pdpt_idx] & PTE_PRESENT == 0 {
        return false;
    }
    // Check for 1 GiB huge page
    if pdpt[pdpt_idx] & PTE_HUGE != 0 {
        return true;
    }
    let pd = unsafe { pt_at(pdpt[pdpt_idx] & !0xFFF) };
    if pd[pd_idx] & PTE_PRESENT == 0 {
        return false;
    }
    // Check for 2 MiB huge page
    if pd[pd_idx] & PTE_HUGE != 0 {
        return true;
    }
    let pt = unsafe { pt_at(pd[pd_idx] & !0xFFF) };
    pt[pt_idx] & PTE_PRESENT != 0
}

// ── Boot services callback ──────────────────────────────────────────

/// State for the boot services callback. Lives at its physical address
/// via the identity map so it remains accessible from both identity-mapped
/// stub code and HHDM.
#[repr(C)]
struct BootServicesState {
    pool: PagePool,
    pml4_phys: u64,
    hhdm_offset: u64,
}

/// Boot services `map_pages` callback implementation.
///
/// Maps `count` physical pages starting at `phys` into the HHDM region.
/// Skips pages that are already mapped (e.g. within the initial 4 GiB).
///
/// # Safety
///
/// `ctx` must point to a valid `BootServicesState`. The stub's page tables
/// must be in CR3.
unsafe extern "C" fn boot_map_pages(
    ctx: *mut (),
    phys: u64,
    count: u64,
    _flags: BootMapFlags,
) -> u64 {
    // SAFETY: ctx was set to point to BootServicesState during setup.
    let state = unsafe { &mut *(ctx as *mut BootServicesState) };
    for i in 0..count {
        let pa = phys + i * PAGE_SIZE;
        let va = state.hhdm_offset + pa;
        // SAFETY: pml4_phys points to the PML4 we built.
        if !unsafe { is_mapped(state.pml4_phys, va) } {
            // SAFETY: PML4 is valid, pool has capacity.
            let pml4 = unsafe { pt_at(state.pml4_phys) };
            unsafe { map_4k_page(pml4, va, pa, &mut state.pool) };
        }
    }
    state.hhdm_offset + phys
}

// ── Fatal halt ───────────────────────────────────────────────────────

fn halt() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

// ── ELF section header iteration ─────────────────────────────────────

/// Iterate over ELF64 section headers.
fn section_headers<'a>(elf: &ElfFile<'a>) -> impl Iterator<Item = Elf64SectionHeader> + 'a {
    let hdr = elf.header();
    let data = elf.data();
    let shoff = hdr.e_shoff as usize;
    let shentsize = hdr.e_shentsize as usize;
    let shnum = hdr.e_shnum as usize;

    (0..shnum).map(move |i| {
        let offset = shoff + i * shentsize;
        Elf64SectionHeader::parse(data, offset)
    })
}

// ── Entry point ──────────────────────────────────────────────────────

/// UEFI application entry point.
#[unsafe(no_mangle)]
extern "efiapi" fn efi_main(handle: EfiHandle, system_table: *mut table::SystemTable) -> EfiStatus {
    // SAFETY: `handle` and `system_table` are provided by UEFI firmware at boot
    // and are valid for the duration of the boot phase.
    let st = unsafe { SystemTable::<Boot>::from_raw(handle, system_table) };

    let mut console = st.console_out();
    let _ = console.clear_screen();
    let _ = write!(console, "Hadron UEFI boot stub\n");

    // ── 1. Parse embedded kernel ELF ──────────────────────────────────

    let elf = match ElfFile::parse(KERNEL_ELF) {
        Ok(e) => e,
        Err(_) => {
            let _ = write!(console, "FATAL: invalid kernel ELF\n");
            halt();
        }
    };

    let entry_vaddr = elf.entry_point();
    let _ = write!(console, "Kernel entry: {:#018x}\n", entry_vaddr);

    // ── 2. Calculate physical memory needed for kernel segments ────────

    let mut kernel_phys_start: u64 = u64::MAX;
    let mut kernel_phys_end: u64 = 0;

    for seg in elf.load_segments() {
        // The segment vaddr is the intended virtual address; compute an offset
        // from KERNEL_VADDR to determine the relative position within the kernel image.
        let offset = seg.vaddr - KERNEL_VADDR;
        let seg_end = offset + seg.memsz;
        if offset < kernel_phys_start {
            kernel_phys_start = offset;
        }
        if seg_end > kernel_phys_end {
            kernel_phys_end = seg_end;
        }
    }

    let kernel_size = kernel_phys_end - kernel_phys_start;
    let kernel_pages = ((kernel_size + PAGE_SIZE - 1) / PAGE_SIZE) as usize;

    let _ = write!(
        console,
        "Kernel size: {} pages ({} KiB)\n",
        kernel_pages,
        kernel_pages * 4
    );

    // ── 3. Get GOP framebuffer info ───────────────────────────────────

    let bs = st.boot_services();
    let fb_info = match bs.locate_protocol::<GraphicsOutputId>() {
        Ok(raw) => {
            let gop = Gop::new(raw);
            let mode = gop.current_mode();
            let format = match mode.pixel_format {
                GopPixelFormat::BlueGreenRedReserved8BitPerColor => PixelFormat::Bgr,
                GopPixelFormat::RedGreenBlueReserved8BitPerColor => PixelFormat::Rgb,
                _ => PixelFormat::Bgr, // fallback
            };
            FramebufferInfo {
                base_phys: gop.frame_buffer_base(),
                size: gop.frame_buffer_size(),
                width: mode.horizontal_resolution,
                height: mode.vertical_resolution,
                stride: mode.pixels_per_scan_line,
                format,
            }
        }
        Err(_) => FramebufferInfo {
            base_phys: 0,
            size: 0,
            width: 0,
            height: 0,
            stride: 0,
            format: PixelFormat::Bgr,
        },
    };

    // ── 4. Allocate physical pages ────────────────────────────────────

    // Kernel image pages
    let kernel_phys = match bs.allocate_pages(
        EfiAllocateType::AllocateAnyPages,
        EfiMemoryType::LoaderData,
        kernel_pages,
    ) {
        Ok(addr) => addr,
        Err(e) => {
            let _ = write!(console, "FATAL: allocate kernel pages failed: {:?}\n", e);
            halt();
        }
    };

    // Page table pages (128 to accommodate identity + HHDM + kernel + on-demand mappings)
    let pt_pool_pages: usize = 128;
    let pt_pool_phys = match bs.allocate_pages(
        EfiAllocateType::AllocateAnyPages,
        EfiMemoryType::LoaderData,
        pt_pool_pages,
    ) {
        Ok(addr) => addr,
        Err(e) => {
            let _ = write!(
                console,
                "FATAL: allocate page table pages failed: {:?}\n",
                e
            );
            halt();
        }
    };

    // Boot stack pages
    let stack_phys = match bs.allocate_pages(
        EfiAllocateType::AllocateAnyPages,
        EfiMemoryType::LoaderData,
        BOOT_STACK_PAGES,
    ) {
        Ok(addr) => addr,
        Err(e) => {
            let _ = write!(console, "FATAL: allocate stack pages failed: {:?}\n", e);
            halt();
        }
    };

    // BootServicesState + BootServices vtable (1 page holds both)
    let boot_svc_state_phys = match bs.allocate_pages(
        EfiAllocateType::AllocateAnyPages,
        EfiMemoryType::LoaderData,
        1,
    ) {
        Ok(addr) => addr,
        Err(e) => {
            let _ = write!(
                console,
                "FATAL: allocate boot_svc_state page failed: {:?}\n",
                e
            );
            halt();
        }
    };

    // BootInfo struct (1 page)
    let boot_info_phys = match bs.allocate_pages(
        EfiAllocateType::AllocateAnyPages,
        EfiMemoryType::LoaderData,
        1,
    ) {
        Ok(addr) => addr,
        Err(e) => {
            let _ = write!(console, "FATAL: allocate boot_info page failed: {:?}\n", e);
            halt();
        }
    };

    let _ = write!(console, "Kernel @ phys {:#018x}\n", kernel_phys);
    let _ = write!(console, "Exiting boot services...\n");
    // Drop console borrow before exit_boot_services consumes st.
    drop(console);

    // ── 5. Exit boot services ─────────────────────────────────────────

    let mut mmap_buf = [0u8; 8192];
    let (_rt, memory_map) = match st.exit_boot_services(&mut mmap_buf) {
        Ok(pair) => pair,
        Err(_) => {
            serial_str("FATAL: exit_boot_services failed\n");
            halt();
        }
    };

    // From here on: no more UEFI boot services. Use serial for output.
    serial_str("[boot] Boot services exited\n");

    // ── 6. Find RSDP ─────────────────────────────────────────────────

    let mut rsdp_phys: u64 = 0;
    for ct in _rt.configuration_tables() {
        if ct.vendor_guid == EfiGuid::ACPI_20_TABLE {
            rsdp_phys = ct.vendor_table as u64;
            break;
        }
    }
    serial_str("[boot] RSDP: ");
    serial_hex(rsdp_phys);
    serial_str("\n");

    // ── 7. Copy kernel segments to physical memory ────────────────────

    // Zero the entire kernel region first (handles BSS)
    // SAFETY: kernel_phys was allocated above and is valid for kernel_pages * PAGE_SIZE bytes.
    unsafe {
        core::ptr::write_bytes(kernel_phys as *mut u8, 0, kernel_pages * PAGE_SIZE as usize);
    }

    for seg in elf.load_segments() {
        let offset = seg.vaddr - KERNEL_VADDR;
        let dst = (kernel_phys + offset) as *mut u8;
        // SAFETY: dst points into the allocated kernel region; seg.data is a valid slice
        // from the embedded ELF.
        unsafe {
            core::ptr::copy_nonoverlapping(seg.data.as_ptr(), dst, seg.data.len());
        }
    }
    serial_str("[boot] Kernel segments copied\n");

    // ── 8. Apply relocations ──────────────────────────────────────────

    // Find .rela.dyn section and apply R_X86_64_RELATIVE relocations.
    // For a static-PIE kernel loaded at KERNEL_VADDR, the base address is KERNEL_VADDR.
    let elf_data = elf.data();
    for shdr in section_headers(&elf) {
        if shdr.sh_type != SHT_RELA {
            continue;
        }
        let rela_offset = shdr.sh_offset as usize;
        let rela_end = rela_offset + shdr.sh_size as usize;
        let rela_iter = RelaIter::new(elf_data, rela_offset, rela_end);

        for rela in rela_iter {
            if rela.r_type != R_X86_64_RELATIVE {
                continue;
            }
            // base_addr = 0 because the ELF is linked at KERNEL_VADDR and loaded
            // at KERNEL_VADDR — no offset adjustment needed. The addends already
            // contain the correct absolute addresses.
            let (target_vaddr, value) = match compute_x86_64_reloc(&rela, 0, 0, rela.r_offset) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // target_vaddr is the virtual address where the relocation should be written.
            // Map it to the physical address in our allocated kernel memory.
            let phys_target = kernel_phys + (target_vaddr - KERNEL_VADDR);

            // SAFETY: phys_target is within the allocated kernel region.
            match value {
                RelocValue::U64(v) => unsafe {
                    (phys_target as *mut u64).write(v);
                },
                RelocValue::U32(v) => unsafe {
                    (phys_target as *mut u32).write(v);
                },
            }
        }
    }
    serial_str("[boot] Relocations applied\n");

    // ── 9. Build page tables ──────────────────────────────────────────

    let mut pool = PagePool::new(pt_pool_phys, pt_pool_pages);
    let pml4_phys = pool.alloc_page();

    // SAFETY: all page table pages were allocated by UEFI and are valid memory.
    unsafe {
        let pml4 = pt_at(pml4_phys);

        // --- Identity map first 4 GiB with 2 MiB huge pages ---
        // PML4[0] → PDPT_low
        let pdpt_low_phys = ensure_entry(pml4, 0, &mut pool);
        let pdpt_low = pt_at(pdpt_low_phys);

        for gib in 0..(MAPPED_PHYS / (1024 * 1024 * 1024)) as usize {
            let pd_phys = ensure_entry(pdpt_low, gib, &mut pool);
            let pd = pt_at(pd_phys);
            for entry in 0..512usize {
                let phys_addr =
                    (gib as u64) * (1024 * 1024 * 1024) + (entry as u64) * HUGE_PAGE_SIZE;
                pd[entry] = phys_addr | PTE_PRESENT | PTE_WRITABLE | PTE_HUGE;
            }
        }

        // --- HHDM: map first 4 GiB at HHDM_OFFSET with 2 MiB huge pages ---
        // PML4 index for HHDM_OFFSET: (0xFFFF_8000_0000_0000 >> 39) & 0x1FF = 256
        let hhdm_pml4_idx = ((HHDM_OFFSET >> 39) & 0x1FF) as usize;
        let pdpt_hhdm_phys = ensure_entry(pml4, hhdm_pml4_idx, &mut pool);
        let pdpt_hhdm = pt_at(pdpt_hhdm_phys);

        for gib in 0..(MAPPED_PHYS / (1024 * 1024 * 1024)) as usize {
            let pd_phys = ensure_entry(pdpt_hhdm, gib, &mut pool);
            let pd = pt_at(pd_phys);
            for entry in 0..512usize {
                let phys_addr =
                    (gib as u64) * (1024 * 1024 * 1024) + (entry as u64) * HUGE_PAGE_SIZE;
                pd[entry] = phys_addr | PTE_PRESENT | PTE_WRITABLE | PTE_HUGE;
            }
        }

        // --- Kernel mapping: KERNEL_VADDR → kernel_phys with 4 KiB pages ---
        for page_idx in 0..kernel_pages {
            let vaddr = KERNEL_VADDR + (page_idx as u64) * PAGE_SIZE;
            let paddr = kernel_phys + (page_idx as u64) * PAGE_SIZE;
            map_4k_page(pml4, vaddr, paddr, &mut pool);
        }
    }

    serial_str("[boot] Page tables built\n");

    // ── 10. Set up boot services callback ─────────────────────────────

    // SAFETY: boot_svc_state_phys was allocated by UEFI and is identity-mapped.
    let boot_svc_state = unsafe { &mut *(boot_svc_state_phys as *mut BootServicesState) };
    *boot_svc_state = BootServicesState {
        pool,
        pml4_phys,
        hhdm_offset: HHDM_OFFSET,
    };

    // Place the BootServices vtable right after the state in the same page.
    let vtable_offset = core::mem::size_of::<BootServicesState>();
    let vtable_ptr = (boot_svc_state_phys + vtable_offset as u64) as *mut BootServices;
    // SAFETY: vtable_ptr is within the allocated page and properly aligned for BootServices.
    unsafe {
        vtable_ptr.write(BootServices {
            ctx: boot_svc_state_phys as *mut (),
            map_pages: boot_map_pages,
        });
    }

    serial_str("[boot] Boot services callback ready\n");

    // ── 11. Build BootInfo ────────────────────────────────────────────

    let mmap_ptr = memory_map
        .iter()
        .next()
        .map_or(0u64, |desc| desc as *const _ as u64);

    // SAFETY: boot_info_phys was allocated above and is valid for one page.
    let boot_info = unsafe { &mut *(boot_info_phys as *mut BootInfo) };
    /// Default base address for kernel virtual regions (must match `hadron_mm::layout`).
    const DEFAULT_REGIONS_BASE: u64 = 0xFFFF_C000_0000_0000;

    let kernel_size_aligned = (kernel_pages as u64) * PAGE_SIZE;

    *boot_info = BootInfo {
        memory_map_ptr: mmap_ptr,
        memory_map_len: memory_map.len(),
        memory_descriptor_size: memory_map.descriptor_size(),
        rsdp_phys,
        framebuffer: fb_info,
        initrd_phys: 0,
        initrd_len: 0,
        hhdm_offset: HHDM_OFFSET,
        kaslr_slide: 0,
        regions_base: DEFAULT_REGIONS_BASE,
        kernel_phys,
        kernel_size: kernel_size_aligned,
        boot_pt_pool_phys: pt_pool_phys,
        boot_pt_pool_pages: boot_svc_state.pool.pages_used(),
        boot_pt_pool_total: pt_pool_pages as u64,
        boot_services: vtable_ptr as *const BootServices,
    };

    serial_str("[boot] BootInfo ready\n");

    // ── 11. Switch to new page tables and jump to kernel ──────────────

    // Stack top (grows downward): stack_phys + BOOT_STACK_PAGES * PAGE_SIZE
    let stack_top = stack_phys + (BOOT_STACK_PAGES as u64) * PAGE_SIZE;

    // BootInfo virtual address via HHDM
    let boot_info_vaddr = HHDM_OFFSET + boot_info_phys;

    serial_str("[boot] Jumping to kernel at ");
    serial_hex(entry_vaddr);
    serial_str("\n");

    // SAFETY: We've built valid page tables mapping:
    //   - identity (0..4G → 0..4G): keeps this code running after CR3 switch
    //   - HHDM (0xFFFF_8000_0000_0000..+4G → 0..4G): kernel's physical memory access
    //   - kernel (0xFFFF_FFFF_8000_0000..+size → kernel_phys): kernel code/data
    // The kernel entry point expects: extern "C" fn(*const BootInfo) -> !
    unsafe {
        core::arch::asm!(
            // Load new page tables
            "mov cr3, {pml4}",
            // Set up kernel boot stack (using HHDM address for stack)
            "mov rsp, {stack}",
            // Pass BootInfo pointer as first argument (rdi in SysV ABI)
            "mov rdi, {boot_info}",
            // Jump to kernel entry point
            "jmp {entry}",
            pml4 = in(reg) pml4_phys,
            stack = in(reg) HHDM_OFFSET + stack_top,
            boot_info = in(reg) boot_info_vaddr,
            entry = in(reg) entry_vaddr,
            options(noreturn),
        );
    }
}

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    serial_str("PANIC: ");
    // Print the panic message if available
    if let Some(msg) = info.message().as_str() {
        serial_str(msg);
    }
    serial_str("\n");
    halt()
}
