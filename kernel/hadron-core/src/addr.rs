//! Typed virtual and physical address wrappers.
//!
//! Provides [`VirtAddr`] and [`PhysAddr`] newtypes that prevent mixing virtual
//! and physical addresses at the type level.

use core::fmt;
use core::ops::{Add, Sub};

/// A canonical 64-bit virtual address.
///
/// On x86_64 with 4-level paging, bits 48..63 must be a sign-extension of
/// bit 47. On aarch64 with 48-bit VA, addresses must fall in the TTBR0
/// (`0x0000_xxxx_xxxx_xxxx`) or TTBR1 (`0xFFFF_xxxx_xxxx_xxxx`) range.
/// This type enforces that invariant via sign-extension from bit 47.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct VirtAddr(u64);

/// A 64-bit physical address (masked to 52 bits).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct PhysAddr(u64);

/// Physical address space mask: bits 0..51.
const PHYS_ADDR_MASK: u64 = 0x000F_FFFF_FFFF_FFFF;

/// Mask for the 12-bit page offset (bits 0..11).
const PAGE_OFFSET_MASK: u64 = 0xFFF;

/// Mask for a 9-bit page table index (used by all paging levels).
const PAGE_TABLE_INDEX_MASK: usize = 0x1FF;

impl VirtAddr {
    /// Creates a new `VirtAddr`, sign-extending from bit 47 to enforce
    /// canonical form. Panics in debug mode if the address is not canonical.
    #[inline]
    pub const fn new(addr: u64) -> Self {
        let canonical = Self::new_truncate(addr);
        // In debug mode, verify the address was already canonical.
        assert!(
            canonical.0 == addr,
            "VirtAddr::new: address is not canonical"
        );
        canonical
    }

    /// Creates a new `VirtAddr`, truncating to canonical form by
    /// sign-extending from bit 47.
    #[inline]
    pub const fn new_truncate(addr: u64) -> Self {
        // Sign-extend from bit 47.
        Self(((addr << 16) as i64 >> 16) as u64)
    }

    /// Creates a new `VirtAddr` without any validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure `addr` is in canonical form.
    #[inline]
    pub const unsafe fn new_unchecked(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the zero address.
    #[inline]
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Returns the raw `u64` value.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Converts this address to a raw pointer.
    #[inline]
    pub const fn as_ptr<T>(self) -> *const T {
        self.0 as *const T
    }

    /// Converts this address to a raw mutable pointer.
    #[inline]
    pub const fn as_mut_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    /// Returns `true` if the address is aligned to `align`.
    ///
    /// `align` must be a power of two.
    #[inline]
    pub const fn is_aligned(self, align: u64) -> bool {
        debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
        self.0 & (align - 1) == 0
    }

    /// Aligns the address down to `align`.
    ///
    /// `align` must be a power of two.
    #[inline]
    pub const fn align_down(self, align: u64) -> Self {
        debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
        Self::new_truncate(self.0 & !(align - 1))
    }

    /// Aligns the address up to `align`.
    ///
    /// `align` must be a power of two.
    #[inline]
    pub const fn align_up(self, align: u64) -> Self {
        debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
        Self::new_truncate((self.0 + align - 1) & !(align - 1))
    }

    /// Returns the page offset (bits 0..11).
    #[inline]
    pub const fn page_offset(self) -> u64 {
        self.0 & PAGE_OFFSET_MASK
    }
}

#[cfg(target_arch = "x86_64")]
impl VirtAddr {
    /// Returns the PML4 table index (bits 39..47).
    #[inline]
    pub const fn pml4_index(self) -> usize {
        ((self.0 >> 39) as usize) & PAGE_TABLE_INDEX_MASK
    }

    /// Returns the Page Directory Pointer Table index (bits 30..38).
    #[inline]
    pub const fn pdpt_index(self) -> usize {
        ((self.0 >> 30) as usize) & PAGE_TABLE_INDEX_MASK
    }

