//! Backend trait abstraction for sync primitives.
//!
//! Provides a trait-generic interface over atomic types, cells, and interrupt
//! control, allowing algorithm code to be written once and tested under both
//! the production [`CoreBackend`] and the model-checked [`LoomBackend`].

use core::sync::atomic::Ordering;

// ─── Atomic operation traits ──────────────────────────────────────────

/// Operations on an atomic boolean.
pub trait AtomicBoolOps {
    /// Loads the value.
    fn load(&self, order: Ordering) -> bool;
    /// Stores a value.
    fn store(&self, val: bool, order: Ordering);
    /// Stores a value, returning the previous value.
    fn swap(&self, val: bool, order: Ordering) -> bool;
    /// Stores if current matches, strong version.
    fn compare_exchange(
        &self,
        current: bool,
        new: bool,
        success: Ordering,
        failure: Ordering,
    ) -> Result<bool, bool>;
    /// Stores if current matches, weak version.
    fn compare_exchange_weak(
        &self,
        current: bool,
        new: bool,
        success: Ordering,
        failure: Ordering,
    ) -> Result<bool, bool>;
    /// Logical OR, returning previous value.
    fn fetch_or(&self, val: bool, order: Ordering) -> bool;
    /// Logical AND, returning previous value.
    fn fetch_and(&self, val: bool, order: Ordering) -> bool;
    /// Logical XOR, returning previous value.
    fn fetch_xor(&self, val: bool, order: Ordering) -> bool;
}

/// Operations on an atomic integer.
pub trait AtomicIntOps<V> {
    /// Loads the value.
    fn load(&self, order: Ordering) -> V;
    /// Stores a value.
    fn store(&self, val: V, order: Ordering);
    /// Stores a value, returning the previous value.
    fn swap(&self, val: V, order: Ordering) -> V;
    /// Stores if current matches, strong version.
    fn compare_exchange(
        &self,
        current: V,
        new: V,
        success: Ordering,
        failure: Ordering,
    ) -> Result<V, V>;
    /// Stores if current matches, weak version.
    fn compare_exchange_weak(
        &self,
        current: V,
        new: V,
        success: Ordering,
        failure: Ordering,
    ) -> Result<V, V>;
    /// Add, returning previous value.
    fn fetch_add(&self, val: V, order: Ordering) -> V;
    /// Subtract, returning previous value.
    fn fetch_sub(&self, val: V, order: Ordering) -> V;
    /// Bitwise OR, returning previous value.
    fn fetch_or(&self, val: V, order: Ordering) -> V;
    /// Bitwise AND, returning previous value.
    fn fetch_and(&self, val: V, order: Ordering) -> V;
    /// Bitwise XOR, returning previous value.
    fn fetch_xor(&self, val: V, order: Ordering) -> V;
}

/// Operations on an unsafe cell.
pub trait UnsafeCellOps<T> {
    /// Obtain a shared pointer to the inner value.
    fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R;
    /// Obtain a mutable pointer to the inner value.
    fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R;
}

// ─── Backend trait ────────────────────────────────────────────────────

/// Abstraction over atomic primitives, cells, and spin-wait hints.
///
/// [`CoreBackend`] delegates to `core::sync::atomic` and `core::cell`.
/// [`LoomBackend`] (behind `cfg(loom)`) delegates to `loom::sync::atomic`
/// and `loom::cell`.
pub trait Backend: 'static {
    /// Atomic boolean type.
    type AtomicBool: AtomicBoolOps + Send + Sync;
    /// Atomic u8 type.
    type AtomicU8: AtomicIntOps<u8> + Send + Sync;
    /// Atomic u32 type.
    type AtomicU32: AtomicIntOps<u32> + Send + Sync;
    /// Atomic usize type.
    type AtomicUsize: AtomicIntOps<usize> + Send + Sync;
    /// Interior-mutable cell.
    type UnsafeCell<T>: UnsafeCellOps<T>;

    /// Creates a new atomic boolean.
    fn new_atomic_bool(val: bool) -> Self::AtomicBool;
    /// Creates a new atomic u8.
    fn new_atomic_u8(val: u8) -> Self::AtomicU8;
    /// Creates a new atomic u32.
    fn new_atomic_u32(val: u32) -> Self::AtomicU32;
    /// Creates a new atomic usize.
    fn new_atomic_usize(val: usize) -> Self::AtomicUsize;
    /// Creates a new unsafe cell.
    fn new_unsafe_cell<T>(val: T) -> Self::UnsafeCell<T>;

    /// Memory fence.
    fn fence(order: Ordering);
    /// Compiler fence.
    fn compiler_fence(order: Ordering);
    /// Yields execution in a spin-wait loop.
    fn spin_wait_hint();
}

