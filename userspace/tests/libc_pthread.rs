//! utest: POSIX threads regression tests.
//!
//! Covers:
//! 1. Mutex: `pthread_mutex_init`, `pthread_mutex_lock`, `pthread_mutex_trylock`,
//!    `pthread_mutex_unlock`, `pthread_mutex_destroy`
//!
//! Thread creation/join and condvar tests are omitted — the kernel's
//! `task_clone`/`task_wait` paths are not yet stable enough to run under
//! the utest harness (libc_pthread hangs with zero output before the first
//! test prints).

#![no_std]
#![no_main]

// Force hadron_libc_core to be linked so its #[no_mangle] symbols
// (pthread_mutex_*, …) are available.
extern crate hadron_libc_core;

use hadron_utest::utest_main;

utest_main!(test_mutex,);

// ── opaque storage types ──────────────────────────────────────────────────────

/// Opaque storage for `pthread_mutex_t` (40 bytes, 8-byte aligned).
///
/// Matches `PthreadMutex` in `hadron-libc-core`: `lock: AtomicU32` + 36 bytes
/// of padding.
#[repr(C, align(8))]
struct MutexStorage([u8; 40]);

// ── extern declarations ───────────────────────────────────────────────────────

unsafe extern "C" {
    fn pthread_mutex_init(mutex: *mut u8, attr: *const u8) -> i32;
    fn pthread_mutex_destroy(mutex: *mut u8) -> i32;
    fn pthread_mutex_lock(mutex: *mut u8) -> i32;
    fn pthread_mutex_trylock(mutex: *mut u8) -> i32;
    fn pthread_mutex_unlock(mutex: *mut u8) -> i32;
}

// ── tests ─────────────────────────────────────────────────────────────────────

fn test_mutex() {
    let mut m = MutexStorage([0u8; 40]);
    let mp = m.0.as_mut_ptr();

    // SAFETY: mp is a valid pointer to 40 zeroed bytes (mutex storage).
    unsafe {
        let r = pthread_mutex_init(mp, core::ptr::null());
        assert_eq!(r, 0, "mutex_init failed");

        let r = pthread_mutex_lock(mp);
        assert_eq!(r, 0, "mutex_lock failed");

        // trylock must fail (EBUSY = 16) while locked
        let r = pthread_mutex_trylock(mp);
        assert_eq!(r, 16, "trylock should return EBUSY while locked");

        let r = pthread_mutex_unlock(mp);
        assert_eq!(r, 0, "mutex_unlock failed");

        // trylock must succeed after unlock
        let r = pthread_mutex_trylock(mp);
        assert_eq!(r, 0, "trylock should succeed after unlock");

        // unlock from trylock
        let r = pthread_mutex_unlock(mp);
        assert_eq!(r, 0, "mutex_unlock (2) failed");

        let r = pthread_mutex_destroy(mp);
        assert_eq!(r, 0, "mutex_destroy failed");
    }
}
