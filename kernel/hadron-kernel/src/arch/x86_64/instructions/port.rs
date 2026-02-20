//! Typed port I/O for x86_64.
//!
//! Provides [`Port<T>`] as a type-safe wrapper around raw port I/O operations,
//! replacing ad-hoc `inb`/`outb` calls with a structured API.

use core::marker::PhantomData;

/// Trait for types that can be read from an I/O port.
///
/// # Safety
///
/// Implementations must use the correct `in` instruction variant for the type
/// size.
pub unsafe trait PortRead {
    /// Reads a value from the given I/O port.
    ///
    /// # Safety
    ///
    /// The caller must ensure `port` is a valid I/O port that is safe to read.
    unsafe fn read_from_port(port: u16) -> Self;
}

/// Trait for types that can be written to an I/O port.
///
/// # Safety
///
/// Implementations must use the correct `out` instruction variant for the type
/// size.
pub unsafe trait PortWrite {
    /// Writes a value to the given I/O port.
    ///
    /// # Safety
    ///
    /// The caller must ensure `port` is a valid I/O port that is safe to write.
    unsafe fn write_to_port(port: u16, value: Self);
}

// SAFETY: Uses `in al, dx` which reads a single byte.
unsafe impl PortRead for u8 {
    #[inline]
    unsafe fn read_from_port(port: u16) -> Self {
        let val: u8;
        unsafe {
            core::arch::asm!(
                "in al, dx",
                in("dx") port,
                out("al") val,
                options(nomem, nostack, preserves_flags),
            );
        }
        val
    }
}

// SAFETY: Uses `out dx, al` which writes a single byte.
unsafe impl PortWrite for u8 {
    #[inline]
    unsafe fn write_to_port(port: u16, value: Self) {
        unsafe {
            core::arch::asm!(
                "out dx, al",
                in("dx") port,
                in("al") value,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

// SAFETY: Uses `in ax, dx` which reads a 16-bit word.
unsafe impl PortRead for u16 {
    #[inline]
    unsafe fn read_from_port(port: u16) -> Self {
        let val: u16;
        unsafe {
            core::arch::asm!(
                "in ax, dx",
                in("dx") port,
                out("ax") val,
                options(nomem, nostack, preserves_flags),
            );
        }
        val
    }
}

// SAFETY: Uses `out dx, ax` which writes a 16-bit word.
unsafe impl PortWrite for u16 {
    #[inline]
    unsafe fn write_to_port(port: u16, value: Self) {
        unsafe {
            core::arch::asm!(
                "out dx, ax",
                in("dx") port,
                in("ax") value,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

// SAFETY: Uses `in eax, dx` which reads a 32-bit dword.
unsafe impl PortRead for u32 {
    #[inline]
    unsafe fn read_from_port(port: u16) -> Self {
        let val: u32;
        unsafe {
            core::arch::asm!(
                "in eax, dx",
                in("dx") port,
                out("eax") val,
                options(nomem, nostack, preserves_flags),
            );
        }
        val
    }
}

// SAFETY: Uses `out dx, eax` which writes a 32-bit dword.
unsafe impl PortWrite for u32 {
    #[inline]
    unsafe fn write_to_port(port: u16, value: Self) {
        unsafe {
            core::arch::asm!(
                "out dx, eax",
                in("dx") port,
                in("eax") value,
                options(nomem, nostack, preserves_flags),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Port<T>
// ---------------------------------------------------------------------------

/// A read-write I/O port at a fixed address, generic over the value type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Port<T: PortRead + PortWrite> {
    port: u16,
    _phantom: PhantomData<T>,
}

impl<T: PortRead + PortWrite> Port<T> {
    /// Creates a new port handle. Does **not** perform any I/O.
    #[inline]
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            _phantom: PhantomData,
        }
    }

    /// Returns the port number.
    #[inline]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Reads a value from this port.
    ///
    /// # Safety
    ///
    /// The caller must ensure this port is valid and safe to read.
    #[inline]
    pub unsafe fn read(&self) -> T {
        unsafe { T::read_from_port(self.port) }
    }

    /// Writes a value to this port.
    ///
    /// # Safety
    ///
    /// The caller must ensure this port is valid and safe to write.
    #[inline]
    pub unsafe fn write(&self, value: T) {
        unsafe { T::write_to_port(self.port, value) }
    }
}

// ---------------------------------------------------------------------------
// ReadOnlyPort<T>
// ---------------------------------------------------------------------------

/// A read-only I/O port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadOnlyPort<T: PortRead> {
    port: u16,
    _phantom: PhantomData<T>,
}

impl<T: PortRead> ReadOnlyPort<T> {
    /// Creates a new read-only port handle.
    #[inline]
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            _phantom: PhantomData,
        }
    }

    /// Returns the port number.
    #[inline]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Reads a value from this port.
    ///
    /// # Safety
    ///
    /// The caller must ensure this port is valid and safe to read.
    #[inline]
    pub unsafe fn read(&self) -> T {
        unsafe { T::read_from_port(self.port) }
    }
}

// ---------------------------------------------------------------------------
// WriteOnlyPort<T>
// ---------------------------------------------------------------------------

/// A write-only I/O port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WriteOnlyPort<T: PortWrite> {
    port: u16,
    _phantom: PhantomData<T>,
}

impl<T: PortWrite> WriteOnlyPort<T> {
    /// Creates a new write-only port handle.
    #[inline]
    pub const fn new(port: u16) -> Self {
        Self {
            port,
            _phantom: PhantomData,
        }
    }

    /// Returns the port number.
    #[inline]
    pub const fn port(&self) -> u16 {
        self.port
    }

    /// Writes a value to this port.
    ///
    /// # Safety
    ///
    /// The caller must ensure this port is valid and safe to write.
    #[inline]
    pub unsafe fn write(&self, value: T) {
        unsafe { T::write_to_port(self.port, value) }
    }
}