    /// Returns the Page Directory index (bits 21..29).
    #[inline]
    pub const fn pd_index(self) -> usize {
        ((self.0 >> 21) as usize) & PAGE_TABLE_INDEX_MASK
    }

    /// Returns the Page Table index (bits 12..20).
    #[inline]
    pub const fn pt_index(self) -> usize {
        ((self.0 >> 12) as usize) & PAGE_TABLE_INDEX_MASK
    }
}

#[cfg(target_arch = "aarch64")]
impl VirtAddr {
    /// Returns the L1 table index (bits 30..38) for 4 KiB granule.
    #[inline]
    pub const fn l1_index(self) -> usize {
        ((self.0 >> 30) as usize) & PAGE_TABLE_INDEX_MASK
    }

    /// Returns the L2 table index (bits 21..29) for 4 KiB granule.
    #[inline]
    pub const fn l2_index(self) -> usize {
        ((self.0 >> 21) as usize) & PAGE_TABLE_INDEX_MASK
    }

    /// Returns the L3 table index (bits 12..20) for 4 KiB granule.
    #[inline]
    pub const fn l3_index(self) -> usize {
        ((self.0 >> 12) as usize) & PAGE_TABLE_INDEX_MASK
    }
}

impl Add<u64> for VirtAddr {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u64) -> Self {
        Self::new_truncate(self.0.wrapping_add(rhs))
    }
}

impl Sub<u64> for VirtAddr {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u64) -> Self {
        Self::new_truncate(self.0.wrapping_sub(rhs))
    }
}

impl Sub<VirtAddr> for VirtAddr {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: VirtAddr) -> u64 {
        self.0.wrapping_sub(rhs.0)
    }
}

impl fmt::Debug for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "VirtAddr({:#x})", self.0)
    }
}

impl fmt::Display for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl fmt::LowerHex for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

impl fmt::UpperHex for VirtAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, f)
    }
}

// ---------------------------------------------------------------------------
// PhysAddr
// ---------------------------------------------------------------------------

impl PhysAddr {
    /// Creates a new `PhysAddr`, masking to the 52-bit physical address space.
    /// Panics in debug mode if bits above 52 are set.
    #[inline]
    pub const fn new(addr: u64) -> Self {
        let masked = addr & PHYS_ADDR_MASK;
        debug_assert!(
            masked == addr,
            "PhysAddr::new: address exceeds 52-bit physical address space"
        );
        Self(masked)
    }

    /// Creates a new `PhysAddr`, truncating to the 52-bit physical address
    /// space.
    #[inline]
    pub const fn new_truncate(addr: u64) -> Self {
        Self(addr & PHYS_ADDR_MASK)
    }

    /// Creates a new `PhysAddr` without any validation.
    ///
    /// # Safety
    ///
    /// The caller must ensure `addr` fits within the 52-bit physical address
    /// space.
    #[inline]
    pub const unsafe fn new_unchecked(addr: u64) -> Self {
        Self(addr)
    }

    /// Returns the zero address.
    #[inline]
    pub const fn zero() -> Self {
        Self(0)
    }

    /// Returns the raw `u64` value.
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    /// Returns `true` if the address is aligned to `align`.
    ///
    /// `align` must be a power of two.
    #[inline]
    pub const fn is_aligned(self, align: u64) -> bool {
        debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
        self.0 & (align - 1) == 0
    }

    /// Aligns the address down to `align`.
    ///
    /// `align` must be a power of two.
    #[inline]
    pub const fn align_down(self, align: u64) -> Self {
        debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
        Self(self.0 & !(align - 1))
    }

    /// Aligns the address up to `align`.
    ///
    /// `align` must be a power of two.
    #[inline]
    pub const fn align_up(self, align: u64) -> Self {
        debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
        Self((self.0 + align - 1) & !(align - 1))
    }
}

impl Add<u64> for PhysAddr {
    type Output = Self;
    #[inline]
    fn add(self, rhs: u64) -> Self {
        Self::new(self.0 + rhs)
    }
}

impl Sub<u64> for PhysAddr {
    type Output = Self;
    #[inline]
    fn sub(self, rhs: u64) -> Self {
        Self::new(self.0 - rhs)
    }
}

