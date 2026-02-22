//! Type-safe identifiers for kernel resources.
//!
//! These newtypes prevent accidental mixing of PIDs, file descriptors,
//! CPU IDs, and IRQ vectors at compile time.

use core::fmt;

/// Process identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Pid(u32);

impl Pid {
    /// Creates a new `Pid`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw `u32` value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// CPU identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct CpuId(u32);

impl CpuId {
    /// Creates a new `CpuId`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw `u32` value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }
}

impl fmt::Display for CpuId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// File descriptor number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct Fd(u32);

impl Fd {
    /// Standard input.
    pub const STDIN: Self = Self(0);
    /// Standard output.
    pub const STDOUT: Self = Self(1);
    /// Standard error.
    pub const STDERR: Self = Self(2);

    /// Creates a new `Fd`.
    pub const fn new(val: u32) -> Self {
        Self(val)
    }

    /// Returns the raw `u32` value.
    pub const fn as_u32(self) -> u32 {
        self.0
    }

    /// Returns the value as `usize` (convenience for indexing).
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for Fd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// IRQ vector number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct IrqVector(u8);

impl IrqVector {
    /// Creates a new `IrqVector`.
    pub const fn new(val: u8) -> Self {
        Self(val)
    }

    /// Returns the raw `u8` value.
    pub const fn as_u8(self) -> u8 {
        self.0
    }
}

impl fmt::Display for IrqVector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_roundtrip() {
        let pid = Pid::new(42);
        assert_eq!(pid.as_u32(), 42);
    }

    #[test]
    fn pid_display() {
        let pid = Pid::new(1);
        assert_eq!(format!("{pid}"), "1");
    }

    #[test]
    fn pid_ordering() {
        assert!(Pid::new(1) < Pid::new(2));
    }

    #[test]
    fn cpu_id_roundtrip() {
        let id = CpuId::new(7);
        assert_eq!(id.as_u32(), 7);
    }

    #[test]
    fn fd_constants() {
        assert_eq!(Fd::STDIN.as_u32(), 0);
        assert_eq!(Fd::STDOUT.as_u32(), 1);
        assert_eq!(Fd::STDERR.as_u32(), 2);
    }

    #[test]
    fn fd_as_usize() {
        assert_eq!(Fd::new(5).as_usize(), 5);
    }

    #[test]
    fn irq_vector_roundtrip() {
        let v = IrqVector::new(33);
        assert_eq!(v.as_u8(), 33);
    }
}
