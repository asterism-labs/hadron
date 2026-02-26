//! Buffered I/O (FILE streams) and standard I/O functions.
//!
//! Provides `FILE` struct with buffering, `stdin`/`stdout`/`stderr` statics,
//! and POSIX functions: `fopen`, `fclose`, `fread`, `fwrite`, `fputc`, `fputs`,
//! `fgets`, `fflush`, `feof`, `ferror`, `clearerr`, `puts`, `putchar`.

pub mod printf;

use crate::errno::{self, EBADF, EINVAL, ENOMEM};
use crate::sys;

const BUF_SIZE: usize = 4096;

/// Buffering mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BufMode {
    FullyBuffered = 0,
    LineBuffered = 1,
    Unbuffered = 2,
}

/// Flags for FILE state.
const FILE_READ: u32 = 0x01;
const FILE_WRITE: u32 = 0x02;
const FILE_EOF: u32 = 0x04;
const FILE_ERROR: u32 = 0x08;
const FILE_OWNED_FD: u32 = 0x10;

/// A buffered I/O stream.
#[repr(C)]
pub struct FILE {
    fd: i32,
    buf: [u8; BUF_SIZE],
    /// For write: bytes pending in buffer. For read: bytes available.
    buf_pos: usize,
    /// For read: total valid bytes in buffer.
    buf_len: usize,
    buf_mode: BufMode,
    flags: u32,
}

// SAFETY: Single-threaded Phase 1.
unsafe impl Sync for FILE {}

impl FILE {
    const fn new(fd: i32, mode: BufMode, flags: u32) -> Self {
        Self {
            fd,
            buf: [0; BUF_SIZE],
            buf_pos: 0,
            buf_len: 0,
            buf_mode: mode,
            flags,
        }
    }
}

static mut STDIN_BUF: FILE = FILE::new(0, BufMode::Unbuffered, FILE_READ);
static mut STDOUT_BUF: FILE = FILE::new(1, BufMode::LineBuffered, FILE_WRITE);
static mut STDERR_BUF: FILE = FILE::new(2, BufMode::Unbuffered, FILE_WRITE);

/// Initialize stdio streams. Called from `_start`.
pub fn init() {
    // Streams are initialized with const defaults, nothing else needed.
}

// ---- Public stream accessors -------------------------------------------------

/// Get stdin FILE pointer.
#[unsafe(no_mangle)]
pub extern "C" fn __stdin() -> *mut FILE {
    // SAFETY: Single-threaded access.
    unsafe { &raw mut STDIN_BUF }
}

/// Get stdout FILE pointer.
#[unsafe(no_mangle)]
pub extern "C" fn __stdout() -> *mut FILE {
    // SAFETY: Single-threaded access.
    unsafe { &raw mut STDOUT_BUF }
}

/// Get stderr FILE pointer.
#[unsafe(no_mangle)]
pub extern "C" fn __stderr() -> *mut FILE {
    // SAFETY: Single-threaded access.
    unsafe { &raw mut STDERR_BUF }
}

// ---- Core I/O operations -----------------------------------------------------

/// Flush the write buffer of a FILE stream.
///
/// # Safety
///
/// `stream` must be a valid, non-null FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fflush(stream: *mut FILE) -> i32 {
    if stream.is_null() {
        // Flush all streams.
        unsafe {
            fflush(__stdout());
            fflush(__stderr());
        }
        return 0;
    }
    // SAFETY: Caller guarantees valid pointer.
    let f = unsafe { &mut *stream };
    if f.flags & FILE_WRITE != 0 && f.buf_pos > 0 {
        let buf = &f.buf[..f.buf_pos];
        let mut written = 0;
        while written < buf.len() {
            // SAFETY: We pass a valid subslice.
            match sys::sys_write(f.fd as usize, &buf[written..]) {
                Ok(n) if n > 0 => written += n,
                _ => {
                    f.flags |= FILE_ERROR;
                    return -1;
                }
            }
        }
        f.buf_pos = 0;
    }
    0
}

