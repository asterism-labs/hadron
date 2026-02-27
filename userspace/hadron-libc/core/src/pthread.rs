//! POSIX threads — `pthread_create`, `pthread_join`, `pthread_mutex_*`,
//! `pthread_cond_*`, `pthread_once`, `pthread_key_*`.
//!
//! # Thread model
//!
//! Threads are created via `task_clone(CLONE_VM | CLONE_FILES |
//! CLONE_SIGHAND | CLONE_SETTLS)`. The Thread Control Block (TCB) is
//! allocated on the thread's stack region and its address is passed as
//! `tls_ptr` (written to FS_BASE by the kernel). Reading `%fs:0` yields
//! the self-pointer at the start of the TCB.
//!
//! # Mutex implementation
//!
//! `pthread_mutex_t.__lock` is a `u32` futex word:
//! - `0` = unlocked
//! - `1` = locked, no waiters
//! - `2` = locked, waiters present
//!
//! # Condition variable implementation
//!
//! `pthread_cond_t.__seq` is a monotonically increasing sequence number.
//! Waiters read the current seq, release the mutex, then `futex(WAIT, seq)`.
//! Signalers increment seq and `futex(WAKE, 1)` or `futex(WAKE, INT_MAX)`.

use core::sync::atomic::{AtomicU32, Ordering};

use crate::sys;

// ---- Constants --------------------------------------------------------------

const DEFAULT_STACK_SIZE: usize = 2 * 1024 * 1024; // 2 MiB
const MAX_PTHREAD_KEYS: usize = 128;
const TCB_ALIGN: usize = 16;

const CLONE_VM: usize = 0x0100;
const CLONE_FILES: usize = 0x0400;
const CLONE_SIGHAND: usize = 0x0800;
const CLONE_SETTLS: usize = 0x0008_0000;

const FUTEX_WAIT: usize = 0;
const FUTEX_WAKE: usize = 1;

// Pthread once states
const ONCE_UNINITIALIZED: u32 = 0;
const ONCE_IN_PROGRESS: u32 = 1;
const ONCE_DONE: u32 = 2;

// Mutex lock states
const MUTEX_UNLOCKED: u32 = 0;
const MUTEX_LOCKED: u32 = 1;
const MUTEX_LOCKED_WAITERS: u32 = 2;

// ---- Thread Control Block ---------------------------------------------------

/// Thread Control Block: stored at the TLS pointer (FS_BASE).
///
/// The self-pointer MUST be the first field (x86-64 TLS convention).
#[repr(C, align(16))]
struct Tcb {
    /// Self-pointer (x86-64 ABI: `%fs:0` = pointer to the TCB itself).
    self_ptr: *mut Tcb,
    /// Thread ID returned by `task_clone`.
    tid: u32,
    /// Stack base (for cleanup on thread exit — currently unused).
    _stack_base: usize,
    /// The user thread function.
    start_fn: unsafe extern "C" fn(*mut u8) -> *mut u8,
    /// Argument to pass to the thread function.
    arg: *mut u8,
    /// Per-thread key values (indexed by `pthread_key_t`).
    tls_values: [*mut u8; MAX_PTHREAD_KEYS],
}

// SAFETY: Tcb is only ever accessed by the owning thread (via FS register)
// or the parent after join. The raw pointers inside are valid for the
// thread's lifetime.
unsafe impl Send for Tcb {}
unsafe impl Sync for Tcb {}

// ---- Key allocator ----------------------------------------------------------

/// Global key generation counter. Keys are never reused in this minimal impl.
static NEXT_KEY: AtomicU32 = AtomicU32::new(0);

// ---- TCB helpers ------------------------------------------------------------

