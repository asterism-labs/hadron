//! Per-CPU storage via the `.percpu` linker section.
//!
//! On kernel targets, each [`CpuLocal<T>`] places a single template `T` in the
//! `.percpu` section. At boot, `percpu_init_phase2` allocates per-CPU copies
//! from the heap. Access is via `per_cpu_base[cpu_id] + section_offset`.
//!
//! On host targets, [`CpuLocal<T>`] wraps a single `T` for unit testing.

/// Hard cap for the global base pointer array. Only this constant
/// remains as a compile-time limit; actual CPU count is runtime.
pub const MAX_CPUS_HARD: usize = 256;

// ── Kernel target ────────────────────────────────────────────────────

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
mod kernel {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::MAX_CPUS_HARD;

    /// Per-CPU base addresses, indexed by cpu_id.
    /// `PERCPU_BASES[i]` = start of CPU i's percpu region.
    pub static PERCPU_BASES: [AtomicUsize; MAX_CPUS_HARD] =
        [const { AtomicUsize::new(0) }; MAX_CPUS_HARD];

    /// Per-CPU variable handle. Stores a pointer to the template in `.percpu`.
    pub struct CpuLocal<T> {
        template: *const T,
    }

    // SAFETY: CpuLocal<T> is designed for per-CPU access. Each CPU only
    // accesses its own copy of the data.
    unsafe impl<T: Send> Send for CpuLocal<T> {}
    unsafe impl<T: Send> Sync for CpuLocal<T> {}

    impl<T> CpuLocal<T> {
        /// Create a `CpuLocal` from a pointer to the template in `.percpu`.
        ///
        /// # Safety
        ///
        /// `ptr` must point to a static in the `.percpu` linker section.
        pub const unsafe fn from_template_ptr(ptr: *const T) -> Self {
            Self { template: ptr }
        }

        /// Returns a reference to the current CPU's instance.
        ///
        /// Reads the per-CPU base from `gs:[32]` (`PerCpuState::percpu_base`)
        /// and adds the template's section offset.
        ///
        /// Falls back to the `.percpu` template before GS is initialized
        /// or before `percpu_init_phase1()`. Uses the same `cpu_is_initialized()`
        /// guard as the old `CpuLocal::get()`.
        #[inline]
        pub fn get(&self) -> &T {
            let id = super::current_cpu_id() as usize;
            if id >= super::MAX_CPUS_HARD {
                // GS not yet initialized — cpu_id is garbage.
                // SAFETY: template points to a valid static in .percpu.
                return unsafe { &*self.template };
            }
            let base = PERCPU_BASES[id].load(Ordering::Relaxed);
            if base == 0 {
                // percpu region not yet allocated for this CPU.
                // SAFETY: template points to a valid static in .percpu.
                return unsafe { &*self.template };
            }
            let offset = self.template as usize - percpu_start();
            // SAFETY: base + offset points into a valid per-CPU copy.
            unsafe { &*((base + offset) as *const T) }
        }

        /// Returns a reference to a specific CPU's instance.
        ///
        /// # Panics
        ///
        /// Panics if the per-CPU region for `cpu_id` has not been initialized.
        #[inline]
        pub fn get_for(&self, cpu_id: u32) -> &T {
            let base = PERCPU_BASES[cpu_id as usize].load(Ordering::Relaxed);
            debug_assert!(base != 0, "per-CPU region not initialized for CPU {cpu_id}");
            if base == 0 {
                // SAFETY: template points to a valid static in .percpu.
                return unsafe { &*self.template };
            }
            let offset = self.template as usize - percpu_start();
            // SAFETY: base + offset points into a valid per-CPU copy.
            unsafe { &*((base + offset) as *const T) }
        }
    }

    /// Returns the virtual address of the `.percpu` section start.
    #[inline]
    pub fn percpu_start() -> usize {
        unsafe extern "C" {
            static __percpu_start: u8;
        }
        // Linker-defined symbol; we only take its address.
        (&raw const __percpu_start) as usize
    }

