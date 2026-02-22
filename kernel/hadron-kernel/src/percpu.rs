//! Per-CPU state foundation (SMP-ready).
//!
//! Provides a per-CPU data structure that holds CPU-local state such as
//! the kernel RSP, APIC ID, and CPU ID. Each CPU accesses its own instance
//! via `GS:[0]` self-pointer. The BSP uses a static instance; APs allocate
//! theirs on the heap during bootstrap.

use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use crate::id::CpuId;

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
/// - offset  0: `self_ptr` (for `GS:[0]` self-pointer pattern)
/// - offset  8: `kernel_rsp`
/// - offset 16: `user_rsp`
/// - offset 24: `cpu_id` (4 bytes)
/// - offset 28: `apic_id` (1 byte)
/// - offset 29: `initialized` (1 byte)
/// - offset 30: 2 bytes padding
/// - offset 32: `user_context_ptr`
/// - offset 40: `saved_kernel_rsp_ptr`
/// - offset 48: `trap_reason_ptr`
/// - offset 56: `saved_regs_ptr`
///
/// Each CPU's GS base points to its own `PerCpu` instance. `current_cpu()`
/// reads `GS:[0]` to get the self-pointer, avoiding global statics.
#[repr(C)]
pub struct PerCpu {
    /// Self-pointer for `GS:[0]` access pattern (offset 0).
    ///
    /// Set during init to point to this struct's own address.
    /// Allows `current_cpu()` to read `GS:[0]` instead of using a global.
    pub self_ptr: u64,
    /// Saved kernel RSP for syscall stack switching (offset 8).
    pub kernel_rsp: u64,
    /// Saved user RSP during syscall handling (offset 16).
    pub user_rsp: u64,
    /// Logical CPU ID (0 for BSP).
    pub cpu_id: AtomicU32,
    /// Local APIC ID.
    pub apic_id: AtomicU8,
    /// Whether this per-CPU instance has been initialized.
    initialized: AtomicBool,
    /// Pointer to this CPU's `USER_CONTEXT` (offset 32).
    ///
    /// Set by `init_percpu_process_ptrs` in hadron-kernel. Used by the
    /// timer preemption stub to save user registers via `GS:[32]`.
    pub user_context_ptr: u64,
    /// Pointer to this CPU's `SAVED_KERNEL_RSP` (offset 40).
    ///
    /// Used by the timer preemption stub via `GS:[40]` to longjmp back
    /// to `process_task` after preempting userspace.
    pub saved_kernel_rsp_ptr: u64,
    /// Pointer to this CPU's `TRAP_REASON` (offset 48).
    ///
    /// Used by the timer preemption stub via `GS:[48]` to set the trap
    /// reason before longjmping back to `process_task`.
    pub trap_reason_ptr: u64,
    /// Pointer to this CPU's `SYSCALL_SAVED_REGS` (offset 56).
    ///
    /// Used by the syscall entry stub via `GS:[56]` to save callee-saved
    /// registers for blocking syscall resume.
    pub saved_regs_ptr: u64,
}

impl PerCpu {
    /// Creates a new uninitialized `PerCpu`.
    pub const fn new() -> Self {
        Self {
            self_ptr: 0,
            kernel_rsp: 0,
            user_rsp: 0,
            cpu_id: AtomicU32::new(0),
            apic_id: AtomicU8::new(0),
            initialized: AtomicBool::new(false),
            user_context_ptr: 0,
            saved_kernel_rsp_ptr: 0,
            trap_reason_ptr: 0,
            saved_regs_ptr: 0,
        }
    }

    /// Initializes this per-CPU instance.
    pub fn init(&self, cpu_id: CpuId, apic_id: u8) {
        self.cpu_id.store(cpu_id.as_u32(), Ordering::Relaxed);
        self.apic_id.store(apic_id, Ordering::Relaxed);
        self.initialized.store(true, Ordering::Release);
    }

