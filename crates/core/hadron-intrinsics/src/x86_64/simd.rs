//! Safe SIMD vector types.
//!
//! [`Simd128`] is an opaque 128-bit (16-byte) value that lives in memory
//! (not in XMM registers) due to the soft-float ABI. It is only meaningful
//! within a `KernelFpuGuard` scope and should be treated as transient.

/// A 128-bit SIMD value, 16-byte aligned.
///
/// This is a memory-resident container — the `x86-softfloat` ABI prevents
/// it from being passed in XMM registers across function boundaries. Use
/// the intrinsics in [`super::sse2`] to load from / store to this type.
#[repr(C, align(16))]
#[derive(Clone, Copy)]
pub struct Simd128([u8; 16]);

impl Simd128 {
    /// A zero-initialized SIMD value.
    pub const ZERO: Self = Self([0u8; 16]);

    /// Returns a pointer to the underlying data (for asm load).
    #[inline(always)]
    pub fn as_ptr(&self) -> *const u8 {
        self.0.as_ptr()
    }

    /// Returns a mutable pointer to the underlying data (for asm store).
    #[inline(always)]
    pub fn as_mut_ptr(&mut self) -> *mut u8 {
        self.0.as_mut_ptr()
    }
}
