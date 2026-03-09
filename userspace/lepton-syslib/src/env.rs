//! Environment variable management for userspace processes.
//!
//! Stores environment variables in a `BTreeMap<String, String>`. Initialized
//! from the envp array passed on the stack by the kernel. Provides `getenv`,
//! `setenv`, `unsetenv`, and iteration.

extern crate alloc;

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;
use core::cell::UnsafeCell;

/// Wrapper to hold the environment map in a global static.
///
/// Single-threaded: Hadron userspace processes are single-threaded, so
/// no synchronization is needed.
struct EnvStorage(UnsafeCell<Option<BTreeMap<String, String>>>);

// SAFETY: Hadron userspace is single-threaded. No concurrent access.
unsafe impl Sync for EnvStorage {}

/// Global environment variable storage.
static ENV: EnvStorage = EnvStorage(UnsafeCell::new(None));

/// Get a reference to the environment map.
///
/// # Safety
///
/// Must only be called after [`init`] and from a single thread.
fn env_map() -> &'static BTreeMap<String, String> {
    // SAFETY: Single-threaded access; ENV is initialized before main().
    unsafe {
        (*ENV.0.get())
            .as_ref()
            .expect("environment not initialized")
    }
}

/// Get a mutable reference to the environment map.
///
/// # Safety
///
/// Must only be called after [`init`] and from a single thread.
fn env_map_mut() -> &'static mut BTreeMap<String, String> {
    // SAFETY: Single-threaded access; ENV is initialized before main().
    unsafe {
        (*ENV.0.get())
            .as_mut()
            .expect("environment not initialized")
    }
}

/// Initialize the environment from the envp array.
///
/// Each entry in `envp` is a `KEY=value` string. Called once from `_start_rust`.
pub fn init(envp: &[&str]) {
    let mut map = BTreeMap::new();
    for entry in envp {
        if let Some(eq_pos) = entry.find('=') {
            let key = &entry[..eq_pos];
            let value = &entry[eq_pos + 1..];
            map.insert(String::from(key), String::from(value));
        }
    }
    // SAFETY: Called once during single-threaded startup.
    unsafe {
        *ENV.0.get() = Some(map);
    }
}

/// Get the value of an environment variable.
pub fn getenv(key: &str) -> Option<&'static str> {
    env_map().get(key).map(|s| s.as_str())
}

/// Set an environment variable.
pub fn setenv(key: &str, value: &str) {
    env_map_mut().insert(String::from(key), String::from(value));
}

/// Remove an environment variable.
pub fn unsetenv(key: &str) {
    env_map_mut().remove(key);
}

/// Iterate over all environment variables as `(key, value)` pairs.
///
/// The callback receives each key-value pair.
pub fn for_each(mut f: impl FnMut(&str, &str)) {
    for (k, v) in env_map() {
        f(k, v);
    }
}

/// Build a vector of `KEY=value` strings for passing to a child process.
pub fn build_env_block() -> Vec<String> {
    let mut result = Vec::new();
    for (k, v) in env_map() {
        let mut entry = String::with_capacity(k.len() + 1 + v.len());
        entry.push_str(k);
        entry.push('=');
        entry.push_str(v);
        result.push(entry);
    }
    result
}

/// Return the number of environment variables.
pub fn count() -> usize {
    env_map().len()
}
