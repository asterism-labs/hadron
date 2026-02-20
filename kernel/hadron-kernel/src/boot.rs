//! Bootloader-agnostic boot information types and kernel entry point.
//!
//! This module defines the [`BootInfo`] trait that abstracts over different bootloaders
//! (Limine, UEFI stub, etc.) and provides a uniform interface for the kernel to access
//! boot-time information such as the memory map, framebuffer, and HHDM offset.

extern crate alloc;
use alloc::boxed::Box;

use crate::addr::{PhysAddr, VirtAddr};
use noalloc::vec::ArrayVec;

/// The kind of a physical memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegionKind {
    /// Normal usable RAM.
    Usable,
    /// Reserved by firmware or hardware.
    Reserved,
    /// ACPI tables that can be reclaimed after parsing.
    AcpiReclaimable,
    /// ACPI Non-Volatile Storage -- must not be used.
    AcpiNvs,
    /// Defective physical memory.
    BadMemory,
    /// Memory used by the bootloader, reclaimable after boot.
    BootloaderReclaimable,
    /// Memory occupied by the kernel image and loaded modules.
    KernelAndModules,
    /// Memory-mapped framebuffer region.
    Framebuffer,
}

/// A contiguous physical memory region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryRegion {
    /// Physical start address.
    pub start: PhysAddr,
    /// Size in bytes.
    pub size: u64,
    /// Kind of memory region.
    pub kind: MemoryRegionKind,
}

/// Pixel format of a framebuffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit RGB (red at lowest byte offset). UEFI `RedGreenBlueReserved8BitPerColor`.
    Rgb32,
    /// 32-bit BGR (blue at lowest byte offset). UEFI `BlueGreenRedReserved8BitPerColor`.
    Bgr32,
    /// Arbitrary bitmask layout described by per-channel size and shift.
    Bitmask {
        /// Number of bits in the red channel.
        red_size: u8,
        /// Bit position of the red channel (from LSB).
        red_shift: u8,
        /// Number of bits in the green channel.
        green_size: u8,
        /// Bit position of the green channel (from LSB).
        green_shift: u8,
        /// Number of bits in the blue channel.
        blue_size: u8,
        /// Bit position of the blue channel (from LSB).
        blue_shift: u8,
    },
}

/// Information about a linear framebuffer.
#[derive(Debug, Clone, Copy)]
pub struct FramebufferInfo {
    /// Virtual address of the framebuffer (HHDM-mapped).
    pub address: VirtAddr,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bytes per scanline.
    pub pitch: u32,
    /// Bits per pixel.
    pub bpp: u8,
    /// Pixel format.
    pub pixel_format: PixelFormat,
}

/// Physical and virtual base addresses of the loaded kernel image.
#[derive(Debug, Clone, Copy)]
pub struct KernelAddressInfo {
    /// Physical base address of the kernel.
    pub physical_base: PhysAddr,
    /// Virtual base address of the kernel.
    pub virtual_base: VirtAddr,
}

/// The paging mode configured by the bootloader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PagingMode {
    /// 4-level paging (48-bit virtual address space).
    #[cfg(target_arch = "x86_64")]
    Level4,
    /// 5-level paging with LA57 (57-bit virtual address space).
    #[cfg(target_arch = "x86_64")]
    Level5,

    /// 4-level paging (48-bit virtual address space).
    #[cfg(target_arch = "aarch64")]
    Level4,
    /// 5-level paging (52-bit virtual address space).
    #[cfg(target_arch = "aarch64")]
    Level5,
}

/// Information about the initial ramdisk loaded by the bootloader.
#[derive(Debug, Clone, Copy)]
pub struct InitrdInfo {
    /// Physical address of the initrd in memory.
    pub phys_addr: PhysAddr,
    /// Size of the initrd in bytes.
    pub size: u64,
}

/// Information about the backtrace data module loaded by the bootloader.
#[derive(Debug, Clone, Copy)]
pub struct BacktraceInfo {
    /// Physical address of the HBTF data in memory.
    pub phys_addr: PhysAddr,
    /// Size of the HBTF data in bytes.
    pub size: u64,
}

/// Maximum number of memory regions the kernel can store.
pub const MAX_MEMORY_REGIONS: usize = 256;

/// Maximum number of framebuffers the kernel can store.
pub const MAX_FRAMEBUFFERS: usize = 4;

/// Maximum number of SMP CPUs the boot info can describe.
///
/// Kept small to avoid stack overflow when `BootInfoData` is constructed
/// on the stack during early boot or in test harnesses.
pub const MAX_SMP_CPUS: usize = 32;

