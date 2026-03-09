//! SIMD intrinsic wrappers for the Hadron kernel.
//!
//! This crate provides thin, `#[inline(always)]` wrappers around SSE2/AVX
//! inline assembly, plus safe SIMD types. The raw intrinsics are `unsafe`
//! and require:
//!
//! 1. The CPU supports the relevant feature (checked via `cpuid`).
//! 2. The caller holds a `KernelFpuGuard` (FPU state saved, interrupts off).
//!
//! The crate compiles to nothing on non-x86_64 targets.

#![no_std]
#![warn(missing_docs)]

#[cfg(target_arch = "x86_64")]
pub mod x86_64;
