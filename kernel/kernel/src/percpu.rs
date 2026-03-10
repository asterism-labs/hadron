//! Minimal per-CPU state stubs.
//!
//! These stubs satisfy compile-time references from GDT, syscall, and other
//! arch modules. Runtime correctness is deferred to a later phase.

/// Maximum number of CPUs supported.
pub const MAX_CPUS: usize = 64;

/// Per-CPU data wrapper indexed by CPU ID.
///
/// Stores one `T` per CPU. In the current stub, this is just a fixed-size array.
pub struct CpuLocal<T> {
    data: [T; MAX_CPUS],
}

// SAFETY: CpuLocal is accessed with interrupts disabled or by a single CPU.
// This Sync impl matches the real implementation's safety contract.
unsafe impl<T> Sync for CpuLocal<T> {}

impl<T> CpuLocal<T> {
    /// Creates a new `CpuLocal` from a pre-initialized array.
    pub const fn new(data: [T; MAX_CPUS]) -> Self {
        Self { data }
    }
}

/// Per-CPU state (stub).
#[repr(C)]
pub struct PerCpuState {
    /// Pointer to self (for GS-relative addressing).
    pub self_ptr: u64,
    /// Kernel stack pointer for this CPU.
    pub kernel_rsp: u64,
    /// Saved user stack pointer.
    pub user_rsp: u64,
}

/// Initialize GS base to point at the BSP's per-CPU data.
///
/// # Safety
///
/// Must be called exactly once during early BSP init, after the GDT is loaded.
pub unsafe fn init_gs_base() {
    // Stub: no-op during early boot. Real implementation sets IA32_GS_BASE MSR.
}

/// Returns the early kernel RSP for TSS initialization.
///
/// Uses a statically allocated BSS stack for the BSP before the VMM is ready.
pub fn early_kernel_rsp() -> u64 {
    /// Early BSP kernel stack (64 KiB, 16-byte aligned).
    #[repr(align(16))]
    struct EarlyStack([u8; 65536]);

    static EARLY_STACK: EarlyStack = EarlyStack([0; 65536]);

    // Stack grows downward; return the top.
    let base = &EARLY_STACK as *const _ as u64;
    base + 65536
}