/// Information about a single CPU for SMP bootstrap.
///
/// The `goto_address_ptr` and `extra_argument_ptr` fields point to
/// bootloader-owned memory. Writing the entry function address to
/// `goto_address_ptr` (after writing `extra_argument_ptr`) atomically
/// starts the AP.
#[derive(Debug, Clone, Copy)]
pub struct SmpCpuEntry {
    /// Bootloader-assigned processor ID.
    pub processor_id: u32,
    /// Local APIC ID.
    pub lapic_id: u32,
    /// Pointer to the goto_address field in bootloader-owned memory.
    pub goto_address_ptr: *mut u64,
    /// Pointer to the extra_argument field in bootloader-owned memory.
    pub extra_argument_ptr: *mut u64,
}

// SAFETY: The pointers reference bootloader-owned memory that is accessible
// from any CPU via the HHDM mapping.
unsafe impl Send for SmpCpuEntry {}
unsafe impl Sync for SmpCpuEntry {}

impl SmpCpuEntry {
    /// Starts this AP by writing the extra argument and then the entry address.
    ///
    /// # Safety
    ///
    /// - `entry` must be the address of a valid `extern "C" fn(u64, u64) -> !`.
    /// - `extra` is passed in RSI to the entry function.
    /// - The pointed-to bootloader memory must still be valid and mapped.
    pub unsafe fn start(&self, entry: usize, extra: u64) {
        use core::sync::atomic::{Ordering, fence};
        // SAFETY: Caller guarantees the pointers are still valid.
        unsafe {
            core::ptr::write_volatile(self.extra_argument_ptr, extra);
            fence(Ordering::Release);
            core::ptr::write_volatile(self.goto_address_ptr, entry as u64);
        }
    }
}

/// Bootloader-agnostic boot information.
///
/// Each bootloader stub (Limine, UEFI, etc.) implements this trait by converting
/// its native data structures into the kernel's canonical types before calling
/// [`kernel_init`].
pub trait BootInfo {
    /// Physical memory map, sorted by start address.
    fn memory_map(&self) -> &[MemoryRegion];

    /// HHDM offset: `virtual = physical + hhdm_offset()`.
    fn hhdm_offset(&self) -> u64;

    /// Kernel load addresses (physical and virtual base).
    fn kernel_address(&self) -> KernelAddressInfo;

    /// Active paging mode configured by the bootloader.
    fn paging_mode(&self) -> PagingMode;

    /// All available framebuffers.
    fn framebuffers(&self) -> &[FramebufferInfo];

    /// ACPI RSDP physical address, if available.
    fn rsdp_address(&self) -> Option<PhysAddr>;

    /// Device Tree Blob physical address, if available.
    fn dtb_address(&self) -> Option<PhysAddr>;

    /// Kernel command line, if any.
    fn command_line(&self) -> Option<&str>;

    /// SMBIOS entry point addresses: (32-bit, 64-bit). Either may be `None`.
    fn smbios_address(&self) -> (Option<PhysAddr>, Option<PhysAddr>);

    /// Physical address of the root page table (PML4 on x86_64, TTBR1 value on aarch64).
    fn page_table_root(&self) -> PhysAddr;

    /// Initial ramdisk (CPIO archive), if loaded by the bootloader.
    fn initrd(&self) -> Option<InitrdInfo>;

    /// Backtrace data (HBTF format), if loaded by the bootloader.
    fn backtrace(&self) -> Option<BacktraceInfo>;

    /// SMP CPU entries for AP bootstrap. Empty if single-processor.
    fn smp_cpus(&self) -> &[SmpCpuEntry];

    /// BSP Local APIC ID (x86_64).
    fn bsp_lapic_id(&self) -> u32;
}

/// A concrete container for boot information, populated by a bootloader stub.
pub struct BootInfoData {
    /// Physical memory map.
    pub memory_map: ArrayVec<MemoryRegion, MAX_MEMORY_REGIONS>,
    /// HHDM offset.
    pub hhdm_offset: u64,
    /// Kernel load addresses.
    pub kernel_address: KernelAddressInfo,
    /// Active paging mode.
    pub paging_mode: PagingMode,
    /// Available framebuffers.
    pub framebuffers: ArrayVec<FramebufferInfo, MAX_FRAMEBUFFERS>,
    /// ACPI RSDP physical address.
    pub rsdp_address: Option<PhysAddr>,
    /// DTB physical address.
    pub dtb_address: Option<PhysAddr>,
    /// Kernel command line.
    pub command_line: Option<&'static str>,
    /// SMBIOS 32-bit entry point address.
    pub smbios_32: Option<PhysAddr>,
    /// SMBIOS 64-bit entry point address.
    pub smbios_64: Option<PhysAddr>,
    /// Physical address of the root page table (PML4 on x86_64, TTBR1 value on aarch64).
    pub page_table_root: PhysAddr,
    /// Initial ramdisk information, if loaded by the bootloader.
    pub initrd: Option<InitrdInfo>,
    /// Backtrace data (HBTF format), if loaded by the bootloader.
    pub backtrace: Option<BacktraceInfo>,
    /// SMP CPU entries for AP bootstrap.
    pub smp_cpus: ArrayVec<SmpCpuEntry, MAX_SMP_CPUS>,
    /// BSP Local APIC ID.
    pub bsp_lapic_id: u32,
}

