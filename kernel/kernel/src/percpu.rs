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
    /// Padding to 32 bytes.
    pub _pad2: [u8; 2],
    /// Padding from offset 32 to offset 56.
    pub _pad3: [u8; 24],
    /// Pointer to this CPU's `SyscallSavedRegs` (read by syscall entry at `GS:[56]`).
    pub saved_regs_ptr: u64,
}

// Static assertions for GS-relative offsets used in cpu_local.rs.
const _: () = assert!(core::mem::offset_of!(PerCpuState, self_ptr) == 0);
const _: () = assert!(core::mem::offset_of!(PerCpuState, kernel_rsp) == 8);
const _: () = assert!(core::mem::offset_of!(PerCpuState, user_rsp) == 16);
const _: () = assert!(core::mem::offset_of!(PerCpuState, cpu_id) == 24);
const _: () = assert!(core::mem::offset_of!(PerCpuState, initialized) == 29);
const _: () = assert!(core::mem::offset_of!(PerCpuState, saved_regs_ptr) == 56);

/// BSP per-CPU state, allocated statically.
static mut BSP_PERCPU: PerCpuState = PerCpuState {
    self_ptr: 0,
    kernel_rsp: 0,
    user_rsp: 0,
    cpu_id: 0,
    _pad: 0,
    initialized: 0,
    _pad2: [0; 2],
    _pad3: [0; 24],
    saved_regs_ptr: 0,
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

        // Set saved_regs_ptr to the BSP's SyscallSavedRegs for the syscall
        // entry stub (reads GS:[56]).
        (*ptr).saved_regs_ptr = crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
            .get_for(0)
            .get() as u64;

        // SAFETY: IA32_GS_BASE is a valid MSR. Writing the per-CPU state
        // address establishes GS-relative addressing for this CPU.
        IA32_GS_BASE.write(ptr as u64);
    }
}

/// Allocate and initialize a per-CPU state for an AP.
///
/// Heap-allocates a `PerCpuState`, initializes it, leaks it (lives forever),
/// and returns its virtual address (suitable for writing to `IA32_GS_BASE`).
///
/// # Safety
///
/// Must be called from the BSP during `boot_aps()`, after the heap is ready.
#[cfg(hadron_smp)]
pub fn init_ap_percpu(cpu_id: u32, kernel_rsp: u64) -> *mut PerCpuState {
    extern crate alloc;
    use alloc::boxed::Box;

    let state = Box::new(PerCpuState {
        self_ptr: 0, // filled below
        kernel_rsp,
        user_rsp: 0,
        cpu_id,
        _pad: 0,
        initialized: 0, // set to 1 by the AP after full init
        _pad2: [0; 2],
        _pad3: [0; 24],
        saved_regs_ptr: crate::arch::x86_64::syscall::SYSCALL_SAVED_REGS
            .get_for(cpu_id as usize)
            .get() as u64,
    });
    let ptr = Box::into_raw(state);
    // SAFETY: We just allocated this and it will be leaked (never freed).
    unsafe {
        (*ptr).self_ptr = ptr as u64;
    }
    ptr
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
