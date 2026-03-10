//! Per-CPU state and storage.
//!
//! Re-exports `CpuLocal` and `MAX_CPUS` from `hadron_core` and provides
//! kernel-specific per-CPU state (GS-base setup, early kernel RSP).

pub use hadron_core::cpu_local::{CpuLocal, MAX_CPUS};

use crate::arch::x86_64::registers::model_specific::IA32_GS_BASE;

/// Per-CPU state stored at the GS base address.
///
/// Field offsets are read directly by inline assembly in `hadron_core::cpu_local`:
/// - `gs:[0]`  → `self_ptr`
/// - `gs:[24]` → `cpu_id`
/// - `gs:[29]` → `initialized`
#[repr(C)]
pub struct PerCpuState {
    /// Pointer to self (for GS-relative addressing validation).
    pub self_ptr: u64,
    /// Kernel stack pointer for this CPU (TSS RSP0).
    pub kernel_rsp: u64,
    /// Saved user stack pointer (swapgs context switch).
    pub user_rsp: u64,
    /// Logical CPU ID (BSP = 0).
    pub cpu_id: u32,
    /// Padding to place `initialized` at offset 29.
    pub _pad: u8,
    /// Set to 1 after this CPU's per-CPU state is fully initialized.
    pub initialized: u8,
    /// Padding to align the struct to 32 bytes.
    pub _pad2: [u8; 2],
}

// Static assertions for GS-relative offsets used in cpu_local.rs.
const _: () = assert!(core::mem::offset_of!(PerCpuState, self_ptr) == 0);
const _: () = assert!(core::mem::offset_of!(PerCpuState, kernel_rsp) == 8);
const _: () = assert!(core::mem::offset_of!(PerCpuState, user_rsp) == 16);
const _: () = assert!(core::mem::offset_of!(PerCpuState, cpu_id) == 24);
const _: () = assert!(core::mem::offset_of!(PerCpuState, initialized) == 29);

/// BSP per-CPU state, allocated statically.
static mut BSP_PERCPU: PerCpuState = PerCpuState {
    self_ptr: 0,
    kernel_rsp: 0,
    user_rsp: 0,
    cpu_id: 0,
    _pad: 0,
    initialized: 0,
    _pad2: [0; 2],
};

/// Initialize GS base to point at the BSP's per-CPU data.
///
/// After this call, `cpu_is_initialized()` returns `true` and the logging
/// subsystem switches from Phase 0 (direct serial) to Phase 1 (ring buffer).
///
/// # Safety
///
/// Must be called exactly once during early BSP init, after the GDT is loaded.
pub unsafe fn init_gs_base() {
    // SAFETY: Single-threaded BSP init — no concurrent access to BSP_PERCPU.
    unsafe {
        let ptr = core::ptr::addr_of_mut!(BSP_PERCPU);
        (*ptr).self_ptr = ptr as u64;
        (*ptr).kernel_rsp = early_kernel_rsp();
        (*ptr).user_rsp = 0;
        (*ptr).cpu_id = 0;
        (*ptr).initialized = 1;

        // SAFETY: IA32_GS_BASE is a valid MSR. Writing the per-CPU state
        // address establishes GS-relative addressing for this CPU.
        IA32_GS_BASE.write(ptr as u64);
    }
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