impl BootInfo for BootInfoData {
    fn memory_map(&self) -> &[MemoryRegion] {
        self.memory_map.as_slice()
    }

    fn hhdm_offset(&self) -> u64 {
        self.hhdm_offset
    }

    fn kernel_address(&self) -> KernelAddressInfo {
        self.kernel_address
    }

    fn paging_mode(&self) -> PagingMode {
        self.paging_mode
    }

    fn framebuffers(&self) -> &[FramebufferInfo] {
        self.framebuffers.as_slice()
    }

    fn rsdp_address(&self) -> Option<PhysAddr> {
        self.rsdp_address
    }

    fn dtb_address(&self) -> Option<PhysAddr> {
        self.dtb_address
    }

    fn command_line(&self) -> Option<&str> {
        self.command_line
    }

    fn smbios_address(&self) -> (Option<PhysAddr>, Option<PhysAddr>) {
        (self.smbios_32, self.smbios_64)
    }

    fn page_table_root(&self) -> PhysAddr {
        self.page_table_root
    }

    fn initrd(&self) -> Option<InitrdInfo> {
        self.initrd
    }

    fn backtrace(&self) -> Option<BacktraceInfo> {
        self.backtrace
    }

    fn smp_cpus(&self) -> &[SmpCpuEntry] {
        self.smp_cpus.as_slice()
    }

    fn bsp_lapic_id(&self) -> u32 {
        self.bsp_lapic_id
    }
}

