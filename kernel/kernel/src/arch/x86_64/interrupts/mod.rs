//! Interrupt management: CPU exception handlers and hardware interrupt dispatch.

pub mod dispatch;
pub(crate) mod exception_table;
pub mod handlers;

pub use dispatch::{
    InterruptError, InterruptHandler, alloc_vector, register_handler, unregister_handler, vectors,
};
pub use hadron_core::id::HwIrqVector;