impl Sub<PhysAddr> for PhysAddr {
    type Output = u64;
    #[inline]
    fn sub(self, rhs: PhysAddr) -> u64 {
        self.0 - rhs.0
    }
}

impl fmt::Debug for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PhysAddr({:#x})", self.0)
    }
}

impl fmt::Display for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:#x}", self.0)
    }
}

impl fmt::LowerHex for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::LowerHex::fmt(&self.0, f)
    }
}

impl fmt::UpperHex for PhysAddr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::UpperHex::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virt_addr_canonical_low() {
        let addr = VirtAddr::new(0x0000_1234_5678_9ABC);
        assert_eq!(addr.as_u64(), 0x0000_1234_5678_9ABC);
    }

    #[test]
    fn virt_addr_truncate_high_half() {
        // Bit 47 set → sign-extends to set bits 48..63.
        let addr = VirtAddr::new_truncate(0x0000_8000_0000_0000);
        assert_eq!(addr.as_u64(), 0xFFFF_8000_0000_0000);
    }

    #[test]
    fn virt_addr_zero() {
        assert_eq!(VirtAddr::zero().as_u64(), 0);
    }

    #[test]
    fn virt_addr_align_down() {
        let addr = VirtAddr::new(0x1234);
        assert_eq!(addr.align_down(4096).as_u64(), 0x1000);
    }

    #[test]
    fn virt_addr_align_up() {
        let addr = VirtAddr::new(0x1001);
        assert_eq!(addr.align_up(4096).as_u64(), 0x2000);
    }

    #[test]
    fn virt_addr_already_aligned() {
        let addr = VirtAddr::new(0x2000);
        assert!(addr.is_aligned(4096));
        assert_eq!(addr.align_up(4096).as_u64(), 0x2000);
        assert_eq!(addr.align_down(4096).as_u64(), 0x2000);
    }

    #[test]
    #[cfg(target_arch = "x86_64")]
    fn virt_addr_page_indices() {
        // Address 0xFFFF_8000_0020_1000 — canonical high-half address.
        let addr = VirtAddr::new(0xFFFF_8000_0020_1000);
        assert_eq!(addr.pml4_index(), 256); // bit 39..47 of this address
        assert_eq!(addr.page_offset(), 0);
    }

    #[test]
    fn virt_addr_add_sub() {
        let addr = VirtAddr::new(0x1000);
        assert_eq!((addr + 0x500).as_u64(), 0x1500);
        assert_eq!((addr - 0x500).as_u64(), 0x0B00);
    }

    #[test]
    fn virt_addr_sub_virt_addr() {
        let a = VirtAddr::new(0x2000);
        let b = VirtAddr::new(0x1000);
        assert_eq!(a - b, 0x1000);
    }

    #[test]
    fn phys_addr_new_valid() {
        let addr = PhysAddr::new(0x0000_1234_5678_9ABC);
        assert_eq!(addr.as_u64(), 0x0000_1234_5678_9ABC);
    }

    #[test]
    fn phys_addr_truncate() {
        // Bits above 52 are masked off.
        let addr = PhysAddr::new_truncate(0xFFFF_FFFF_FFFF_FFFF);
        assert_eq!(addr.as_u64(), PHYS_ADDR_MASK);
    }

    #[test]
    fn phys_addr_zero() {
        assert_eq!(PhysAddr::zero().as_u64(), 0);
    }

    #[test]
    fn phys_addr_alignment() {
        let addr = PhysAddr::new(0x3456);
        assert!(!addr.is_aligned(4096));
        assert_eq!(addr.align_down(4096).as_u64(), 0x3000);
        assert_eq!(addr.align_up(4096).as_u64(), 0x4000);
    }

    #[test]
    fn phys_addr_add_sub() {
        let addr = PhysAddr::new(0x2000);
        assert_eq!((addr + 0x100).as_u64(), 0x2100);
        assert_eq!((addr - 0x100).as_u64(), 0x1F00);
    }
}
