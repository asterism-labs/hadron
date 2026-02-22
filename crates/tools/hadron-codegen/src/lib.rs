//! Build-time code generator for the Hadron kernel.
//!
//! This crate provides tools for generating Rust source files at build time,
//! currently focused on bitmap font rasterization from TTF files. Generated
//! files are `no_std`-compatible and checked into the repository.
//!
//! # Usage
//!
//! Invoke via `cargo xtask codegen`, which reads `codegen.toml` at the
//! workspace root and writes generated `.rs` files to the configured paths.

pub mod config;
pub mod error;
pub mod font;
