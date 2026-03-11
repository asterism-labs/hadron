//! User pointer validation for syscall arguments.
//!
//! All pointers passed from userspace must be validated before the kernel
//! dereferences them. The user-accessible virtual address range is
//! `0x1000..0x0000_8000_0000_0000` (lower-half canonical addresses, excluding
//! the null guard page).

use hadron_syscall::EFAULT;

/// Minimum valid user address (first page is unmapped as a null guard).
const USER_ADDR_MIN: usize = 0x1000;

/// Upper bound of the user-accessible virtual address range (non-inclusive).
const USER_ADDR_LIMIT: usize = 0x0000_8000_0000_0000;

/// A validated pointer to a `T` in user memory.
///
/// Created via [`UserPtr::new`], which checks that the entire `T` resides
/// within the valid user address range.
#[derive(Clone, Copy)]
pub struct UserPtr<T> {
    ptr: *const T,
}

impl<T> UserPtr<T> {
    /// Validate that `addr` points to a complete `T` within user memory.
    ///
    /// Returns `-EFAULT` if the address range is invalid.
    pub fn new(addr: usize) -> Result<Self, isize> {
        let size = core::mem::size_of::<T>();
        validate_range(addr, size)?;
        Ok(Self {
            ptr: addr as *const T,
        })
    }

    /// Read the value from user memory.
    ///
    /// # Safety
    ///
    /// The pointed-to memory must be mapped and readable. The address has
    /// been range-checked, but page-level access depends on the process
    /// having mapped the page.
    pub unsafe fn read(&self) -> T {
        // SAFETY: Caller guarantees pages are mapped.
        unsafe { self.ptr.read_unaligned() }
    }

    /// Return the raw pointer.
    pub fn as_ptr(&self) -> *const T {
        self.ptr
    }
}

/// A validated mutable pointer to a `T` in user memory.
#[derive(Clone, Copy)]
pub struct UserPtrMut<T> {
    ptr: *mut T,
}

impl<T> UserPtrMut<T> {
    /// Validate that `addr` points to a writable `T` within user memory.
    pub fn new(addr: usize) -> Result<Self, isize> {
        let size = core::mem::size_of::<T>();
        validate_range(addr, size)?;
        Ok(Self {
            ptr: addr as *mut T,
        })
    }

    /// Write a value to user memory.
    ///
    /// # Safety
    ///
    /// The pointed-to memory must be mapped and writable.
    pub unsafe fn write(&self, val: T) {
        // SAFETY: Caller guarantees pages are mapped and writable.
        unsafe { self.ptr.write_unaligned(val) };
    }

    /// Return the raw mutable pointer.
    pub fn as_mut_ptr(&self) -> *mut T {
        self.ptr
    }
}

/// A validated slice of bytes in user memory.
pub struct UserSlice {
    ptr: *const u8,
    len: usize,
}

impl UserSlice {
    /// Validate that `addr..addr+len` resides within user memory.
    pub fn new(addr: usize, len: usize) -> Result<Self, isize> {
        validate_range(addr, len)?;
        Ok(Self {
            ptr: addr as *const u8,
            len,
        })
    }

    /// Read the slice contents into a `Vec<u8>`.
    ///
    /// # Safety
    ///
    /// The pointed-to memory must be mapped and readable.
    pub unsafe fn read_to_vec(&self) -> alloc::vec::Vec<u8> {
        let mut buf = alloc::vec![0u8; self.len];
        // SAFETY: Caller guarantees pages are mapped.
        unsafe {
            core::ptr::copy_nonoverlapping(self.ptr, buf.as_mut_ptr(), self.len);
        }
        buf
    }

    /// Copy kernel data into the user buffer.
    ///
    /// # Safety
    ///
    /// The pointed-to memory must be mapped and writable.
    pub unsafe fn write_from_slice(&self, src: &[u8]) {
        let copy_len = src.len().min(self.len);
        // SAFETY: Caller guarantees pages are mapped and writable.
        unsafe {
            core::ptr::copy_nonoverlapping(src.as_ptr(), self.ptr as *mut u8, copy_len);
        }
    }

    /// Length of the user buffer.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// Validate that `addr..addr+len` is within the user address range.
fn validate_range(addr: usize, len: usize) -> Result<(), isize> {
    if addr < USER_ADDR_MIN {
        return Err(-EFAULT);
    }
    let end = addr.checked_add(len).ok_or(-EFAULT)?;
    if end > USER_ADDR_LIMIT {
        return Err(-EFAULT);
    }
    Ok(())
}
