//! System structures as plain data, with no inline assembly.
//!
//! References are to the Intel SDM, Volume 3A: §3.4.2 (segment selectors),
//! §3.4.5 (segment descriptors), §3.5 (system descriptor types) and the
//! chapter on task management for the 64-bit TSS and its descriptor, §6.3
//! (exceptions) and §6.14 (64-bit interrupt handling), and §5.5 (4-level
//! paging).

pub mod descriptor;
pub mod exception;
pub mod idt;
pub mod paging;
pub mod segment;
pub mod table;
pub mod tss;