/// Returns a pointer to the current thread's TCB by reading `%fs:0`.
///
/// # Safety
///
/// Must only be called from a thread that has a valid TCB set up
/// (i.e. not before `pthread_create` or `_start` sets FS_BASE).
unsafe fn current_tcb() -> *mut Tcb {
    let ptr: *mut Tcb;
    // SAFETY: FS_BASE was set by task_clone(CLONE_SETTLS) or by the kernel
    // for the initial thread. Reading FS:0 yields the self-pointer.
    unsafe {
        core::arch::asm!(
            "mov {}, qword ptr fs:[0]",
            out(reg) ptr,
            options(nostack, preserves_flags),
        );
    }
    ptr
}

/// Sets the current thread's FS_BASE to point to `tcb`.
///
/// On Hadron, the kernel sets FS_BASE from `tls_ptr` at thread creation.
/// We only need this for the main thread, which did not go through
/// `task_clone`.
#[allow(dead_code)] // Phase 2: called from thread-initialisation path
unsafe fn set_current_tcb(tcb: *mut Tcb) {
    // SAFETY: `tcb` is valid; arch_prctl / WRFSBASE would be the clean way
    // but Hadron exposes FS_BASE configuration only via task_clone.
    // For the main thread, FS_BASE may already be set by the kernel.
    // This is a no-op stub — the main thread accesses keys via a fallback.
    let _ = tcb;
}

// ---- Stack allocation -------------------------------------------------------

/// Allocate a stack of `size` bytes using anonymous mmap.
///
/// Returns a pointer to the **base** (lowest address) of the mapping.
unsafe fn alloc_stack(size: usize) -> Option<*mut u8> {
    use crate::flags::{MAP_ANONYMOUS, MAP_PRIVATE, PROT_READ, PROT_WRITE};
    let prot = (PROT_READ | PROT_WRITE) as usize;
    let flags_hadron = crate::flags::posix_mmap_to_hadron(MAP_ANONYMOUS | MAP_PRIVATE);
    match sys::sys_mmap(0, size, prot, flags_hadron, usize::MAX) {
        Ok(ptr) if !ptr.is_null() => Some(ptr),
        _ => None,
    }
}

// ---- pthread_create / pthread_join / pthread_self ---------------------------

/// Create a new thread.
///
/// Allocates a stack + TCB, stores the start function and argument in the TCB,
/// then calls `task_clone`. Returns 0 on success or a positive errno on error.
///
/// # Safety
///
/// `thread` must be a valid, writable pointer. `start_routine` and `arg` must
/// satisfy the threading contract (the function runs until it returns or
/// `pthread_exit` is called).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_create(
    thread: *mut u64,
    attr: *const PthreadAttr,
    start_routine: unsafe extern "C" fn(*mut u8) -> *mut u8,
    arg: *mut u8,
) -> i32 {
    let stack_size = if attr.is_null() {
        DEFAULT_STACK_SIZE
    } else {
        let s = unsafe { (*attr).stack_size };
        if s == 0 { DEFAULT_STACK_SIZE } else { s }
    };

    // Align stack_size up to page boundary (4 KiB).
    let stack_size = (stack_size + 0xFFF) & !0xFFF;

    // Allocate stack + TCB in one contiguous region.
    // Layout: [guard gap] [stack grows down] [TCB at top]
    // We put the TCB just above the stack (at the highest address),
    // so stack_top = region_base + stack_size, TCB starts at stack_top.
    let tcb_size = core::mem::size_of::<Tcb>();
    let tcb_size = (tcb_size + TCB_ALIGN - 1) & !(TCB_ALIGN - 1);
    let total = stack_size + tcb_size;

    let region = unsafe { alloc_stack(total) };
    let Some(region_base) = region else {
        return crate::errno::ENOMEM.0;
    };

    // Stack occupies [region_base, region_base + stack_size).
    // The stack pointer passed to task_clone is the TOP of the stack
    // (stacks grow downward on x86-64). We leave 16 bytes headroom for
    // the red zone / alignment.
    let stack_top = region_base.add(stack_size).sub(16) as usize;

    // TCB is placed at [region_base + stack_size, region_base + total).
    // SAFETY: region_base points to a valid, writable mmap region of `total`
    // bytes; the TCB fits within that region.
    let tcb = region_base.add(stack_size) as *mut Tcb;
    unsafe {
        core::ptr::write_bytes(tcb as *mut u8, 0, tcb_size);
        (*tcb).self_ptr = tcb;
        (*tcb).tid = 0;
        (*tcb)._stack_base = region_base as usize;
        (*tcb).start_fn = start_routine;
        (*tcb).arg = arg;
    }

    let flags = CLONE_VM | CLONE_FILES | CLONE_SIGHAND | CLONE_SETTLS;
    let child_tid = unsafe { sys::sys_task_clone(flags, stack_top, tcb as usize) };

    match child_tid {
        Ok(0) => {
            // We are the child thread.
            // Read the TCB via FS_BASE (the kernel set it to `tcb` above).
            let t = unsafe { current_tcb() };
            let f = unsafe { (*t).start_fn };
            let a = unsafe { (*t).arg };
            // Call user function; ignore return value for now.
            let _ = unsafe { f(a) };
            // Terminate this thread.
            unsafe { sys::sys_exit(0) }
        }
        Ok(child_tid) => {
            // We are the parent.
            unsafe { *thread = child_tid as u64 };
            0
        }
        Err(e) => e.0,
    }
}