/// Extension of [`Backend`] with interrupt control for IRQ-safe locks.
pub trait IrqBackend: Backend {
    /// Save current interrupt state and disable interrupts.
    fn save_flags_and_cli() -> u64;
    /// Restore interrupt state from a previously saved flags value.
    fn restore_flags(flags: u64);
}

// ─── Trait impl macros ────────────────────────────────────────────────

macro_rules! impl_atomic_bool_ops {
    ($ty:ty) => {
        impl AtomicBoolOps for $ty {
            #[inline]
            fn load(&self, order: Ordering) -> bool {
                self.load(order)
            }
            #[inline]
            fn store(&self, val: bool, order: Ordering) {
                self.store(val, order);
            }
            #[inline]
            fn swap(&self, val: bool, order: Ordering) -> bool {
                self.swap(val, order)
            }
            #[inline]
            fn compare_exchange(
                &self,
                current: bool,
                new: bool,
                success: Ordering,
                failure: Ordering,
            ) -> Result<bool, bool> {
                self.compare_exchange(current, new, success, failure)
            }
            #[inline]
            fn compare_exchange_weak(
                &self,
                current: bool,
                new: bool,
                success: Ordering,
                failure: Ordering,
            ) -> Result<bool, bool> {
                self.compare_exchange_weak(current, new, success, failure)
            }
            #[inline]
            fn fetch_or(&self, val: bool, order: Ordering) -> bool {
                self.fetch_or(val, order)
            }
            #[inline]
            fn fetch_and(&self, val: bool, order: Ordering) -> bool {
                self.fetch_and(val, order)
            }
            #[inline]
            fn fetch_xor(&self, val: bool, order: Ordering) -> bool {
                self.fetch_xor(val, order)
            }
        }
    };
}

macro_rules! impl_atomic_int_ops {
    ($atomic:ty, $val:ty) => {
        impl AtomicIntOps<$val> for $atomic {
            #[inline]
            fn load(&self, order: Ordering) -> $val {
                self.load(order)
            }
            #[inline]
            fn store(&self, val: $val, order: Ordering) {
                self.store(val, order);
            }
            #[inline]
            fn swap(&self, val: $val, order: Ordering) -> $val {
                self.swap(val, order)
            }
            #[inline]
            fn compare_exchange(
                &self,
                current: $val,
                new: $val,
                success: Ordering,
                failure: Ordering,
            ) -> Result<$val, $val> {
                self.compare_exchange(current, new, success, failure)
            }
            #[inline]
            fn compare_exchange_weak(
                &self,
                current: $val,
                new: $val,
                success: Ordering,
                failure: Ordering,
            ) -> Result<$val, $val> {
                self.compare_exchange_weak(current, new, success, failure)
            }
            #[inline]
            fn fetch_add(&self, val: $val, order: Ordering) -> $val {
                self.fetch_add(val, order)
            }
            #[inline]
            fn fetch_sub(&self, val: $val, order: Ordering) -> $val {
                self.fetch_sub(val, order)
            }
            #[inline]
            fn fetch_or(&self, val: $val, order: Ordering) -> $val {
                self.fetch_or(val, order)
            }
            #[inline]
            fn fetch_and(&self, val: $val, order: Ordering) -> $val {
                self.fetch_and(val, order)
            }
            #[inline]
            fn fetch_xor(&self, val: $val, order: Ordering) -> $val {
                self.fetch_xor(val, order)
            }
        }
    };
}

// ─── Core trait impls ─────────────────────────────────────────────────

impl_atomic_bool_ops!(core::sync::atomic::AtomicBool);
impl_atomic_int_ops!(core::sync::atomic::AtomicU8, u8);
impl_atomic_int_ops!(core::sync::atomic::AtomicU32, u32);
impl_atomic_int_ops!(core::sync::atomic::AtomicUsize, usize);

impl<T> UnsafeCellOps<T> for core::cell::UnsafeCell<T> {
    #[inline]
    fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
        f(self.get() as *const T)
    }
    #[inline]
    fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        f(self.get())
    }
}

// ─── Loom trait impls ─────────────────────────────────────────────────

#[cfg(loom)]
impl_atomic_bool_ops!(loom::sync::atomic::AtomicBool);
#[cfg(loom)]
impl_atomic_int_ops!(loom::sync::atomic::AtomicU8, u8);
#[cfg(loom)]
impl_atomic_int_ops!(loom::sync::atomic::AtomicU32, u32);
#[cfg(loom)]
impl_atomic_int_ops!(loom::sync::atomic::AtomicUsize, usize);

#[cfg(loom)]
impl<T> UnsafeCellOps<T> for loom::cell::UnsafeCell<T> {
    #[inline]
    fn with<R>(&self, f: impl FnOnce(*const T) -> R) -> R {
        loom::cell::UnsafeCell::with(self, f)
    }
    #[inline]
    fn with_mut<R>(&self, f: impl FnOnce(*mut T) -> R) -> R {
        loom::cell::UnsafeCell::with_mut(self, f)
    }
}

