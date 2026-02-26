//! TLB (Translation Lookaside Buffer) management instructions.

use crate::addr::VirtAddr;
use crate::arch::x86_64::registers::control::Cr3;

/// Flushes the TLB entry for the given virtual address (INVLPG).
#[inline]
pub fn flush(addr: VirtAddr) {
    // SAFETY: INVLPG only invalidates a single TLB entry and has no other
    // side effects.
    unsafe {
        core::arch::asm!(
            "invlpg [{}]",
            in(reg) addr.as_u64(),
            options(nostack, preserves_flags),
        );
    }
}

/// Flushes the entire TLB by reloading CR3.
#[inline]
pub fn flush_all() {
    // SAFETY: Writing back the same CR3 value only flushes non-global TLB
    // entries. The page table root remains unchanged.
    unsafe { Cr3::write(Cr3::read()) };
}
