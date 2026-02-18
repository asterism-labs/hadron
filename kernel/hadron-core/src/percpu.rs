//! Per-CPU state foundation (SMP-ready).
//!
//! Provides a minimal per-CPU data structure that holds CPU-local state
//! such as the APIC ID and LAPIC reference. Currently uses a single
//! static instance for the BSP; designed to be replaced with GS-base
//! indexing when APs are booted.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

/// Syscall stack size for early boot (16 KiB).
///
/// This static BSS stack is used during early boot before the VMM is
/// available. Once the VMM is initialized, `kernel_init` should allocate
/// a proper guarded kernel stack via `vmm.alloc_kernel_stack()` and call
/// `set_kernel_rsp()` to switch to it. The VMM-allocated stack includes
/// a guard page at its base that triggers a page fault on overflow,
/// preventing silent corruption of `BSP_PERCPU` or other BSS data.
const EARLY_SYSCALL_STACK_SIZE: usize = 16384;

/// Aligned stack for early-boot syscall use.
#[repr(align(16))]
struct AlignedStack(
    #[allow(dead_code, reason = "backing storage accessed by assembly")]
    [u8; EARLY_SYSCALL_STACK_SIZE],
);

/// Dedicated stack for the syscall entry path (early boot only).
///
/// Must be `static mut` so the linker places it in `.bss` (writable),
/// not `.rodata` (read-only). The assembly entry stub writes to this stack.
///
/// **Warning**: This stack has no guard page. Stack overflow will silently
/// corrupt adjacent BSS data. After VMM init, replace with a guarded stack
/// by calling `set_kernel_rsp()` with the top of a VMM-allocated stack.
static mut SYSCALL_STACK: AlignedStack = AlignedStack([0; EARLY_SYSCALL_STACK_SIZE]);

/// Per-CPU data structure.
///
/// Holds CPU-local state. `#[repr(C)]` ensures deterministic field offsets
/// for inline assembly access via GS-base:
/// - offset 0: `kernel_rsp`
/// - offset 8: `user_rsp`
///
/// For now, a single static instance is used for the BSP. In future phases,
/// each AP will have its own instance accessed via GS-base.
#[repr(C)]
pub struct PerCpu {
    /// Saved kernel RSP for syscall stack switching (offset 0).
    pub kernel_rsp: u64,
    /// Saved user RSP during syscall handling (offset 8).
    pub user_rsp: u64,
    /// Logical CPU ID (0 for BSP).
    pub cpu_id: AtomicU32,
    /// Local APIC ID.
    pub apic_id: AtomicU8,
    /// Whether this per-CPU instance has been initialized.
    initialized: AtomicBool,
}

impl PerCpu {
    /// Creates a new uninitialized `PerCpu`.
    const fn new() -> Self {
        Self {
            kernel_rsp: 0,
            user_rsp: 0,
            cpu_id: AtomicU32::new(0),
            apic_id: AtomicU8::new(0),
            initialized: AtomicBool::new(false),
        }
    }

    /// Initializes this per-CPU instance.
    pub fn init(&self, cpu_id: u32, apic_id: u8) {
        self.cpu_id.store(cpu_id, Ordering::Relaxed);
        self.apic_id.store(apic_id, Ordering::Relaxed);
        self.initialized.store(true, Ordering::Release);
    }

    /// Returns the CPU ID.
    pub fn get_cpu_id(&self) -> u32 {
        self.cpu_id.load(Ordering::Relaxed)
    }

    /// Returns the APIC ID.
    pub fn get_apic_id(&self) -> u8 {
        self.apic_id.load(Ordering::Relaxed)
    }

    /// Returns whether this instance has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::Acquire)
    }
}

/// BSP per-CPU data (single static instance for now).
static mut BSP_PERCPU: PerCpu = PerCpu::new();

