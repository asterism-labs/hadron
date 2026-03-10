//! Per-CPU state and storage.
//!
//! Re-exports `CpuLocal` and `MAX_CPUS` from `hadron_core` and provides
//! kernel-specific per-CPU state (GS-base setup, early kernel RSP).

pub use hadron_core::cpu_local::{CpuLocal, MAX_CPUS};

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
