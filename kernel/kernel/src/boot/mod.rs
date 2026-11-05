//! The kernel's entry point, [`kernel_main`], and [`BootInfo`], what a
//! bootloader hands the kernel.
//!
//! Each bootloader has an adapter module, so far only [`limine`]. The
//! adapter owns the entry symbol the bootloader jumps to, and it calls
//! [`kernel_main`] with a function that reads the bootloader's data into a
//! [`BootInfo`]. Nothing past the adapter knows which bootloader ran.

pub mod limine;

use core::ptr::{self, NonNull};
use core::sync::atomic::{AtomicPtr, Ordering};

use arch_x86_64::paging::PAGE_SIZE;
use arch_x86_64::{PhysAddr, VirtAddr};
use mm::{DirectMap, Region};

use crate::arch::x86_64::qemu::{self, ExitCode};
use crate::arch::x86_64::{cpu, gdt, idt, pic};
use crate::console::{self, kprintln};
use crate::sync::{Ctx, LockBefore, levels};
use crate::{heap, memory, percpu, sched};

/// CR0.WP, bit 16: supervisor writes honour read-only pages.
const CR0_WP: u64 = 1 << 16;
/// CR0.PG, bit 31: paging.
const CR0_PG: u64 = 1 << 31;
/// CR4.PAE, bit 5: physical address extension, which long mode needs.
const CR4_PAE: u64 = 1 << 5;
/// CR4.LA57, bit 12: 5-level paging.
const CR4_LA57: u64 = 1 << 12;
/// EFER.LMA, bit 10: long mode is active.
const EFER_LMA: u64 = 1 << 10;
/// EFER.NXE, bit 11: the no-execute page bit.
const EFER_NXE: u64 = 1 << 11;

unsafe extern "C" {
    /// The kernel's link address, from `kernel.ld`.
    static __kernel_start: u8;
}

/// What the main thread runs, set once by [`kernel_main`]. A static
/// carries it because [`memory::init`] continues in a plain function.
static MAIN: AtomicPtr<()> = AtomicPtr::new(ptr::null_mut());

/// The kernel's entry point, for every bootloader.
///
/// It sets up the boot CPU's per-CPU block and the console, checks the CPU
/// state listed below, and loads the GDT and IDT. Then it calls
/// `boot_info`, so the adapter reads the bootloader's data with the
/// console up and a panic handler that can report what went wrong. Last,
/// it switches to the kernel's page tables and starts the scheduler.
/// The main thread runs `main`, then exits QEMU with success.
///
/// # Safety
///
/// - It runs once, on the boot CPU, while no other CPU runs kernel code.
/// - The CPU is in 64-bit mode with 4-level paging, CR0.WP and EFER.NXE
///   set, and maskable interrupts masked. `kernel_main` checks each of
///   these and panics if one does not hold.
/// - The active page tables map the kernel image at its link address.
/// - The stack has at least 64 KiB free. Boot runs on it until
///   [`memory::init`] switches to the kernel's boot stack.
/// - `boot_info` upholds what [`BootInfo::new`] requires.
pub(crate) unsafe fn kernel_main(boot_info: impl FnOnce() -> BootInfo, main: fn()) -> ! {
    MAIN.store(main as *mut (), Ordering::Relaxed);
    // Locks mask interrupts through the per-CPU block, so it comes first.
    percpu::init_boot_cpu();
    // With no console there is nowhere to report the error, so boot goes on.
    let _ = console::init();
    check_cpu_state();
    gdt::init();
    idt::init();

    let info = boot_info();
    check_boot_info(&info);
    kprintln!(
        "hadron: booted by {} {}",
        info.loader_name(),
        info.loader_version()
    );
    memory::init(info, kmain_paged)
}

/// Panics unless the CPU is in the state [`kernel_main`] requires.
fn check_cpu_state() {
    assert!(
        !cpu::interrupts_enabled(),
        "interrupts are masked at kernel entry"
    );
    let cr0 = cpu::cr0();
    assert!(cr0 & CR0_PG != 0, "paging is on at kernel entry");
    assert!(cr0 & CR0_WP != 0, "CR0.WP is set at kernel entry");
    let cr4 = cpu::cr4();
    assert!(
        cr4 & CR4_PAE != 0 && cr4 & CR4_LA57 == 0,
        "4-level paging is active at kernel entry"
    );
    let efer = cpu::efer();
    assert!(efer & EFER_LMA != 0, "long mode is active at kernel entry");
    assert!(efer & EFER_NXE != 0, "EFER.NXE is set at kernel entry");
}

/// Panics unless `info` passes the checks that are cheap to make.
fn check_boot_info(info: &BootInfo) {
    let link = VirtAddr::from_ptr(&raw const __kernel_start);
    assert_eq!(
        info.kernel_virt(),
        link,
        "the kernel is loaded at its link address"
    );
    assert!(
        info.kernel_phys().is_aligned(PAGE_SIZE),
        "the kernel is loaded on a page boundary"
    );
    assert!(
        info.memory_map().iter().all(|r| r.start <= r.end),
        "memory map regions end after they start"
    );
    assert!(
        info.memory_map()
            .windows(2)
            .all(|pair| pair[0].end <= pair[1].start),
        "the memory map is sorted and has no overlaps"
    );
}

/// Runs on the kernel's own page tables and boot stack, after
/// [`memory::init`]. Maps the heap, turns the boot context into the idle
/// thread, and starts the main thread.
fn kmain_paged(info: BootInfo) -> ! {
    let mut ctx = Ctx::root();
    heap::init(&mut ctx);
    sched::init();
    pic::mask_all();
    // SAFETY: the PICs are masked, and nothing else is set up to deliver
    // interrupts.
    unsafe { cpu::enable_interrupts() };
    // The main thread runs to the end of boot, so its handle is not needed.
    drop(sched::spawn(&mut ctx, "main", move |ctx| {
        main_thread(ctx, info);
    }));
    sched::run_idle(&mut ctx)
}

