//! GDT initialization, static instance, and TSS setup.

use core::cell::UnsafeCell;

use hadron_core::arch::x86_64::structures::gdt::{
    Descriptor, GlobalDescriptorTable, SegmentSelector, TaskStateSegment,
};

use crate::sync::LazyLock;

/// Double-fault handler stack size (16 KiB).
const DOUBLE_FAULT_STACK_SIZE: usize = 16384;

/// IST index used for the double-fault handler (IST1, 1-indexed).
pub const DOUBLE_FAULT_IST_INDEX: u8 = 1;

/// Dedicated stack for the double-fault handler.
#[repr(align(16))]
#[allow(dead_code)] // Used only for its address; the array itself backs the double-fault stack.
struct AlignedStack([u8; DOUBLE_FAULT_STACK_SIZE]);

static DOUBLE_FAULT_STACK: AlignedStack = AlignedStack([0; DOUBLE_FAULT_STACK_SIZE]);

/// Wrapper around `UnsafeCell<T>` that is `Sync`.
///
/// The TSS is only mutated by `set_tss_rsp0` with interrupts disabled, so
/// there is no data race from software. The CPU reads it on ring transitions
/// but does not race with writes between interrupt entry and IRET.
#[repr(transparent)]
struct SyncUnsafeCell<T>(UnsafeCell<T>);

// SAFETY: Access is synchronized by disabling interrupts before mutation.
// Only `set_tss_rsp0` writes to the inner value, and it runs with interrupts
// disabled during context switches.
unsafe impl<T> Sync for SyncUnsafeCell<T> {}

impl<T> SyncUnsafeCell<T> {
    const fn new(val: T) -> Self {
        Self(UnsafeCell::new(val))
    }

    fn get(&self) -> *mut T {
        self.0.get()
    }
}

/// Static Task State Segment, wrapped in `SyncUnsafeCell` to allow mutation of
/// RSP0 during context switches without UB (the CPU reads the TSS directly
/// from memory).
static TSS: LazyLock<SyncUnsafeCell<TaskStateSegment>> = LazyLock::new(|| {
    let mut tss = TaskStateSegment::new();
    // IST entries are 1-indexed in the IDT but 0-indexed in the TSS array.
    tss.interrupt_stack_table[(DOUBLE_FAULT_IST_INDEX - 1) as usize] = {
        let stack_start = &DOUBLE_FAULT_STACK as *const _ as u64;
        stack_start + DOUBLE_FAULT_STACK_SIZE as u64
    };
    // Set RSP0 to early BSS stack (same as percpu.kernel_rsp during early boot).
    tss.privilege_stack_table[0] = hadron_core::percpu::early_kernel_rsp();
    SyncUnsafeCell::new(tss)
});

/// Cached segment selectors from GDT initialization.
pub struct Selectors {
    /// Kernel code segment selector.
    pub kernel_code: SegmentSelector,
    /// Kernel data segment selector.
    pub kernel_data: SegmentSelector,
    /// User code segment selector.
    pub user_code: SegmentSelector,
    /// User data segment selector.
    pub user_data: SegmentSelector,
    /// TSS selector.
    pub tss: SegmentSelector,
}

/// Static GDT and its selectors.
static GDT: LazyLock<(GlobalDescriptorTable, Selectors)> = LazyLock::new(|| {
    let mut gdt = GlobalDescriptorTable::new();
    let kernel_code = gdt.append(Descriptor::kernel_code_segment());
    let kernel_data = gdt.append(Descriptor::kernel_data_segment());
    // user_data before user_code: SYSRET requires SS at STAR[63:48]+8, CS at STAR[63:48]+16
    let user_data = gdt.append(Descriptor::user_data_segment());
    let user_code = gdt.append(Descriptor::user_code_segment());
    // SAFETY: The TSS is fully initialized by the LazyLock closure above.
    // We take a shared reference for the descriptor, which only reads the address.
    let tss = gdt.append(Descriptor::tss_segment(unsafe { &*TSS.get() }));
    let selectors = Selectors {
        kernel_code,
        kernel_data,
        user_code,
        user_data,
        tss,
    };
    (gdt, selectors)
});

/// Initializes the GDT, reloads all segment registers, and loads the TSS.
///
/// # Safety
///
/// Must be called exactly once during early kernel initialization.
pub unsafe fn init() {
    use hadron_core::arch::x86_64::instructions::segmentation::{
        load_ds, load_es, load_fs, load_gs, load_ss, load_tss, set_cs,
    };

    let (gdt, selectors) = &*GDT;

    // SAFETY: The GDT contains valid descriptors built above. Segment register
    // reloads match the GDT layout (kernel_code in CS, kernel_data in DS/SS,
    // null in ES/FS/GS, TSS in TR).
    unsafe {
        gdt.load();
        set_cs(selectors.kernel_code);
        load_ds(selectors.kernel_data);
        load_ss(selectors.kernel_data);
        load_es(SegmentSelector::new(0, 0));
        load_fs(SegmentSelector::new(0, 0));
        load_gs(SegmentSelector::new(0, 0));
        load_tss(selectors.tss);
    }

    hadron_core::kdebug!("GDT initialized");
}

/// Updates RSP0 in the TSS (ring 3 → ring 0 stack pointer).
///
/// The CPU reads this value from memory on every interrupt/exception
/// from ring 3, so writing to it takes effect immediately (no TR reload needed).
///
/// # Safety
///
/// `rsp` must point to the top of a valid, mapped kernel stack.
pub unsafe fn set_tss_rsp0(rsp: u64) {
    // SAFETY: The TSS is wrapped in UnsafeCell specifically to allow this
    // mutation. This is called with interrupts disabled during context
    // switches, so there is no concurrent access from software. The CPU
    // hardware reads the TSS on privilege transitions but does not race
    // with this write (it occurs between interrupt entry and IRET).
    unsafe {
        (*TSS.get()).privilege_stack_table[0] = rsp;
    }
}

/// Returns a reference to the cached segment selectors.
pub fn selectors() -> &'static Selectors {
    &GDT.1
}
