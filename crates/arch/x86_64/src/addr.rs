//! Physical and virtual addresses.
//!
//! Physical addresses are at most 52 bits wide, the architectural limit
//! (Vol. 3A §5.1.4). Virtual addresses must be canonical for 4-level paging:
//! bits 63:47 all equal (Vol. 1 §3.3.7.1). The kernel asks Limine for
//! 4-level paging, so 5-level addresses are not supported.

use core::fmt;
use core::ops::{Add, Sub};

/// Number of bits in a physical address.
pub const PHYS_ADDR_BITS: u32 = 52;

/// Number of bits a 4-level virtual address translates.
pub const VIRT_ADDR_BITS: u32 = 48;

/// A physical address below 2^52.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct PhysAddr(u64);

impl PhysAddr {
    /// Physical address 0.
    pub const ZERO: Self = Self(0);

    /// Wraps `addr`, or returns `None` if it has bits at or above 52.
    #[must_use]
    pub const fn try_new(addr: u64) -> Option<Self> {
        if addr >> PHYS_ADDR_BITS == 0 {
            Some(Self(addr))
        } else {
            None
        }
    }

    /// Wraps `addr`.
    ///
    /// # Panics
    ///
    /// Panics if `addr` has bits at or above 52.
    #[must_use]
    pub const fn new(addr: u64) -> Self {
        match Self::try_new(addr) {
            Some(addr) => addr,
            None => panic!("physical address wider than 52 bits"),
        }
    }

    /// The address as a number.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Whether the address is a multiple of `align`, a power of two.
    #[must_use]
    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }

    /// The address rounded down to a multiple of `align`, a power of two.
    #[must_use]
    pub const fn align_down(self, align: u64) -> Self {
        Self(self.0 & !(align - 1))
    }

    /// The address rounded up to a multiple of `align`, a power of two.
    ///
    /// # Panics
    ///
    /// Panics if the result does not fit in 52 bits.
    #[must_use]
    pub const fn align_up(self, align: u64) -> Self {
        Self::new((self.0 + align - 1) & !(align - 1))
    }
}

impl Add<u64> for PhysAddr {
    type Output = Self;

    fn add(self, rhs: u64) -> Self {
        Self::new(self.0 + rhs)
    }
}

impl Sub<PhysAddr> for PhysAddr {
    type Output = u64;

    fn sub(self, rhs: PhysAddr) -> u64 {
        self.0 - rhs.0
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysAddr({:#x})", self.0)
    }
}

impl fmt::LowerHex for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

/// A canonical virtual address for 4-level paging.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct VirtAddr(u64);

impl VirtAddr {
    /// Virtual address 0.
    pub const ZERO: Self = Self(0);

    /// Wraps `addr`, or returns `None` if it is not canonical.
    #[must_use]
    pub const fn try_new(addr: u64) -> Option<Self> {
        let canonical = Self::sign_extend(addr);
        if canonical == addr {
            Some(Self(addr))
        } else {
            None
        }
    }

    /// Wraps `addr`.
    ///
    /// # Panics
    ///
    /// Panics if `addr` is not canonical.
    #[must_use]
    pub const fn new(addr: u64) -> Self {
        match Self::try_new(addr) {
            Some(addr) => addr,
            None => panic!("virtual address is not canonical"),
        }
    }

    /// Copies bit 47 into bits 63:48.
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_wrap)] // bit casts
    const fn sign_extend(addr: u64) -> u64 {
        let shift = 64 - VIRT_ADDR_BITS;
        (((addr << shift) as i64) >> shift) as u64
    }

    /// The address of `ptr`.
    ///
    /// # Panics
    ///
    /// Panics if the pointer is not canonical, which cannot happen on
    /// `x86_64` with 4-level paging.
    #[must_use]
    pub fn from_ptr<T: ?Sized>(ptr: *const T) -> Self {
        Self::new(ptr.cast::<u8>() as u64)
    }

    /// The address as a number.
    #[must_use]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// The address as a pointer.
    #[must_use]
    pub const fn as_ptr<T>(self) -> *const T {
        self.0 as *const T
    }

    /// The address as a mutable pointer.
    #[must_use]
    pub const fn as_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    /// Whether the address is a multiple of `align`, a power of two.
    #[must_use]
    pub const fn is_aligned(self, align: u64) -> bool {
        self.0 & (align - 1) == 0
    }

    /// The address rounded down to a multiple of `align`, a power of two.
    #[must_use]
    pub const fn align_down(self, align: u64) -> Self {
        Self(self.0 & !(align - 1))
    }

    /// The 9-bit table index this address selects at `level`.
    #[must_use]
    #[allow(clippy::cast_possible_truncation)] // masked to 9 bits
    pub const fn table_index(self, level: crate::structures::paging::Level) -> usize {
        ((self.0 >> level.shift()) & 0x1ff) as usize
    }

    /// `self + offset`, or `None` if the result is not canonical or
    /// overflows.
    #[must_use]
    pub const fn checked_add(self, offset: u64) -> Option<Self> {
        match self.0.checked_add(offset) {
            Some(addr) => Self::try_new(addr),
            None => None,
        }
    }
}

impl Add<u64> for VirtAddr {
    type Output = Self;

    fn add(self, rhs: u64) -> Self {
        self.checked_add(rhs)
            .expect("virtual address addition left the canonical range")
    }
}

impl Sub<VirtAddr> for VirtAddr {
    type Output = u64;

    fn sub(self, rhs: VirtAddr) -> u64 {
        self.0 - rhs.0
    }
}

impl fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}

impl fmt::LowerHex for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structures::paging::Level;

    #[test]
    fn phys_addr_rejects_bits_above_51() {
        assert!(PhysAddr::try_new((1 << 52) - 1).is_some());
        assert!(PhysAddr::try_new(1 << 52).is_none());
    }

    #[test]
    fn virt_addr_requires_canonical_form() {
        assert!(VirtAddr::try_new(0x0000_7fff_ffff_ffff).is_some());
        assert!(VirtAddr::try_new(0x0000_8000_0000_0000).is_none());
        assert!(VirtAddr::try_new(0xffff_7fff_ffff_ffff).is_none());
        assert!(VirtAddr::try_new(0xffff_8000_0000_0000).is_some());
        assert!(VirtAddr::try_new(0xffff_ffff_8000_0000).is_some());
    }

    #[test]
    fn checked_add_stops_at_the_canonical_hole() {
        let top_of_low_half = VirtAddr::new(0x0000_7fff_ffff_f000);
        assert_eq!(top_of_low_half.checked_add(0x1000), None);
        assert_eq!(
            VirtAddr::new(0xffff_ffff_ffff_f000).checked_add(0x1000),
            None
        );
    }

    #[test]
    fn table_indices_of_the_kernel_base() {
        let addr = VirtAddr::new(0xffff_ffff_8020_1000);
        assert_eq!(addr.table_index(Level::Pml4), 511);
        assert_eq!(addr.table_index(Level::Pdpt), 510);
        assert_eq!(addr.table_index(Level::Pd), 1);
        assert_eq!(addr.table_index(Level::Pt), 1);
    }

    #[test]
    fn alignment_helpers() {
        let addr = PhysAddr::new(0x1234);
        assert_eq!(addr.align_down(0x1000), PhysAddr::new(0x1000));
        assert_eq!(addr.align_up(0x1000), PhysAddr::new(0x2000));
        assert!(!addr.is_aligned(0x1000));
        assert!(PhysAddr::new(0x20_0000).is_aligned(0x20_0000));
    }
}
