//! Interrupt management: CPU exception handlers and hardware interrupt dispatch.

pub mod dispatch;
pub mod handlers;

pub use dispatch::{
    InterruptError, InterruptHandler, alloc_vector, register_handler, unregister_handler, vectors,
};