/// Write `nmemb` elements of `size` bytes from `ptr` to `stream`.
///
/// # Safety
///
/// `ptr` must be valid for `size * nmemb` bytes. `stream` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fwrite(
    ptr: *const u8,
    size: usize,
    nmemb: usize,
    stream: *mut FILE,
) -> usize {
    if stream.is_null() || ptr.is_null() || size == 0 || nmemb == 0 {
        return 0;
    }
    let total = size * nmemb;
    // SAFETY: Caller guarantees valid pointer for total bytes.
    let data = unsafe { core::slice::from_raw_parts(ptr, total) };
    // SAFETY: stream is valid.
    let f = unsafe { &mut *stream };

    if f.flags & FILE_WRITE == 0 {
        f.flags |= FILE_ERROR;
        return 0;
    }

    match f.buf_mode {
        BufMode::Unbuffered => {
            // Write directly.
            let mut written = 0;
            while written < total {
                match sys::sys_write(f.fd as usize, &data[written..]) {
                    Ok(n) if n > 0 => written += n,
                    _ => {
                        f.flags |= FILE_ERROR;
                        return written / size;
                    }
                }
            }
            nmemb
        }
        BufMode::LineBuffered => {
            // Buffer, but flush on newline or when full.
            for &byte in data {
                f.buf[f.buf_pos] = byte;
                f.buf_pos += 1;
                if byte == b'\n' || f.buf_pos >= BUF_SIZE {
                    if unsafe { fflush(stream) } != 0 {
                        return 0;
                    }
                }
            }
            nmemb
        }
        BufMode::FullyBuffered => {
            // Buffer, flush only when full.
            let mut i = 0;
            while i < total {
                let space = BUF_SIZE - f.buf_pos;
                let chunk = (total - i).min(space);
                f.buf[f.buf_pos..f.buf_pos + chunk].copy_from_slice(&data[i..i + chunk]);
                f.buf_pos += chunk;
                i += chunk;
                if f.buf_pos >= BUF_SIZE {
                    if unsafe { fflush(stream) } != 0 {
                        return i / size;
                    }
                }
            }
            nmemb
        }
    }
}

/// Read `nmemb` elements of `size` bytes into `ptr` from `stream`.
///
/// # Safety
///
/// `ptr` must be valid for `size * nmemb` bytes. `stream` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fread(
    ptr: *mut u8,
    size: usize,
    nmemb: usize,
    stream: *mut FILE,
) -> usize {
    if stream.is_null() || ptr.is_null() || size == 0 || nmemb == 0 {
        return 0;
    }
    let total = size * nmemb;
    // SAFETY: stream is valid.
    let f = unsafe { &mut *stream };

    if f.flags & FILE_READ == 0 {
        f.flags |= FILE_ERROR;
        return 0;
    }

    let mut read_total = 0;
    while read_total < total {
        // If buffer has data, consume it.
        if f.buf_pos < f.buf_len {
            let avail = f.buf_len - f.buf_pos;
            let chunk = (total - read_total).min(avail);
            // SAFETY: Caller guarantees ptr valid for total bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    f.buf.as_ptr().add(f.buf_pos),
                    ptr.add(read_total),
                    chunk,
                );
            }
            f.buf_pos += chunk;
            read_total += chunk;
        } else {
            // Refill buffer.
            match sys::sys_read(f.fd as usize, &mut f.buf) {
                Ok(0) => {
                    f.flags |= FILE_EOF;
                    break;
                }
                Ok(n) => {
                    f.buf_pos = 0;
                    f.buf_len = n;
                }
                Err(_) => {
                    f.flags |= FILE_ERROR;
                    break;
                }
            }
        }
    }
    read_total / size
}

/// Write a single character to `stream`.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputc(c: i32, stream: *mut FILE) -> i32 {
    let byte = c as u8;
    if unsafe { fwrite(&byte, 1, 1, stream) } == 1 {
        c
    } else {
        -1 // EOF
    }
}