// ─── Shuttle trait impls ─────────────────────────────────────────────

#[cfg(shuttle)]
impl_atomic_bool_ops!(shuttle::sync::atomic::AtomicBool);
#[cfg(shuttle)]
impl_atomic_int_ops!(shuttle::sync::atomic::AtomicU8, u8);
#[cfg(shuttle)]
impl_atomic_int_ops!(shuttle::sync::atomic::AtomicU32, u32);
#[cfg(shuttle)]
impl_atomic_int_ops!(shuttle::sync::atomic::AtomicUsize, usize);

// ShuttleBackend uses core::cell::UnsafeCell, which already has
// UnsafeCellOps implemented above.

// ─── CoreBackend ──────────────────────────────────────────────────────

/// Production backend using `core::sync::atomic` and `core::cell`.
pub struct CoreBackend;

impl Backend for CoreBackend {
    type AtomicBool = core::sync::atomic::AtomicBool;
    type AtomicU8 = core::sync::atomic::AtomicU8;
    type AtomicU32 = core::sync::atomic::AtomicU32;
    type AtomicUsize = core::sync::atomic::AtomicUsize;
    type UnsafeCell<T> = core::cell::UnsafeCell<T>;

    #[inline]
    fn new_atomic_bool(val: bool) -> Self::AtomicBool {
        core::sync::atomic::AtomicBool::new(val)
    }
    #[inline]
    fn new_atomic_u8(val: u8) -> Self::AtomicU8 {
        core::sync::atomic::AtomicU8::new(val)
    }
    #[inline]
    fn new_atomic_u32(val: u32) -> Self::AtomicU32 {
        core::sync::atomic::AtomicU32::new(val)
    }
    #[inline]
    fn new_atomic_usize(val: usize) -> Self::AtomicUsize {
        core::sync::atomic::AtomicUsize::new(val)
    }
    #[inline]
    fn new_unsafe_cell<T>(val: T) -> Self::UnsafeCell<T> {
        core::cell::UnsafeCell::new(val)
    }

    #[inline]
    fn fence(order: Ordering) {
        core::sync::atomic::fence(order);
    }
    #[inline]
    fn compiler_fence(order: Ordering) {
        core::sync::atomic::compiler_fence(order);
    }
    #[inline]
    fn spin_wait_hint() {
        core::hint::spin_loop();
    }
}

impl IrqBackend for CoreBackend {
    #[inline]
    fn save_flags_and_cli() -> u64 {
        save_flags_and_cli_impl()
    }
    #[inline]
    fn restore_flags(flags: u64) {
        restore_flags_impl(flags);
    }
}

// ─── Platform-specific IRQ helpers ────────────────────────────────────

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[inline]
fn save_flags_and_cli_impl() -> u64 {
    let flags: u64;
    // SAFETY: Reading RFLAGS and disabling interrupts is safe in kernel mode.
    unsafe {
        core::arch::asm!(
            "pushfq",
            "pop {}",
            "cli",
            out(reg) flags,
            options(nomem),
        );
    }
    flags
}

#[cfg(all(target_os = "none", target_arch = "x86_64"))]
#[inline]
fn restore_flags_impl(flags: u64) {
    if flags & (1 << 9) != 0 {
        // SAFETY: Re-enabling interrupts is safe; we are restoring a previous state.
        unsafe {
            core::arch::asm!("sti", options(nomem, nostack, preserves_flags));
        }
    }
}

#[cfg(all(target_os = "none", target_arch = "aarch64"))]
#[inline]
fn save_flags_and_cli_impl() -> u64 {
    let flags: u64;
    // SAFETY: Reading DAIF and masking interrupts is safe in kernel mode.
    unsafe {
        core::arch::asm!(
            "mrs {}, DAIF",
            "msr DAIFSet, #0xf",
            out(reg) flags,
            options(nomem),
        );
    }
    flags
}

#[cfg(all(target_os = "none", target_arch = "aarch64"))]
#[inline]
fn restore_flags_impl(flags: u64) {
    // SAFETY: Restoring DAIF is safe; we are restoring a previous state.
    unsafe {
        core::arch::asm!(
            "msr DAIF, {}",
            in(reg) flags,
            options(nomem, nostack, preserves_flags),
        );
    }
}

#[cfg(not(target_os = "none"))]
#[inline]
fn save_flags_and_cli_impl() -> u64 {
    0
}

#[cfg(not(target_os = "none"))]
#[inline]
fn restore_flags_impl(_flags: u64) {}

// ─── LoomBackend ──────────────────────────────────────────────────────