/// Kernel entry point, called by every bootloader stub.
///
/// The boot stub constructs a [`BootInfo`] implementation from its native data,
/// then calls this function. Static dispatch ensures zero overhead.
pub fn kernel_init(boot_info: &impl BootInfo) -> ! {
    // 1. Arch-specific CPU init.
    crate::arch::cpu_init();

    // 2. Initialize HHDM global offset.
    crate::mm::hhdm::init(boot_info.hhdm_offset());
    crate::kinfo!("HHDM initialized at offset {:#x}", boot_info.hhdm_offset());

    // 2b. Initialize backtrace support (must be after HHDM so we can access the module data).
    if let Some(bt) = boot_info.backtrace() {
        let virt = crate::mm::hhdm::phys_to_virt(bt.phys_addr);
        // SAFETY: The bootloader loaded the HBTF data into contiguous physical memory
        // covered by the HHDM. The slice remains valid for the kernel's lifetime
        // because the module memory region is marked KernelAndModules and is never
        // reclaimed.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "x86_64: u64 and usize are the same width"
        )]
        let data =
            unsafe { core::slice::from_raw_parts(virt.as_u64() as *const u8, bt.size as usize) };
        crate::backtrace::init(data, boot_info.kernel_address().virtual_base.as_u64());
    }

    // 3. Initialize PMM (bitmap from memory map).
    crate::mm::pmm::init(boot_info);
    crate::mm::pmm::with_pmm(|pmm| {
        let free = pmm.free_frames();
        let total = pmm.total_frames();
        crate::kinfo!(
            "PMM: {} MiB free / {} MiB total",
            free * 4 / 1024,
            total * 4 / 1024
        );
        crate::kdebug!("PMM: {} free frames", free);
    });

    // 4. Initialize VMM (wraps root page table, creates memory layout).
    crate::mm::vmm::init(boot_info);

    // 4b. Allocate a guarded kernel syscall stack (replaces the early BSS stack).
    {
        use crate::mm::pmm::BitmapFrameAllocRef;
        crate::mm::pmm::with_pmm(|pmm| {
            let mut alloc = BitmapFrameAllocRef(pmm);
            crate::mm::vmm::with_vmm(|vmm| {
                let stack = vmm
                    .alloc_kernel_stack(&mut alloc, None)
                    .expect("failed to allocate guarded kernel stack");
                crate::kinfo!(
                    "Guarded kernel stack: {:#x}..{:#x} (guard at {:#x})",
                    stack.bottom().as_u64(),
                    stack.top().as_u64(),
                    stack.guard().as_u64(),
                );
                // SAFETY: The stack was just allocated and mapped. Setting
                // kernel_rsp and RSP0 to its top is safe because no
                // syscall or interrupt will use the old stack between
                // these two stores (interrupts are still disabled).
                unsafe {
                    crate::percpu::set_kernel_rsp(stack.top().as_u64());
                    crate::arch::x86_64::gdt::set_tss_rsp0(stack.top().as_u64());
                }
            });
        });
    }

    // 5. Map initial heap and initialize the heap allocator.
    crate::mm::heap::init();
    crate::kinfo!("Heap allocator initialized");

    // 5b. Initialize device registry (before driver probing).
    crate::drivers::device_registry::init();

    // 6. Initialize the full logger (replaces early serial functions).
    crate::log::init_logger();

    // 7. Register framebuffer sink if available.
    if let Some(fb_info) = boot_info.framebuffers().first() {
        if let Some(early_fb) = crate::drivers::early_fb::EarlyFramebuffer::new(fb_info) {
            crate::log::add_sink(Box::new(crate::log::FramebufferSink::new(
                early_fb,
                crate::log::LogLevel::Info,
            )));
        }
    }

    // 8. Arch-specific platform init (ACPI, PCI, drivers, etc.).
    crate::arch::platform_init(boot_info);

    // 9. Switch framebuffer sink to a device-registry framebuffer if one was
    //    registered during driver probing (e.g., Bochs VGA). The early FB sink
    //    wrote to the same physical framebuffer (via HHDM) but the driver may
    //    have re-initialized it, so we re-zero and reset the cursor.
    #[cfg(target_arch = "x86_64")]
    if let Some(fb) =
        crate::drivers::device_registry::with_device_registry(|dr| dr.take_framebuffer("bochs-vga"))
    {
        let info = fb.info();
        let total = info.pitch as usize * info.height as usize;
        // SAFETY: Entire framebuffer is within the mapped MMIO region.
        unsafe { fb.fill_zero(0, total) };

        // Reset cursor so the new sink starts at the top-left corner.
        {
            let mut cursor = crate::drivers::early_fb::CURSOR.lock();
            cursor.col = 0;
            cursor.row = 0;
        }
        let dev_fb_sink = Box::new(crate::log::DeviceFramebufferSink::new(
            fb,
            crate::log::LogLevel::Info,
        ));
        if crate::log::replace_sink_by_name("framebuffer", dev_fb_sink) {
            crate::kinfo!("Switched display to device framebuffer");
        }
    }

    crate::kinfo!("Hadron kernel initialized successfully.");

    // 8b. Initialize cross-CPU wakeup IPI, then boot Application Processors.
    crate::sched::smp::init();
    #[cfg(target_arch = "x86_64")]
    crate::arch::x86_64::smp::boot_aps(boot_info);

    // 9. Spawn platform tasks + heartbeat.
    crate::arch::spawn_platform_tasks();

    crate::sched::spawn_background("heartbeat", async {
        let mut n = 0u64;
        loop {
            crate::sched::primitives::sleep_ms(5000).await;
            n += 1;
            crate::kdebug!("[heartbeat] {}s elapsed", n * 5);
        }
    });

    // 10. Extract initrd data via HHDM.
    let initrd_info = boot_info.initrd().expect("no initrd loaded by bootloader");
    let initrd_data = {
        let virt = crate::mm::hhdm::phys_to_virt(initrd_info.phys_addr);
        // SAFETY: The bootloader loaded the initrd into contiguous physical memory
        // covered by the HHDM. The slice remains valid for the kernel's lifetime
        // because the initrd memory region is marked KernelAndModules and is never
        // reclaimed.
        #[expect(
            clippy::cast_possible_truncation,
            reason = "x86_64: u64 and usize are the same width"
        )]
        unsafe {
            core::slice::from_raw_parts(virt.as_u64() as *const u8, initrd_info.size as usize)
        }
    };
    crate::kinfo!(
        "Initrd loaded: {} bytes at {}",
        initrd_info.size,
        initrd_info.phys_addr
    );

    // 10b. Initialize VFS and mount filesystems.
    {
        use crate::fs::{self, FileSystem};
        use alloc::sync::Arc;

        fs::vfs::init();

        // Discover and mount the root virtual filesystem (ramfs) from the
        // driver registry. The ramfs virtual_fs_entry is in hadron-drivers.
        let ramfs = {
            let entries = crate::drivers::registry::virtual_fs_entries();
            let ramfs_entry = entries
                .iter()
                .find(|e| e.name == "ramfs")
                .expect("no ramfs virtual_fs_entry registered");
            (ramfs_entry.create)()
        };
        let ramfs_root = ramfs.root();
        fs::vfs::with_vfs_mut(|vfs| vfs.mount("/", ramfs));

        // Unpack initrd CPIO archive into the root filesystem using the
        // registered initramfs unpacker.
        {
            let entries = crate::drivers::registry::initramfs_entries();
            if let Some(entry) = entries.first() {
                let file_count = (entry.unpack)(initrd_data, &ramfs_root);
                crate::kinfo!("Initramfs ({}): Unpacked {} files", entry.name, file_count);
            } else {
                crate::kwarn!("No initramfs unpacker registered");
            }
        }

        // Mount devfs at /dev (kernel-internal, not from driver registry).
        let devfs = Arc::new(fs::devfs::DevFs::new());
        fs::vfs::with_vfs_mut(|vfs| vfs.mount("/dev", devfs));

        // Mount block-device-backed filesystems discovered from the registry.
        // Each block device is passed to registered block FS drivers until one succeeds.
        let block_fs_entries = crate::drivers::registry::block_fs_entries();

        // VirtIO block → try registered block FS drivers at /mnt.
        if let Some(disk) = crate::drivers::device_registry::with_device_registry_mut(|dr| {
            dr.take_block_device("virtio-blk-0")
        }) {
            mount_block_device(disk, "/mnt", block_fs_entries, "virtio-blk-0");
        }

        // AHCI block → try registered block FS drivers at /cdrom.
        if let Some(disk) = crate::drivers::device_registry::with_device_registry_mut(|dr| {
            dr.take_block_device("ahci-0")
        }) {
            mount_block_device(disk, "/cdrom", block_fs_entries, "ahci-0");
        }
    }

    // Initialize IRQ-driven keyboard input for /dev/console reads.
    crate::fs::console_input::init();

    crate::proc::save_kernel_cr3();

    // Populate BSP per-CPU pointers for assembly stubs (timer, syscall).
    // These pointers let the naked ASM access per-CPU CpuLocal elements
    // via GS:[offset] instead of RIP-relative addressing.
    {
        let percpu = crate::percpu::current_cpu();
        // SAFETY: We have exclusive BSP access during init. The pointers
        // are to static CpuLocal elements that live forever.
        unsafe {
            let percpu_mut =
                percpu as *const crate::percpu::PerCpu as *mut crate::percpu::PerCpu;
            (*percpu_mut).user_context_ptr = crate::proc::user_context_ptr() as u64;
            (*percpu_mut).saved_kernel_rsp_ptr = crate::proc::saved_kernel_rsp_ptr() as u64;
            (*percpu_mut).trap_reason_ptr = crate::proc::trap_reason_ptr() as u64;
            (*percpu_mut).saved_regs_ptr = crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
                .get()
                .get() as u64;
        }
    }

    crate::proc::spawn_init();

    // 11. Enable BSP interrupts now that all init is done and APs are online.
    // SAFETY: IDT, LAPIC, I/O APIC, per-CPU state, and SMP are all initialized.
    unsafe { crate::arch::x86_64::instructions::interrupts::enable() };
    crate::kinfo!("BSP interrupts enabled");

    // 12. Run the executor — drives ALL kernel tasks including the process task.
    crate::sched::executor().run();
}

/// Try to mount a block device at the given mount point using registered FS drivers.
///
/// Iterates block FS entries from the linker section, passing the device to each
/// mount function until one succeeds. The block device is consumed on first attempt
/// (success or failure) since the mount function takes ownership.
#[cfg(target_os = "none")]
fn mount_block_device(
    disk: alloc::boxed::Box<dyn crate::driver_api::dyn_dispatch::DynBlockDevice>,
    mount_point: &str,
    block_fs_entries: &[crate::driver_api::registration::BlockFsEntry],
    device_name: &str,
) {
    for entry in block_fs_entries {
        match (entry.mount)(disk) {
            Ok(fs_instance) => {
                crate::fs::vfs::with_vfs_mut(|vfs| vfs.mount(mount_point, fs_instance));
                return;
            }
            Err(e) => {
                crate::kinfo!(
                    "FS '{}' failed to mount {}: {:?}",
                    entry.name,
                    device_name,
                    e
                );
                // Block device consumed by mount attempt; cannot retry.
                return;
            }
        }
    }
    crate::kinfo!("No filesystem driver for {}", device_name);
}