/// The first kernel thread: reports memory, gives back the bootloader's
/// memory, runs the `main` [`kernel_main`] got, and exits.
fn main_thread(ctx: &mut Ctx<'static, levels::ThreadRoot>, info: BootInfo) {
    const MIB: u64 = 1 << 20;
    let usable = memory::map().total(mm::RegionKind::Usable);
    let (free, count) = {
        let frames = memory::frames(ctx);
        (frames.free_frames(), frames.frame_count())
    };
    kprintln!(
        "hadron: {} MiB usable, {free} of {count} frames free",
        usable / MIB
    );
    kprintln!("hadron: {} KiB heap", heap::free_bytes(ctx) / 1024);
    let reclaimed = info.reclaim(ctx);
    let free = memory::frames(ctx).free_frames();
    kprintln!(
        "hadron: reclaimed {} KiB of bootloader memory and {} KiB of page tables, {free} frames free",
        reclaimed.boot * PAGE_SIZE / 1024,
        reclaimed.tables * PAGE_SIZE / 1024,
    );
    // SAFETY: `kernel_main` stored a `fn()` before boot went on, and
    // nothing else writes `MAIN`.
    let main = unsafe { core::mem::transmute::<*mut (), fn()>(MAIN.load(Ordering::Relaxed)) };
    main();
    qemu::exit(ExitCode::Success)
}

/// What the bootloader hands the kernel, in a form that does not depend
/// on the bootloader.
///
/// A `BootInfo` stands for the bootloader's memory. Some of what it points
/// at lives there, read through the bootloader's direct map. Every
/// reference it gives out borrows from it, and [`reclaim`](Self::reclaim)
/// takes it by value, so no such reference outlives the memory. Code that
/// keeps boot data copies it.
///
/// Dropping it without calling `reclaim` leaks the bootloader's memory,
/// which is safe.
pub(crate) struct BootInfo {
    /// The physical memory map, in the kernel's static storage.
    memory_map: &'static [Region],
    loader_direct_map: DirectMap,
    kernel_phys: PhysAddr,
    kernel_virt: VirtAddr,
    /// In bootloader memory.
    loader_name: NonNull<str>,
    /// In bootloader memory.
    loader_version: NonNull<str>,
}

// SAFETY: nothing writes the memory the pointers lead to after the
// bootloader hands over, so whichever thread holds the value may read it.
unsafe impl Send for BootInfo {}

impl BootInfo {
    /// Wraps what an adapter read from the bootloader.
    ///
    /// # Safety
    ///
    /// - No other `BootInfo` exists.
    /// - `memory_map` describes physical memory accurately. Usable regions
    ///   hold RAM that nothing else uses.
    /// - In the active page tables, `loader_direct_map` maps every region
    ///   of `memory_map` whose kind is
    ///   [`direct mapped`](mm::RegionKind::is_direct_mapped), read-write.
    /// - The kernel image lies at `kernel_phys`, physically contiguous,
    ///   and is mapped at `kernel_virt`.
    /// - `loader_name` and `loader_version` stay valid and unchanged while
    ///   `loader_direct_map` is mapped and bootloader-reclaimable memory
    ///   is in use.
    /// - After `new` returns, the caller reads no bootloader-reclaimable
    ///   memory, since [`reclaim`](Self::reclaim) frees it.
    pub(crate) unsafe fn new(
        memory_map: &'static [Region],
        loader_direct_map: DirectMap,
        kernel_phys: PhysAddr,
        kernel_virt: VirtAddr,
        loader_name: &str,
        loader_version: &str,
    ) -> Self {
        Self {
            memory_map,
            loader_direct_map,
            kernel_phys,
            kernel_virt,
            loader_name: NonNull::from(loader_name),
            loader_version: NonNull::from(loader_version),
        }
    }

    /// The physical memory map, sorted by address.
    pub(crate) fn memory_map(&self) -> &[Region] {
        self.memory_map
    }

    /// The bootloader's direct map of physical memory.
    pub(crate) fn loader_direct_map(&self) -> DirectMap {
        self.loader_direct_map
    }

    /// Where the kernel image starts in physical memory.
    pub(crate) fn kernel_phys(&self) -> PhysAddr {
        self.kernel_phys
    }

    /// Where the kernel image starts in virtual memory.
    pub(crate) fn kernel_virt(&self) -> VirtAddr {
        self.kernel_virt
    }

    /// The bootloader's name.
    pub(crate) fn loader_name(&self) -> &str {
        // SAFETY: `new` requires the string to stay valid while `self`
        // lives, and the borrow ends before `reclaim` frees it.
        unsafe { self.loader_name.as_ref() }
    }

    /// The bootloader's version.
    pub(crate) fn loader_version(&self) -> &str {
        // SAFETY: as in `loader_name`.
        unsafe { self.loader_version.as_ref() }
    }

    /// Gives the bootloader's memory back to the frame allocator, unmaps
    /// the bootloader's direct map, and frees the page tables that held
    /// it. Copy anything still needed first.
    pub(crate) fn reclaim<P: LockBefore<levels::KernelTables>>(
        self,
        ctx: &mut Ctx<'_, P>,
    ) -> memory::Reclaimed {
        // SAFETY: `memory::init` took this `BootInfo`. Taking `self` ends
        // every borrow from it, and `new` requires that nothing else reads
        // bootloader-reclaimable memory. The rest of the kernel reaches
        // memory through `memory::DIRECT_MAP`.
        unsafe { memory::reclaim_boot_memory(ctx, self.loader_direct_map, self.memory_map) }
    }
}
