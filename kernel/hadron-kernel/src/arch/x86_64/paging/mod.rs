//! x86_64 page table types and utilities.
//!
//! Re-exports type definitions from `hadron_core`.

pub use hadron_core::arch::x86_64::paging::{PageTableMapper, TranslateResult, UnmapError};
pub use hadron_core::arch::x86_64::structures::paging::{
    PageTable, PageTableEntry, PageTableFlags,
};