/// Returns a reference to the current CPU's per-CPU data.
///
/// Currently always returns the BSP instance. When SMP is implemented,
/// this will use GS-base to index per-CPU storage.
pub fn current_cpu() -> &'static PerCpu {
    // SAFETY: BSP_PERCPU is only mutated during early init (single-threaded),
    // and all subsequent accesses are read-only or via atomic fields.
    unsafe { &*core::ptr::addr_of!(BSP_PERCPU) }
}

/// Initializes GS-base MSRs to point to the BSP per-CPU data.
///
/// Sets both `IA32_GS_BASE` and `IA32_KERNEL_GS_BASE` to `&BSP_PERCPU`.
/// This means `swapgs` in the syscall path is a safe no-op when called
/// from ring 0 (both MSRs point to the same address).
///
/// Also initializes `kernel_rsp` to the top of the dedicated syscall stack.
///
/// # Safety
///
/// Must be called after GDT init and before any syscall can be triggered.
#[cfg(target_arch = "x86_64")]
pub unsafe fn init_gs_base() {
    use crate::arch::x86_64::registers::model_specific::{IA32_GS_BASE, IA32_KERNEL_GS_BASE};

    let percpu_addr = core::ptr::addr_of!(BSP_PERCPU) as u64;
    let stack_top = core::ptr::addr_of!(SYSCALL_STACK) as u64 + EARLY_SYSCALL_STACK_SIZE as u64;

    // SAFETY: BSP_PERCPU is a module-level static; addr_of_mut! is valid.
    // Writing kernel_rsp before any syscall can fire is the caller's
    // requirement (guaranteed by the # Safety contract). Writing both
    // GS_BASE and KERNEL_GS_BASE to the same address means swapgs is a
    // no-op from ring 0, which is correct before any user process exists.
    unsafe {
        let percpu_ptr = core::ptr::addr_of_mut!(BSP_PERCPU);
        (*percpu_ptr).kernel_rsp = stack_top;

        IA32_GS_BASE.write(percpu_addr);
        IA32_KERNEL_GS_BASE.write(percpu_addr);
    }

    crate::kdebug!(
        "GS base initialized: percpu={:#x}, kernel_rsp={:#x}",
        percpu_addr,
        stack_top
    );
}

/// Maximum supported CPUs. BSP-only for now.
pub const MAX_CPUS: usize = 1;

/// CPU-local storage. Wraps `[T; MAX_CPUS]`, indexed by current CPU ID.
///
/// When SMP is enabled (Phase 14), increase `MAX_CPUS` and each AP
/// gets its own instance.
pub struct CpuLocal<T> {
    data: [T; MAX_CPUS],
}

impl<T> CpuLocal<T> {
    /// Creates a new `CpuLocal` wrapping the given array.
    pub const fn new(data: [T; MAX_CPUS]) -> Self {
        Self { data }
    }

    /// Returns a reference to the current CPU's instance.
    pub fn get(&self) -> &T {
        &self.data[current_cpu().get_cpu_id() as usize]
    }
}

// SAFETY: CpuLocal<T> is designed for per-CPU access. Send/Sync are safe
// because each CPU only accesses its own slot.
unsafe impl<T: Send> Send for CpuLocal<T> {}
unsafe impl<T: Send> Sync for CpuLocal<T> {}

/// Returns the early-boot kernel RSP (top of BSS syscall stack).
/// Used during TSS initialization before the guarded stack is allocated.
pub fn early_kernel_rsp() -> u64 {
    core::ptr::addr_of!(SYSCALL_STACK) as u64 + EARLY_SYSCALL_STACK_SIZE as u64
}

/// Updates the stored kernel RSP in the current per-CPU data.
///
/// # Safety
///
/// Must only be called when it is safe to change the syscall return stack.
pub unsafe fn set_kernel_rsp(rsp: u64) {
    // SAFETY: BSP_PERCPU is a module-level static; addr_of_mut! is valid.
    // The caller guarantees it is safe to change the syscall return stack.
    unsafe {
        let percpu_ptr = core::ptr::addr_of_mut!(BSP_PERCPU);
        (*percpu_ptr).kernel_rsp = rsp;
    }
}