/// Loom model-checker backend using `loom::sync::atomic` and `loom::cell`.
#[cfg(loom)]
pub struct LoomBackend;

#[cfg(loom)]
impl Backend for LoomBackend {
    type AtomicBool = loom::sync::atomic::AtomicBool;
    type AtomicU8 = loom::sync::atomic::AtomicU8;
    type AtomicU32 = loom::sync::atomic::AtomicU32;
    type AtomicUsize = loom::sync::atomic::AtomicUsize;
    type UnsafeCell<T> = loom::cell::UnsafeCell<T>;

    #[inline]
    fn new_atomic_bool(val: bool) -> Self::AtomicBool {
        loom::sync::atomic::AtomicBool::new(val)
    }
    #[inline]
    fn new_atomic_u8(val: u8) -> Self::AtomicU8 {
        loom::sync::atomic::AtomicU8::new(val)
    }
    #[inline]
    fn new_atomic_u32(val: u32) -> Self::AtomicU32 {
        loom::sync::atomic::AtomicU32::new(val)
    }
    #[inline]
    fn new_atomic_usize(val: usize) -> Self::AtomicUsize {
        loom::sync::atomic::AtomicUsize::new(val)
    }
    #[inline]
    fn new_unsafe_cell<T>(val: T) -> Self::UnsafeCell<T> {
        loom::cell::UnsafeCell::new(val)
    }

    #[inline]
    fn fence(order: Ordering) {
        loom::sync::atomic::fence(order);
    }
    #[inline]
    fn compiler_fence(order: Ordering) {
        // Loom makes compiler fences redundant; use core version.
        core::sync::atomic::compiler_fence(order);
    }
    #[inline]
    fn spin_wait_hint() {
        loom::thread::yield_now();
    }
}

#[cfg(loom)]
impl IrqBackend for LoomBackend {
    #[inline]
    fn save_flags_and_cli() -> u64 {
        super::loom_mock::mock_save_flags_and_cli()
    }
    #[inline]
    fn restore_flags(flags: u64) {
        super::loom_mock::mock_restore_flags(flags);
    }
}

// ─── ShuttleBackend ──────────────────────────────────────────────────

/// Shuttle random-testing backend using `shuttle::sync::atomic`.
///
/// Unlike loom (which exhaustively explores all interleavings), Shuttle uses
/// randomized scheduling to cover large state spaces (3+ threads, many
/// iterations) that would be infeasible under exhaustive exploration.
///
/// Shuttle has no cell model — `core::cell::UnsafeCell` is used directly.
/// Cell-level data races are loom's domain; Shuttle tests focus on protocol
/// correctness under realistic concurrency.
#[cfg(shuttle)]
pub struct ShuttleBackend;

#[cfg(shuttle)]
impl Backend for ShuttleBackend {
    type AtomicBool = shuttle::sync::atomic::AtomicBool;
    type AtomicU8 = shuttle::sync::atomic::AtomicU8;
    type AtomicU32 = shuttle::sync::atomic::AtomicU32;
    type AtomicUsize = shuttle::sync::atomic::AtomicUsize;
    type UnsafeCell<T> = core::cell::UnsafeCell<T>;

    #[inline]
    fn new_atomic_bool(val: bool) -> Self::AtomicBool {
        shuttle::sync::atomic::AtomicBool::new(val)
    }
    #[inline]
    fn new_atomic_u8(val: u8) -> Self::AtomicU8 {
        shuttle::sync::atomic::AtomicU8::new(val)
    }
    #[inline]
    fn new_atomic_u32(val: u32) -> Self::AtomicU32 {
        shuttle::sync::atomic::AtomicU32::new(val)
    }
    #[inline]
    fn new_atomic_usize(val: usize) -> Self::AtomicUsize {
        shuttle::sync::atomic::AtomicUsize::new(val)
    }
    #[inline]
    fn new_unsafe_cell<T>(val: T) -> Self::UnsafeCell<T> {
        core::cell::UnsafeCell::new(val)
    }

    #[inline]
    fn fence(order: Ordering) {
        shuttle::sync::atomic::fence(order);
    }
    #[inline]
    fn compiler_fence(order: Ordering) {
        // Shuttle does not intercept compiler fences; use core version.
        core::sync::atomic::compiler_fence(order);
    }
    #[inline]
    fn spin_wait_hint() {
        shuttle::thread::yield_now();
    }
}

#[cfg(shuttle)]
impl IrqBackend for ShuttleBackend {
    #[inline]
    fn save_flags_and_cli() -> u64 {
        super::shuttle_mock::mock_save_flags_and_cli()
    }
    #[inline]
    fn restore_flags(flags: u64) {
        super::shuttle_mock::mock_restore_flags(flags);
    }
}