    /// Returns the CPU ID.
    pub fn get_cpu_id(&self) -> CpuId {
        CpuId::new(self.cpu_id.load(Ordering::Relaxed))
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

/// BSP per-CPU data (single static instance for BSP).
static mut BSP_PERCPU: PerCpu = PerCpu::new();

/// Number of online CPUs.
static CPU_COUNT: AtomicU32 = AtomicU32::new(1);

/// Returns the number of online CPUs.
pub fn cpu_count() -> u32 {
    CPU_COUNT.load(Ordering::Acquire)
}

/// Sets the number of online CPUs.
pub fn set_cpu_count(count: u32) {
    CPU_COUNT.store(count, Ordering::Release);
}

/// Returns a reference to the current CPU's per-CPU data.
///
/// Reads the self-pointer from `GS:[0]`, which was set during CPU init.
#[cfg(target_arch = "x86_64")]
pub fn current_cpu() -> &'static PerCpu {
    unsafe {
        let ptr: u64;
        // SAFETY: GS:[0] contains the self_ptr field, which points to the
        // PerCpu struct itself. This was set during init_gs_base (BSP) or
        // AP bootstrap. The read is lock-free and always valid after init.
        core::arch::asm!("mov {}, gs:[0]", out(reg) ptr, options(readonly, nostack));
        &*(ptr as *const PerCpu)
    }
}

/// Initializes GS-base MSRs to point to the BSP per-CPU data.
///
/// Sets both `IA32_GS_BASE` and `IA32_KERNEL_GS_BASE` to `&BSP_PERCPU`.
/// Also sets the `self_ptr` field so `current_cpu()` works via `GS:[0]`.
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
    // Writing self_ptr and kernel_rsp before any syscall can fire is the
    // caller's requirement (guaranteed by the # Safety contract). Writing
    // both GS_BASE and KERNEL_GS_BASE to the same address means swapgs is
    // a no-op from ring 0, which is correct before any user process exists.
    unsafe {
        let percpu_ptr = core::ptr::addr_of_mut!(BSP_PERCPU);
        (*percpu_ptr).self_ptr = percpu_addr;
        (*percpu_ptr).kernel_rsp = stack_top;

        // Initialize the SYSCALL_SAVED_REGS pointer for the BSP so that
        // the SYSCALL entry stub (GS:[56]) doesn't dereference a null
        // pointer. This is needed both in the full kernel boot path and
        // in the test harness (which calls cpu_init but not kernel_init).
        (*percpu_ptr).saved_regs_ptr = crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
            .get_for(CpuId::new(0))
            .get() as u64;

        IA32_GS_BASE.write(percpu_addr);
        IA32_KERNEL_GS_BASE.write(percpu_addr);
    }

    crate::kdebug!(
        "GS base initialized: percpu={:#x}, kernel_rsp={:#x}",
        percpu_addr,
        stack_top
    );
}

/// Maximum supported CPUs (from Kconfig).
pub use crate::config::MAX_CPUS;

/// CPU-local storage. Wraps `[T; MAX_CPUS]`, indexed by current CPU ID.
///
/// Each AP gets its own instance via the CPU ID index.
pub struct CpuLocal<T> {
    data: [T; MAX_CPUS],
}

impl<T> CpuLocal<T> {
    /// Creates a new `CpuLocal` wrapping the given array.
    pub const fn new(data: [T; MAX_CPUS]) -> Self {
        Self { data }
    }

    /// Returns a reference to the current CPU's instance.
    #[cfg(target_arch = "x86_64")]
    pub fn get(&self) -> &T {
        &self.data[current_cpu().get_cpu_id().as_u32() as usize]
    }

    /// Host-only fallback: always returns CPU 0's instance.
    #[cfg(not(target_arch = "x86_64"))]
    pub fn get(&self) -> &T {
        &self.data[0]
    }

    /// Returns a reference to a specific CPU's instance.
    pub fn get_for(&self, cpu_id: CpuId) -> &T {
        &self.data[cpu_id.as_u32() as usize]
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
