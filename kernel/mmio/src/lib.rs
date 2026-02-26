//! Typed MMIO register block abstractions.
//!
//! This crate re-exports the [`register_block!`] macro from
//! `hadron-mmio-macros`, which generates safe, typed MMIO register accessor
//! structs from a declarative definition. The generated code consolidates all
//! `unsafe` volatile access into the struct's `new()` constructor, making all
//! individual register reads and writes safe.
//!
//! # Example
//!
//! ```ignore
//! use hadron_mmio::register_block;
//!
//! register_block! {
//!     /// HPET timer registers.
//!     pub HpetRegs {
//!         /// General Capabilities and ID.
//!         [0x000; u64; ro] capabilities,
//!         /// General Configuration.
//!         [0x010; u64; rw] configuration,
//!         /// Main Counter Value.
//!         [0x0F0; u64; rw] main_counter,
//!     }
//! }
//! ```

#![no_std]
#![warn(missing_docs)]

pub use hadron_mmio_macros::register_block;
