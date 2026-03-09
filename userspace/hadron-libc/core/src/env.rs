//! Environment variable functions.
//!
//! POSIX functions: `getenv`, `setenv`, `unsetenv`.
//! Global: `environ`.

use crate::errno;

/// Maximum number of environment variables.
const MAX_ENV: usize = 256;

/// The environment array. NULL-terminated array of `"KEY=VALUE\0"` pointers.
///
/// Initialized by `_start` from the kernel-provided envp.
static mut ENVIRON: [*mut u8; MAX_ENV + 1] = [core::ptr::null_mut(); MAX_ENV + 1];

/// Public `environ` pointer (POSIX).
///
/// Initialized by `init_environ()` to point to the `ENVIRON` array.
#[unsafe(no_mangle)]
pub static mut environ: *mut *mut u8 = core::ptr::null_mut();

/// Initialize the environment from envp provided by the kernel.
///
/// # Safety
///
/// `envp` must be a NULL-terminated array of NUL-terminated C strings.
pub unsafe fn init_environ(envp: *const *const u8) {
    // Set the environ pointer to our static array.
    unsafe { environ = (&raw mut ENVIRON).cast() };

    if envp.is_null() {
        return;
    }
    let mut i = 0;
    // SAFETY: envp is NULL-terminated.
    while i < MAX_ENV {
        let p = unsafe { *envp.add(i) };
        if p.is_null() {
            break;
        }
        // Store pointer directly — the strings live on the initial stack
        // and remain valid for the process lifetime.
        unsafe { ENVIRON[i] = p as *mut u8 };
        i += 1;
    }
    // SAFETY: i <= MAX_ENV.
    unsafe { ENVIRON[i] = core::ptr::null_mut() };
}

/// Get the value of an environment variable.
///
/// # Safety
///
/// `name` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn getenv(name: *const u8) -> *const u8 {
    if name.is_null() {
        return core::ptr::null();
    }
    // SAFETY: name is NUL-terminated.
    let name_len = unsafe { crate::string::strlen(name) };
    if name_len == 0 {
        return core::ptr::null();
    }

    let mut i = 0;
    loop {
        // SAFETY: ENVIRON is valid up to the NULL terminator.
        let entry = unsafe { ENVIRON[i] };
        if entry.is_null() {
            break;
        }
        // Check if entry starts with "name=".
        let entry_matches = unsafe {
            crate::string::strncmp(entry, name, name_len) == 0 && *entry.add(name_len) == b'='
        };
        if entry_matches {
            // Return pointer past "name=".
            return unsafe { entry.add(name_len + 1) };
        }
        i += 1;
    }
    core::ptr::null()
}

/// Set an environment variable.
///
/// # Safety
///
/// `name` and `value` must be valid NUL-terminated strings.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn setenv(name: *const u8, value: *const u8, overwrite: i32) -> i32 {
    if name.is_null() || value.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let name_len = unsafe { crate::string::strlen(name) };
    let value_len = unsafe { crate::string::strlen(value) };

    // Check for '=' in name (invalid).
    for j in 0..name_len {
        if unsafe { *name.add(j) } == b'=' {
            errno::set_errno(crate::errno::EINVAL);
            return -1;
        }
    }

    // Search for existing entry.
    let mut idx: Option<usize> = None;
    let mut count = 0;
    loop {
        let entry = unsafe { ENVIRON[count] };
        if entry.is_null() {
            break;
        }
        if idx.is_none() {
            let entry_matches = unsafe {
                crate::string::strncmp(entry, name, name_len) == 0 && *entry.add(name_len) == b'='
            };
            if entry_matches {
                idx = Some(count);
            }
        }
        count += 1;
    }

    if let Some(i) = idx {
        if overwrite == 0 {
            return 0; // Don't overwrite.
        }
        // Allocate new "name=value\0".
        let total = name_len + 1 + value_len + 1;
        let buf = unsafe { crate::alloc::malloc(total) };
        if buf.is_null() {
            errno::set_errno(crate::errno::ENOMEM);
            return -1;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(name, buf, name_len);
            *buf.add(name_len) = b'=';
            core::ptr::copy_nonoverlapping(value, buf.add(name_len + 1), value_len);
            *buf.add(total - 1) = 0;
            ENVIRON[i] = buf;
        }
        0
    } else {
        // Add new entry.
        if count >= MAX_ENV {
            errno::set_errno(crate::errno::ENOMEM);
            return -1;
        }
        let total = name_len + 1 + value_len + 1;
        let buf = unsafe { crate::alloc::malloc(total) };
        if buf.is_null() {
            errno::set_errno(crate::errno::ENOMEM);
            return -1;
        }
        unsafe {
            core::ptr::copy_nonoverlapping(name, buf, name_len);
            *buf.add(name_len) = b'=';
            core::ptr::copy_nonoverlapping(value, buf.add(name_len + 1), value_len);
            *buf.add(total - 1) = 0;
            ENVIRON[count] = buf;
            ENVIRON[count + 1] = core::ptr::null_mut();
        }
        0
    }
}

/// Remove an environment variable.
///
/// # Safety
///
/// `name` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn unsetenv(name: *const u8) -> i32 {
    if name.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return -1;
    }
    let name_len = unsafe { crate::string::strlen(name) };

    let mut i = 0;
    loop {
        let entry = unsafe { ENVIRON[i] };
        if entry.is_null() {
            break;
        }
        let entry_matches = unsafe {
            crate::string::strncmp(entry, name, name_len) == 0 && *entry.add(name_len) == b'='
        };
        if entry_matches {
            // Shift remaining entries down.
            let mut j = i;
            loop {
                unsafe { ENVIRON[j] = ENVIRON[j + 1] };
                if unsafe { ENVIRON[j] }.is_null() {
                    break;
                }
                j += 1;
            }
            // Don't increment i; re-check the position.
            continue;
        }
        i += 1;
    }
    0
}
