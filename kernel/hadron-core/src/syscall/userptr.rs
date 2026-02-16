//! User-space pointer validation for syscall arguments.
//!
//! Provides [`UserPtr`] and [`UserSlice`] types that validate pointers passed
//! from user space before dereferencing, preventing the kernel from blindly
//! trusting user-supplied addresses.

use core::marker::PhantomData;

use super::EFAULT;

/// Upper bound of canonical user-space addresses on x86_64.
///
/// Addresses with bit 63 set are kernel addresses (upper-half). User
/// pointers must be below this boundary.
const USER_ADDR_MAX: usize = 0x0000_8000_0000_0000;

/// A validated pointer to user-space memory of type `T`.
///
/// Constructing a `UserPtr` checks that the address:
/// - Is below the user/kernel boundary (`0x0000_8000_0000_0000`)
/// - Is properly aligned for `T`
/// - Does not overflow when combined with `size_of::<T>()`
///
/// This type does **not** guarantee that the memory is mapped or readable;
/// page faults can still occur. It only ensures the address is in the user
/// half of the address space.
#[derive(Debug, Clone, Copy)]
pub struct UserPtr<T> {
    addr: usize,
    _marker: PhantomData<*const T>,
}

impl<T> UserPtr<T> {
    /// Validate a raw user-space address.
    ///
    /// Returns `Err(-EFAULT)` if the address is in kernel space, misaligned,
    /// or would overflow when adding `size_of::<T>()`.
    pub fn new(addr: usize) -> Result<Self, isize> {
        let size = core::mem::size_of::<T>();

        // Check alignment
        let align = core::mem::align_of::<T>();
        if align > 1 && addr % align != 0 {
            return Err(-EFAULT);
        }

        // Check overflow
        let end = addr.checked_add(size).ok_or(-EFAULT)?;

        // Check that the entire range is in user space
        if end > USER_ADDR_MAX {
            return Err(-EFAULT);
        }

        Ok(Self {
            addr,
            _marker: PhantomData,
        })
    }

    /// Returns the raw address.
    pub fn addr(&self) -> usize {
        self.addr
    }

    /// Dereference the validated pointer.
    ///
    /// # Safety
    ///
    /// The caller must ensure:
    /// - The memory at this address is mapped and readable
    /// - The memory contains a valid `T`
    /// - No mutable aliases exist
    pub unsafe fn as_ref(&self) -> &T {
        unsafe { &*(self.addr as *const T) }
    }
}

/// A validated user-space byte slice (pointer + length).
///
/// Validates that the entire range `[addr, addr + len)` lies within user
/// address space and does not overflow.
#[derive(Debug, Clone, Copy)]
pub struct UserSlice {
    addr: usize,
    len: usize,
}

impl UserSlice {
    /// Validate a user-space buffer described by a raw address and length.
    ///
    /// Returns `Err(-EFAULT)` if any byte of the range falls outside user
    /// address space.
    pub fn new(addr: usize, len: usize) -> Result<Self, isize> {
        if len == 0 {
            return Ok(Self { addr: 0, len: 0 });
        }

        let end = addr.checked_add(len).ok_or(-EFAULT)?;
        if end > USER_ADDR_MAX {
            return Err(-EFAULT);
        }

        Ok(Self { addr, len })
    }

    /// Convert the validated range to a byte slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory is mapped, readable, and not
    /// mutably aliased.
    pub unsafe fn as_slice(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        unsafe { core::slice::from_raw_parts(self.addr as *const u8, self.len) }
    }

    /// Convert the validated range to a mutable byte slice.
    ///
    /// # Safety
    ///
    /// The caller must ensure the memory is mapped, writable, and not
    /// aliased.
    pub unsafe fn as_mut_slice(&self) -> &mut [u8] {
        if self.len == 0 {
            return &mut [];
        }
        unsafe { core::slice::from_raw_parts_mut(self.addr as *mut u8, self.len) }
    }

    /// Returns the raw address.
    pub fn addr(&self) -> usize {
        self.addr
    }

    /// Returns the length in bytes.
    pub fn len(&self) -> usize {
        self.len
    }

    /// Returns whether the slice is empty.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn userptr_valid_low_address() {
        let ptr = UserPtr::<u32>::new(0x1000);
        assert!(ptr.is_ok());
        assert_eq!(ptr.unwrap().addr(), 0x1000);
    }

    #[test]
    fn userptr_rejects_kernel_address() {
        let ptr = UserPtr::<u8>::new(USER_ADDR_MAX);
        assert!(ptr.is_err());
    }

    #[test]
    fn userptr_rejects_overflow() {
        let ptr = UserPtr::<u8>::new(usize::MAX);
        assert!(ptr.is_err());
    }

    #[test]
    fn userptr_rejects_misaligned() {
        let ptr = UserPtr::<u64>::new(0x1001);
        assert!(ptr.is_err());
    }

    #[test]
    fn userslice_valid() {
        let slice = UserSlice::new(0x1000, 4096);
        assert!(slice.is_ok());
        let slice = slice.unwrap();
        assert_eq!(slice.addr(), 0x1000);
        assert_eq!(slice.len(), 4096);
    }

    #[test]
    fn userslice_empty() {
        let slice = UserSlice::new(0, 0).unwrap();
        assert!(slice.is_empty());
        assert_eq!(slice.len(), 0);
    }

    #[test]
    fn userslice_rejects_kernel_range() {
        let slice = UserSlice::new(USER_ADDR_MAX - 10, 20);
        assert!(slice.is_err());
    }

    #[test]
    fn userslice_rejects_overflow() {
        let slice = UserSlice::new(usize::MAX, 1);
        assert!(slice.is_err());
    }

    #[test]
    fn is_kernel_caller_low() {
        assert!(!is_kernel_caller(0x1000));
        assert!(!is_kernel_caller(USER_ADDR_MAX - 1));
    }

    #[test]
    fn is_kernel_caller_high() {
        assert!(is_kernel_caller(USER_ADDR_MAX));
        assert!(is_kernel_caller(usize::MAX));
    }
}

/// Check whether we are currently handling a kernel-mode syscall test.
///
/// During early boot testing (before userspace exists), syscalls are invoked
/// from kernel space where addresses have bit 63 set. In that context,
/// `UserPtr` / `UserSlice` validation would reject every pointer. Callers
/// can use this function to skip validation when the saved return RIP
/// indicates a kernel-mode caller.
///
/// # Safety
///
/// This must only be used in the syscall dispatch path and never to bypass
/// security checks for actual user-space callers.
pub fn is_kernel_caller(saved_rip: usize) -> bool {
    saved_rip >= USER_ADDR_MAX
}
