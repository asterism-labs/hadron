//! Shared Wayland wire protocol library for compositor and client.
//!
//! Provides the wire encoding/decoding, Unix socket helpers, and protocol
//! constants used by both `lepton-compositor` (server) and
//! `lepton-display-client` (client).

#![no_std]

extern crate alloc;

pub mod consts;
pub mod net;
pub mod wire;
