//! Configuration types for the code generator.
//!
//! Deserialized from `codegen.toml` at the workspace root.

use serde::Deserialize;
use std::path::PathBuf;

/// Pixel format for generated font data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// 1 bit per pixel, MSB = leftmost pixel (matches VGA BIOS layout).
    Bitmap1bpp,
    /// 8 bits per pixel, one byte of coverage per pixel (0-255).
    Grayscale8bpp,
}

/// Specification for a single font to generate.
#[derive(Debug, Clone, Deserialize)]
pub struct FontSpec {
    /// Short name used as module/identifier prefix (e.g. "console").
    pub name: String,
    /// Path to a TTF font file, relative to workspace root.
    /// If absent, the embedded VGA 8x16 fallback is used.
    pub ttf_path: Option<PathBuf>,
    /// Pixel heights to rasterize (e.g. `[16]`).
    pub sizes: Vec<u32>,
    /// Inclusive codepoint ranges (e.g. `[[0x00, 0x7F]]`).
    pub ranges: Vec<[u32; 2]>,
    /// Pixel format for the output data.
    pub format: PixelFormat,
    /// Output file path, relative to workspace root.
    pub output: PathBuf,
}

/// Top-level codegen configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct CodegenConfig {
    /// Font generation specifications.
    pub fonts: Vec<FontSpec>,
}