/// Wait for a thread to terminate.
///
/// `retval` is currently ignored (thread return values are not propagated
/// in this minimal implementation).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_join(thread: u64, retval: *mut *mut u8) -> i32 {
    // task_wait with pid = thread TID, WNOHANG = 0 (blocking).
    let _ = retval;
    match sys::sys_waitpid(thread as usize, core::ptr::null_mut(), 0) {
        Ok(_) => 0,
        Err(e) => e.0,
    }
}

/// Return the calling thread's identifier.
///
/// Uses `task_info` syscall which returns the current TID.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_self() -> u64 {
    sys::sys_getpid() as u64
}

/// Terminate the calling thread.
///
/// # Safety
///
/// This function does not return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_exit(_retval: *mut u8) -> ! {
    unsafe { sys::sys_exit(0) }
}

/// Detach a thread (marks it so resources are freed on exit automatically).
///
/// In this minimal implementation, detach is a no-op because we do not
/// maintain a join-state table. Threads that are never joined will have
/// their kernel resources reclaimed when the process exits.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_detach(_thread: u64) -> i32 {
    0
}

/// Compare two thread IDs.
#[unsafe(no_mangle)]
pub extern "C" fn pthread_equal(t1: u64, t2: u64) -> i32 {
    if t1 == t2 { 1 } else { 0 }
}

// ---- Thread attributes ------------------------------------------------------

