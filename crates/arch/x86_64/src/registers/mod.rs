//! CPU registers. Each register is a unit type whose `read` and `write`
//! functions build only for `x86_64`. The flag types build everywhere.

pub mod control;
pub mod segmentation;
