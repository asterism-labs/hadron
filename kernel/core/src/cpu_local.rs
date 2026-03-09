//! Minimal per-CPU storage for host-testable primitives.
//!
//! Provides [`CpuLocal`] indexed by CPU ID. On kernel targets, reads
//! the CPU ID from the GS-based per-CPU data structure. On host targets,
//! always returns index 0 (single-threaded test assumption).

/// Maximum supported CPUs. Matches the Kconfig upper bound.
pub const MAX_CPUS: usize = 256;

/// Per-CPU storage. Wraps `[T; MAX_CPUS]`, indexed by current CPU ID.
pub struct CpuLocal<T> {
    data: [T; MAX_CPUS],
}

impl<T> CpuLocal<T> {
    /// Creates a new `CpuLocal` wrapping the given array.
    pub const fn new(data: [T; MAX_CPUS]) -> Self {
        Self { data }
    }

    /// Returns a reference to the current CPU's instance.
    ///
    /// If the GS base is not yet initialized (e.g. during AP early boot),
    /// `current_cpu_id()` may return garbage. In that case, falls back to
    /// CPU 0's slot to prevent an out-of-bounds panic. This is acceptable
    /// for the atomic counters (IRQ lock depth, lockdep) that are the
    /// primary users of `CpuLocal`.
    pub fn get(&self) -> &T {
        let id = current_cpu_id() as usize;
        if id < MAX_CPUS {
            &self.data[id]
        } else {
            &self.data[0]
        }
    }

    /// Returns a reference to a specific CPU's instance.
    ///
    /// # Panics
    ///
    /// Panics if `cpu_id >= MAX_CPUS`.
    pub fn get_for(&self, cpu_id: u32) -> &T {
        &self.data[cpu_id as usize]
    }
}

// SAFETY: CpuLocal<T> is designed for per-CPU access. Send/Sync are safe
// because each CPU only accesses its own slot.
unsafe impl<T: Send> Send for CpuLocal<T> {}
unsafe impl<T: Send> Sync for CpuLocal<T> {}

/// Returns the current CPU ID.
///
/// On kernel targets, reads from the GS-based per-CPU data structure
/// (offset 24 = `PerCpu::cpu_id`). On host targets, returns 0.
#[inline]
pub fn current_cpu_id() -> u32 {
    #[cfg(all(target_os = "none", target_arch = "x86_64"))]
    {
        // SAFETY: GS:[24] contains the cpu_id field of the PerCpu struct,
        // which is AtomicU32 at offset 24 in the #[repr(C)] layout. This
        // is valid after GS-base initialization during CPU init.
        unsafe {
            let id: u32;
            core::arch::asm!("mov {:e}, gs:[24]", out(reg) id, options(readonly, nostack));
            id
        }
    }
    #[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
    {
        0
    }
}

/// Returns whether the current CPU's per-CPU data has been initialized.
///
/// On kernel targets, reads the `initialized` field from the GS-based
/// per-CPU data (offset 29 = `PerCpu::initialized`). On host targets,
/// always returns `true`.
#[inline]
pub fn cpu_is_initialized() -> bool {
    #[cfg(all(target_os = "none", target_arch = "x86_64"))]
    {
        // SAFETY: GS:[0] contains the self_ptr. Before GS base is set up
        // (e.g. AP early boot with GS base = 0), reading GS:[0] fetches from
        // VA 0, which holds real-mode IVT entries — non-zero but well below
        // the kernel half. We check that the self-pointer is in the kernel
        // upper half (>= 0xFFFF_8000_0000_0000) to catch both null and
        // garbage reads.
        unsafe {
            let self_ptr: u64;
            core::arch::asm!("mov {}, gs:[0]", out(reg) self_ptr, options(readonly, nostack));
            if self_ptr < 0xFFFF_8000_0000_0000 {
                return false;
            }
            let init: u8;
            core::arch::asm!("mov {}, gs:[29]", out(reg_byte) init, options(readonly, nostack));
            init != 0
        }
    }
    #[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
    {
        true
    }
}