/// Write a string to `stream` (without trailing newline).
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string. `stream` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fputs(s: *const u8, stream: *mut FILE) -> i32 {
    if s.is_null() {
        return -1;
    }
    // SAFETY: s is NUL-terminated.
    let len = unsafe { crate::string::strlen(s) };
    if unsafe { fwrite(s, 1, len, stream) } == len {
        0
    } else {
        -1
    }
}

/// Read a line into `s` (at most `n-1` chars), NUL-terminated.
///
/// # Safety
///
/// `s` must be valid for `n` bytes. `stream` must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fgets(s: *mut u8, n: i32, stream: *mut FILE) -> *mut u8 {
    if s.is_null() || stream.is_null() || n <= 0 {
        return core::ptr::null_mut();
    }
    let max = (n - 1) as usize;
    let mut i = 0;
    while i < max {
        let mut byte = 0u8;
        if unsafe { fread(&mut byte, 1, 1, stream) } != 1 {
            if i == 0 {
                return core::ptr::null_mut();
            }
            break;
        }
        // SAFETY: i < max < n, s valid for n bytes.
        unsafe { *s.add(i) = byte };
        i += 1;
        if byte == b'\n' {
            break;
        }
    }
    // SAFETY: i <= max < n.
    unsafe { *s.add(i) = 0 };
    s
}

/// Write string + newline to stdout.
///
/// # Safety
///
/// `s` must be a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn puts(s: *const u8) -> i32 {
    let stdout = __stdout();
    if unsafe { fputs(s, stdout) } == -1 {
        return -1;
    }
    if unsafe { fputc(b'\n' as i32, stdout) } == -1 {
        return -1;
    }
    0
}

/// Write a single character to stdout.
#[unsafe(no_mangle)]
pub extern "C" fn putchar(c: i32) -> i32 {
    unsafe { fputc(c, __stdout()) }
}

/// Open a file and return a FILE stream.
///
/// # Safety
///
/// `path` must be a valid NUL-terminated path string.
/// `mode` must be a valid NUL-terminated mode string ("r", "w", "a", etc.).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fopen(path: *const u8, mode: *const u8) -> *mut FILE {
    if path.is_null() || mode.is_null() {
        errno::set_errno(EINVAL);
        return core::ptr::null_mut();
    }

    // Parse mode string.
    // SAFETY: mode is NUL-terminated.
    let mode_byte = unsafe { *mode };
    let mut hadron_flags: usize = 0;
    let mut file_flags: u32 = FILE_OWNED_FD;

    match mode_byte {
        b'r' => {
            hadron_flags = crate::flags::posix_open_to_hadron(crate::flags::O_RDONLY);
            file_flags |= FILE_READ;
        }
        b'w' => {
            hadron_flags = crate::flags::posix_open_to_hadron(
                crate::flags::O_WRONLY | crate::flags::O_CREAT | crate::flags::O_TRUNC,
            );
            file_flags |= FILE_WRITE;
        }
        b'a' => {
            hadron_flags = crate::flags::posix_open_to_hadron(
                crate::flags::O_WRONLY | crate::flags::O_CREAT | crate::flags::O_APPEND,
            );
            file_flags |= FILE_WRITE;
        }
        _ => {
            errno::set_errno(EINVAL);
            return core::ptr::null_mut();
        }
    }

    // Check for '+' (read+write).
    // SAFETY: mode is NUL-terminated; if mode_byte != 0, mode+1 is valid.
    if unsafe { *mode.add(1) } == b'+' {
        hadron_flags = crate::flags::posix_open_to_hadron(crate::flags::O_RDWR);
        file_flags |= FILE_READ | FILE_WRITE;
    }

    // SAFETY: path is NUL-terminated.
    let path_len = unsafe { crate::string::strlen(path) };
    let path_slice = unsafe { core::slice::from_raw_parts(path, path_len) };

    let fd = match sys::sys_open(path_slice, hadron_flags) {
        Ok(fd) => fd,
        Err(e) => {
            errno::set_errno(e);
            return core::ptr::null_mut();
        }
    };

    // Allocate FILE struct.
    // SAFETY: malloc returns valid memory or null.
    let file_ptr = unsafe { crate::alloc::malloc(core::mem::size_of::<FILE>()) } as *mut FILE;
    if file_ptr.is_null() {
        let _ = sys::sys_close(fd);
        errno::set_errno(ENOMEM);
        return core::ptr::null_mut();
    }

    // SAFETY: file_ptr is valid allocated memory.
    unsafe {
        core::ptr::write(
            file_ptr,
            FILE::new(fd as i32, BufMode::FullyBuffered, file_flags),
        );
    }
    file_ptr
}

