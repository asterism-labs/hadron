//! Page table mapper for walking and building page tables via the HHDM.

mod mapper;

pub use mapper::{PageTableMapper, TranslateResult, UnmapError};
