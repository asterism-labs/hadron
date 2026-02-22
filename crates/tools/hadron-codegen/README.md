# hadron-codegen

A host-side build tool for generating Rust source files at build time, currently focused on bitmap font rasterization. It reads a `codegen.toml` configuration file, rasterizes TTF fonts (via `fontdue`) or uses an embedded VGA 8x16 fallback, and emits `no_std`-compatible `.rs` files containing bitmap font data for use by the kernel's framebuffer console.

## Features

- Rasterizes TTF fonts into 1bpp bitmap or 8bpp grayscale pixel data at configurable sizes and codepoint ranges
- Includes an embedded public-domain VGA 8x16 BIOS font as a fallback when no TTF file is specified
- Configuration driven by `codegen.toml` with support for multiple font specifications, each with its own output path
- Generates checked-in `no_std` Rust source files with `const` arrays suitable for direct use in kernel code
- Invoked via `cargo xtask codegen` as part of the build pipeline