/// Close a FILE stream.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer from `fopen`, or a standard stream.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fclose(stream: *mut FILE) -> i32 {
    if stream.is_null() {
        return -1;
    }
    // Flush pending writes.
    unsafe { fflush(stream) };

    // SAFETY: stream is valid.
    let f = unsafe { &*stream };
    let ret = if f.flags & FILE_OWNED_FD != 0 {
        match sys::sys_close(f.fd as usize) {
            Ok(()) => 0,
            Err(e) => {
                errno::set_errno(e);
                -1
            }
        }
    } else {
        0
    };

    // Free if heap-allocated (owned fd means fopen'd).
    if f.flags & FILE_OWNED_FD != 0 {
        unsafe { crate::alloc::free(stream.cast()) };
    }

    ret
}

/// Check end-of-file indicator.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn feof(stream: *mut FILE) -> i32 {
    if stream.is_null() {
        return 0;
    }
    // SAFETY: stream is valid.
    i32::from(unsafe { (*stream).flags } & FILE_EOF != 0)
}

/// Check error indicator.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ferror(stream: *mut FILE) -> i32 {
    if stream.is_null() {
        return 0;
    }
    // SAFETY: stream is valid.
    i32::from(unsafe { (*stream).flags } & FILE_ERROR != 0)
}

/// Clear error and EOF indicators.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn clearerr(stream: *mut FILE) {
    if !stream.is_null() {
        // SAFETY: stream is valid.
        unsafe { (*stream).flags &= !(FILE_EOF | FILE_ERROR) };
    }
}

/// Get the file descriptor underlying a FILE stream.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fileno(stream: *mut FILE) -> i32 {
    if stream.is_null() {
        errno::set_errno(EBADF);
        return -1;
    }
    // SAFETY: stream is valid.
    unsafe { (*stream).fd }
}

/// Seek within a FILE stream.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fseek(stream: *mut FILE, offset: i64, whence: i32) -> i32 {
    if stream.is_null() {
        return -1;
    }
    // Flush write buffer before seeking.
    unsafe { fflush(stream) };
    // SAFETY: stream is valid.
    let f = unsafe { &mut *stream };
    // Discard read buffer.
    f.buf_pos = 0;
    f.buf_len = 0;
    f.flags &= !FILE_EOF;

    match sys::sys_lseek(f.fd as usize, offset, whence as usize) {
        Ok(_) => 0,
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Return current position in stream.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ftell(stream: *mut FILE) -> i64 {
    if stream.is_null() {
        return -1;
    }
    // SAFETY: stream is valid.
    let f = unsafe { &*stream };
    match sys::sys_lseek(f.fd as usize, 0, crate::flags::SEEK_CUR as usize) {
        Ok(pos) => {
            // Adjust for buffered but unconsumed read data.
            let buffered = if f.flags & FILE_READ != 0 && f.buf_len > f.buf_pos {
                (f.buf_len - f.buf_pos) as i64
            } else {
                0
            };
            pos as i64 - buffered
        }
        Err(e) => {
            errno::set_errno(e);
            -1
        }
    }
}

/// Rewind stream to beginning.
///
/// # Safety
///
/// `stream` must be a valid FILE pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rewind(stream: *mut FILE) {
    unsafe { fseek(stream, 0, crate::flags::SEEK_SET) };
    if !stream.is_null() {
        unsafe { clearerr(stream) };
    }
}
