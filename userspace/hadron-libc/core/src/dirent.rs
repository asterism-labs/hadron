//! Directory traversal functions.
//!
//! POSIX functions: `opendir`, `readdir`, `closedir`.

use crate::errno;
use crate::sys;

/// Maximum filename length.
const NAME_MAX: usize = 255;

/// Directory entry returned by `readdir`.
#[repr(C)]
pub struct Dirent {
    /// Inode number (0 if unavailable).
    pub d_ino: u64,
    /// File type (DT_REG, DT_DIR, etc.).
    pub d_type: u8,
    /// Filename (NUL-terminated).
    pub d_name: [u8; NAME_MAX + 1],
}

/// An open directory stream.
pub struct Dir {
    fd: i32,
    /// Raw kernel readdir buffer.
    buf: [u8; 4096],
    /// Current position in buf.
    pos: usize,
    /// Valid bytes in buf.
    len: usize,
    /// Scratch dirent for readdir to return.
    entry: Dirent,
}

/// Open a directory stream.
///
/// # Safety
///
/// `name` must be a valid NUL-terminated path.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn opendir(name: *const u8) -> *mut Dir {
    if name.is_null() {
        errno::set_errno(crate::errno::EINVAL);
        return core::ptr::null_mut();
    }
    let name_len = unsafe { crate::string::strlen(name) };
    let slice = unsafe { core::slice::from_raw_parts(name, name_len) };
    let flags =
        crate::flags::posix_open_to_hadron(crate::flags::O_RDONLY | crate::flags::O_DIRECTORY);
    let fd = match sys::sys_open(slice, flags) {
        Ok(fd) => fd,
        Err(e) => {
            errno::set_errno(e);
            return core::ptr::null_mut();
        }
    };

    // Allocate Dir struct.
    let dir_ptr = unsafe { crate::alloc::malloc(core::mem::size_of::<Dir>()) } as *mut Dir;
    if dir_ptr.is_null() {
        let _ = sys::sys_close(fd);
        errno::set_errno(crate::errno::ENOMEM);
        return core::ptr::null_mut();
    }

    // SAFETY: dir_ptr is valid allocated memory.
    unsafe {
        core::ptr::write(
            dir_ptr,
            Dir {
                fd: fd as i32,
                buf: [0; 4096],
                pos: 0,
                len: 0,
                entry: Dirent {
                    d_ino: 0,
                    d_type: 0,
                    d_name: [0; NAME_MAX + 1],
                },
            },
        );
    }
    dir_ptr
}

/// Read the next directory entry.
///
/// Returns a pointer to a `Dirent`, or null on end-of-directory or error.
///
/// # Safety
///
/// `dirp` must be a valid `Dir` pointer from `opendir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn readdir(dirp: *mut Dir) -> *mut Dirent {
    if dirp.is_null() {
        return core::ptr::null_mut();
    }
    // SAFETY: dirp is valid.
    let d = unsafe { &mut *dirp };

    loop {
        // If buffer is exhausted, refill.
        if d.pos >= d.len {
            match sys::sys_readdir(d.fd as usize, d.buf.as_mut_ptr(), d.buf.len()) {
                Ok(0) => return core::ptr::null_mut(), // End of directory.
                Ok(n) => {
                    d.pos = 0;
                    d.len = n;
                }
                Err(e) => {
                    errno::set_errno(e);
                    return core::ptr::null_mut();
                }
            }
        }

        // Parse one entry from the buffer.
        // Hadron readdir format: each entry is a NUL-terminated filename.
        // Find the end of the current entry.
        let start = d.pos;
        while d.pos < d.len && d.buf[d.pos] != 0 {
            d.pos += 1;
        }
        let name_len = d.pos - start;
        if d.pos < d.len {
            d.pos += 1; // skip NUL
        }

        if name_len == 0 {
            continue; // skip empty entries
        }

        // Fill in the dirent.
        d.entry.d_ino = 0; // Inode not provided by basic readdir.
        d.entry.d_type = 0; // Type unknown from basic readdir.
        let copy_len = name_len.min(NAME_MAX);
        d.entry.d_name[..copy_len].copy_from_slice(&d.buf[start..start + copy_len]);
        d.entry.d_name[copy_len] = 0;

        return &raw mut d.entry;
    }
}

/// Close a directory stream.
///
/// # Safety
///
/// `dirp` must be a valid `Dir` pointer from `opendir`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn closedir(dirp: *mut Dir) -> i32 {
    if dirp.is_null() {
        return -1;
    }
    // SAFETY: dirp is valid.
    let d = unsafe { &*dirp };
    let ret = match sys::sys_close(d.fd as usize) {
        Ok(()) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    };
    unsafe { crate::alloc::free(dirp.cast()) };
    ret
}