/// Opaque thread attribute structure (C-visible layout).
#[repr(C)]
pub struct PthreadAttr {
    stack_size: usize,
    _opaque: [u8; 56],
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_init(attr: *mut PthreadAttr) -> i32 {
    if attr.is_null() {
        return crate::errno::EINVAL.0;
    }
    unsafe {
        core::ptr::write_bytes(attr as *mut u8, 0, core::mem::size_of::<PthreadAttr>());
        (*attr).stack_size = DEFAULT_STACK_SIZE;
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_destroy(_attr: *mut PthreadAttr) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_setstacksize(
    attr: *mut PthreadAttr,
    stacksize: usize,
) -> i32 {
    if attr.is_null() || stacksize < 4096 {
        return crate::errno::EINVAL.0;
    }
    unsafe { (*attr).stack_size = stacksize };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_attr_getstacksize(
    attr: *const PthreadAttr,
    stacksize: *mut usize,
) -> i32 {
    if attr.is_null() || stacksize.is_null() {
        return crate::errno::EINVAL.0;
    }
    unsafe { *stacksize = (*attr).stack_size };
    0
}

// ---- Mutex ------------------------------------------------------------------

/// C-visible mutex layout: 40 bytes (glibc-size-compatible).
#[repr(C)]
pub struct PthreadMutex {
    lock: AtomicU32,
    _pad: [u8; 36],
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_init(
    mutex: *mut PthreadMutex,
    _attr: *const PthreadMutexAttr,
) -> i32 {
    if mutex.is_null() {
        return crate::errno::EINVAL.0;
    }
    unsafe {
        core::ptr::write_bytes(mutex as *mut u8, 0, core::mem::size_of::<PthreadMutex>());
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_destroy(_mutex: *mut PthreadMutex) -> i32 {
    0
}

/// Lock a mutex.
///
/// Uses a two-state futex:
/// - `0` → try to CAS to `1`; success = acquired
/// - `1` or `2` → CAS to `2`; then `futex(WAIT, 2)` until woken
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_lock(mutex: *mut PthreadMutex) -> i32 {
    if mutex.is_null() {
        return crate::errno::EINVAL.0;
    }
    let lock = unsafe { &(*mutex).lock };
    // Fast path: try to grab from unlocked state.
    if lock
        .compare_exchange(
            MUTEX_UNLOCKED,
            MUTEX_LOCKED,
            Ordering::Acquire,
            Ordering::Relaxed,
        )
        .is_ok()
    {
        return 0;
    }
    // Slow path: mark waiters present then sleep on the futex.
    loop {
        let prev = lock.swap(MUTEX_LOCKED_WAITERS, Ordering::Acquire);
        if prev == MUTEX_UNLOCKED {
            // We grabbed the lock while setting it to LOCKED_WAITERS.
            return 0;
        }
        // Sleep until woken (ignore return value — spurious wakeups are OK).
        let _ = sys::sys_futex(
            lock.as_ptr(),
            FUTEX_WAIT,
            MUTEX_LOCKED_WAITERS as usize,
            core::ptr::null(),
        );
    }
}

/// Try to lock a mutex without blocking.
///
/// Returns 0 on success, `EBUSY` if the mutex is already locked.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_trylock(mutex: *mut PthreadMutex) -> i32 {
    if mutex.is_null() {
        return crate::errno::EINVAL.0;
    }
    let lock = unsafe { &(*mutex).lock };
    match lock.compare_exchange(
        MUTEX_UNLOCKED,
        MUTEX_LOCKED,
        Ordering::Acquire,
        Ordering::Relaxed,
    ) {
        Ok(_) => 0,
        Err(_) => crate::errno::EBUSY.0,
    }
}

/// Unlock a mutex.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutex_unlock(mutex: *mut PthreadMutex) -> i32 {
    if mutex.is_null() {
        return crate::errno::EINVAL.0;
    }
    let lock = unsafe { &(*mutex).lock };
    let prev = lock.swap(MUTEX_UNLOCKED, Ordering::Release);
    if prev == MUTEX_LOCKED_WAITERS {
        // There were waiters — wake one.
        let _ = sys::sys_futex(lock.as_ptr(), FUTEX_WAKE, 1, core::ptr::null());
    }
    0
}

// ---- Mutex attributes -------------------------------------------------------

#[repr(C)]
pub struct PthreadMutexAttr {
    kind: i32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_init(attr: *mut PthreadMutexAttr) -> i32 {
    if attr.is_null() {
        return crate::errno::EINVAL.0;
    }
    unsafe { (*attr).kind = 0 };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_destroy(_attr: *mut PthreadMutexAttr) -> i32 {
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_mutexattr_settype(attr: *mut PthreadMutexAttr, kind: i32) -> i32 {
    if attr.is_null() {
        return crate::errno::EINVAL.0;
    }
    unsafe { (*attr).kind = kind };
    0
}

// ---- Condition variable -----------------------------------------------------

/// C-visible condvar layout: two u32s.
#[repr(C)]
pub struct PthreadCond {
    seq: AtomicU32,
    waiters: AtomicU32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_init(
    cond: *mut PthreadCond,
    _attr: *const PthreadCondAttr,
) -> i32 {
    if cond.is_null() {
        return crate::errno::EINVAL.0;
    }
    unsafe {
        core::ptr::write_bytes(cond as *mut u8, 0, core::mem::size_of::<PthreadCond>());
    }
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_destroy(_cond: *mut PthreadCond) -> i32 {
    0
}

/// Wait on a condition variable.
///
/// Atomically releases `mutex`, waits until signalled, then re-acquires.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_wait(
    cond: *mut PthreadCond,
    mutex: *mut PthreadMutex,
) -> i32 {
    if cond.is_null() || mutex.is_null() {
        return crate::errno::EINVAL.0;
    }
    let cond_seq = unsafe { &(*cond).seq };
    let cond_waiters = unsafe { &(*cond).waiters };

    // Record the current sequence before releasing the mutex.
    let seq = cond_seq.load(Ordering::Relaxed);
    cond_waiters.fetch_add(1, Ordering::Relaxed);

    // Release the mutex.
    unsafe { pthread_mutex_unlock(mutex) };

    // Sleep while the sequence has not changed.
    loop {
        let _ = sys::sys_futex(
            cond_seq.as_ptr(),
            FUTEX_WAIT,
            seq as usize,
            core::ptr::null(),
        );
        if cond_seq.load(Ordering::Acquire) != seq {
            break;
        }
    }

    cond_waiters.fetch_sub(1, Ordering::Relaxed);

    // Re-acquire the mutex.
    unsafe { pthread_mutex_lock(mutex) };
    0
}

/// Signal one waiter on a condition variable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_signal(cond: *mut PthreadCond) -> i32 {
    if cond.is_null() {
        return crate::errno::EINVAL.0;
    }
    let cond_seq = unsafe { &(*cond).seq };
    cond_seq.fetch_add(1, Ordering::Release);
    let _ = sys::sys_futex(cond_seq.as_ptr(), FUTEX_WAKE, 1, core::ptr::null());
    0
}

/// Wake all waiters on a condition variable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_cond_broadcast(cond: *mut PthreadCond) -> i32 {
    if cond.is_null() {
        return crate::errno::EINVAL.0;
    }
    let cond_seq = unsafe { &(*cond).seq };
    cond_seq.fetch_add(1, Ordering::Release);
    let _ = sys::sys_futex(
        cond_seq.as_ptr(),
        FUTEX_WAKE,
        i32::MAX as usize,
        core::ptr::null(),
    );
    0
}

// ---- Condition variable attributes ------------------------------------------

#[repr(C)]
pub struct PthreadCondAttr {
    _unused: i32,
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_init(attr: *mut PthreadCondAttr) -> i32 {
    if attr.is_null() {
        crate::errno::EINVAL.0
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_condattr_destroy(_attr: *mut PthreadCondAttr) -> i32 {
    0
}

// ---- pthread_once -----------------------------------------------------------

/// Run `init_routine` exactly once for the given `once_control`.
///
/// State machine: `ONCE_UNINITIALIZED` (0) → `ONCE_IN_PROGRESS` (1) →
/// `ONCE_DONE` (2). Concurrent callers in state 1 sleep on a futex.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_once(
    once_control: *mut u32,
    init_routine: unsafe extern "C" fn(),
) -> i32 {
    if once_control.is_null() {
        return crate::errno::EINVAL.0;
    }
    let ctrl = unsafe { &*(once_control as *const AtomicU32) };

    loop {
        match ctrl.load(Ordering::Acquire) {
            ONCE_DONE => return 0,
            ONCE_UNINITIALIZED => {
                // Try to claim the slot.
                if ctrl
                    .compare_exchange(
                        ONCE_UNINITIALIZED,
                        ONCE_IN_PROGRESS,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    )
                    .is_ok()
                {
                    // We won the race; run the initializer.
                    unsafe { init_routine() };
                    ctrl.store(ONCE_DONE, Ordering::Release);
                    // Wake any threads waiting in state IN_PROGRESS.
                    let _ = sys::sys_futex(
                        ctrl.as_ptr(),
                        FUTEX_WAKE,
                        i32::MAX as usize,
                        core::ptr::null(),
                    );
                    return 0;
                }
                // Lost the race; someone else is running init. Fall through
                // to ONCE_IN_PROGRESS handling.
            }
            _ => {
                // ONCE_IN_PROGRESS: sleep until the initializer finishes.
                let _ = sys::sys_futex(
                    ctrl.as_ptr(),
                    FUTEX_WAIT,
                    ONCE_IN_PROGRESS as usize,
                    core::ptr::null(),
                );
            }
        }
    }
}

// ---- Thread-specific data (pthread_key_*) -----------------------------------
//
// Key values are stored per-thread in the Tcb::tls_values array. When no TCB
// is available (main thread before TCB initialisation), we fall back to a
// small static table for the main-thread case. This is sufficient for Mesa
// which typically uses fewer than 8 keys.

/// Global key validity bitmap: bit `k` is set when key `k` is live.
static KEY_LIVE: AtomicU32 = AtomicU32::new(0);

/// Per-main-thread fallback key values (main thread only, up to 32 keys).
static mut MAIN_THREAD_VALUES: [*mut u8; 32] = [core::ptr::null_mut(); 32];

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_key_create(
    key: *mut u32,
    _destructor: *const u8, // fn(*mut void) — ignored in Phase 1
) -> i32 {
    if key.is_null() {
        return crate::errno::EINVAL.0;
    }
    let k = NEXT_KEY.fetch_add(1, Ordering::Relaxed);
    if k as usize >= MAX_PTHREAD_KEYS {
        return crate::errno::EAGAIN.0;
    }
    KEY_LIVE.fetch_or(1 << (k & 31), Ordering::Relaxed);
    unsafe { *key = k };
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_key_delete(key: u32) -> i32 {
    if key as usize >= MAX_PTHREAD_KEYS {
        return crate::errno::EINVAL.0;
    }
    KEY_LIVE.fetch_and(!(1 << (key & 31)), Ordering::Relaxed);
    0
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_getspecific(key: u32) -> *mut u8 {
    if key as usize >= MAX_PTHREAD_KEYS {
        return core::ptr::null_mut();
    }
    // Try to read from the TCB if FS_BASE is set.
    let tcb = unsafe { current_tcb() };
    if !tcb.is_null() && unsafe { (*tcb).self_ptr } == tcb {
        return unsafe { (*tcb).tls_values[key as usize] };
    }
    // Fallback: main thread without TCB (up to 32 keys).
    if key < 32 {
        return unsafe { MAIN_THREAD_VALUES[key as usize] };
    }
    core::ptr::null_mut()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pthread_setspecific(key: u32, value: *const u8) -> i32 {
    if key as usize >= MAX_PTHREAD_KEYS {
        return crate::errno::EINVAL.0;
    }
    let tcb = unsafe { current_tcb() };
    if !tcb.is_null() && unsafe { (*tcb).self_ptr } == tcb {
        unsafe { (*tcb).tls_values[key as usize] = value.cast_mut() };
        return 0;
    }
    // Fallback: main thread (up to 32 keys).
    if key < 32 {
        // SAFETY: MAIN_THREAD_VALUES is only written from the main thread or
        // before any threads are created — no concurrent access.
        unsafe { MAIN_THREAD_VALUES[key as usize] = value.cast_mut() };
        return 0;
    }
    crate::errno::EINVAL.0
}

// ---- sched_yield ------------------------------------------------------------

/// Yield the processor to another thread.
///
/// Implemented as a `futex(WAIT)` with timeout 0, which returns immediately
/// after giving the scheduler a hint to pick another runnable task.
#[unsafe(no_mangle)]
pub extern "C" fn sched_yield() -> i32 {
    // A timeout of 0 returns immediately; we use this as a lightweight yield.
    // On a future kernel with a dedicated yield syscall this should be updated.
    let mut dummy: u32 = 0;
    let _ = sys::sys_futex(
        &raw mut dummy,
        FUTEX_WAIT,
        1, // val != dummy, so kernel returns EAGAIN immediately
        core::ptr::null(),
    );
    0
}