    /// Returns the size of the `.percpu` template section in bytes.
    #[inline]
    pub fn percpu_section_size() -> usize {
        unsafe extern "C" {
            static __percpu_start: u8;
            static __percpu_end: u8;
        }
        // Linker-defined symbols; we only take their addresses.
        (&raw const __percpu_end) as usize - (&raw const __percpu_start) as usize
    }
}

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
pub use kernel::*;

// ── Host target (testing) ────────────────────────────────────────────

#[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
mod host {
    /// Host-mode per-CPU variable. Wraps a single `T` (single-CPU assumption).
    pub struct CpuLocal<T> {
        data: T,
    }

    // SAFETY: Host mode is single-threaded for per-CPU purposes.
    unsafe impl<T: Send> Send for CpuLocal<T> {}
    unsafe impl<T: Send> Sync for CpuLocal<T> {}

    impl<T> CpuLocal<T> {
        /// Creates a host-mode `CpuLocal` wrapping the given value.
        pub const fn new_host(data: T) -> Self {
            Self { data }
        }

        /// Returns a reference to the wrapped value.
        pub fn get(&self) -> &T {
            &self.data
        }

        /// Returns a reference to the wrapped value (ignores `cpu_id`).
        pub fn get_for(&self, _cpu_id: u32) -> &T {
            &self.data
        }
    }
}

#[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
pub use host::*;

// ── percpu_static! macro ─────────────────────────────────────────────

/// Wrapper to make a `T: Send` usable as a `static` template.
///
/// Per-CPU templates are only accessed via pointer arithmetic (never
/// through the static directly), so `Sync` is not required for safety.
/// This wrapper provides the `Sync` impl the compiler demands for statics.
#[repr(transparent)]
pub struct PercpuTemplate<T>(T);

// SAFETY: The template static is never accessed concurrently. It serves
// only as a byte pattern that is copied to per-CPU regions at boot.
unsafe impl<T: Send> Sync for PercpuTemplate<T> {}

impl<T> PercpuTemplate<T> {
    /// Creates a new template wrapper.
    pub const fn new(val: T) -> Self {
        Self(val)
    }
}

/// Declares a per-CPU static variable.
///
/// On kernel targets, places the template in the `.percpu` linker section.
/// On host targets, wraps a single value for unit testing.
///
/// # Examples
///
/// ```ignore
/// use core::sync::atomic::AtomicU32;
/// percpu_static!(static MY_COUNTER: AtomicU32 = AtomicU32::new(0));
/// ```
#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[macro_export]
macro_rules! percpu_static {
    ($vis:vis static $name:ident : $ty:ty = $init:expr) => {
        $vis static $name: $crate::cpu_local::CpuLocal<$ty> = {
            #[unsafe(link_section = ".percpu")]
            #[used]
            static TEMPLATE: $crate::cpu_local::PercpuTemplate<$ty> =
                $crate::cpu_local::PercpuTemplate::new($init);
            // SAFETY: TEMPLATE is placed in the .percpu section by the linker.
            // The pointer to the inner T is at the same address as the wrapper
            // due to #[repr(transparent)].
            unsafe {
                $crate::cpu_local::CpuLocal::from_template_ptr(
                    &raw const TEMPLATE as *const $ty
                )
            }
        };
    };
}

/// Host-mode `percpu_static!` — wraps a single value.
#[cfg(not(all(target_os = "none", target_arch = "x86_64")))]
#[macro_export]
macro_rules! percpu_static {
    ($vis:vis static $name:ident : $ty:ty = $init:expr) => {
        $vis static $name: $crate::cpu_local::CpuLocal<$ty> =
            $crate::cpu_local::CpuLocal::new_host($init);
    };
}

// ── Utility functions (shared) ───────────────────────────────────────

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

// ── Host tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicU32, Ordering};

    percpu_static!(static TEST_COUNTER: AtomicU32 = AtomicU32::new(42));

    #[test]
    fn test_percpu_static_get_returns_init_value() {
        assert_eq!(TEST_COUNTER.get().load(Ordering::Relaxed), 42);
    }

    #[test]
    fn test_percpu_static_get_for_zero() {
        assert_eq!(TEST_COUNTER.get_for(0).load(Ordering::Relaxed), 42);
    }
}
