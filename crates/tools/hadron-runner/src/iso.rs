//! ISO image builder wrapping `hadris-iso`.
//!
//! Creates bootable ISO 9660 images with El Torito BIOS boot support,
//! using a staging directory approach matching `cargo-image-runner`'s layout.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hadris_iso::boot::EmulationType;
use hadris_iso::boot::options::{BootEntryOptions, BootOptions};
use hadris_iso::joliet::JolietLevel;
use hadris_iso::read::PathSeparator;
use hadris_iso::rrip::RripOptions;
use hadris_iso::write::options::{
    BaseIsoLevel, CreationFeatures, FormatOptions, HybridBootOptions,
};
use hadris_iso::write::{InputFiles, IsoImageWriter};

/// Builder for creating bootable ISO images.
///
/// Assembles a staging directory with bootloader binaries, kernel, config,
/// and extra files, then produces an ISO with El Torito BIOS boot.
pub struct IsoBuilder {
    /// Path to the directory containing extracted Limine binaries.
    limine_dir: PathBuf,
    /// Content of `limine.conf`.
    config_content: String,
    /// Path to the kernel binary (e.g. `hadron_boot_limine`).
    kernel_binary: PathBuf,
    /// Extra files to include: `(iso_path, host_path)`.
    extra_files: Vec<(String, PathBuf)>,
}

impl IsoBuilder {
    /// Create a new ISO builder.
    ///
    /// `limine_dir` should contain `limine-bios.sys` and `limine-bios-cd.bin`.
    /// `kernel_binary` is the path to the built kernel ELF/binary.
    #[must_use]
    pub fn new(limine_dir: &Path, kernel_binary: &Path) -> Self {
        Self {
            limine_dir: limine_dir.to_path_buf(),
            config_content: String::new(),
            kernel_binary: kernel_binary.to_path_buf(),
            extra_files: Vec::new(),
        }
    }

    /// Set the `limine.conf` content.
    pub fn config(&mut self, content: &str) -> &mut Self {
        self.config_content = content.to_string();
        self
    }

    /// Add an extra file to the ISO.
    ///
    /// `iso_path` is the path inside the ISO (e.g. `"boot/initrd.cpio"`).
    /// `host_path` is the source file on the host filesystem.
    pub fn extra_file(&mut self, iso_path: &str, host_path: &Path) -> &mut Self {
        self.extra_files
            .push((iso_path.to_string(), host_path.to_path_buf()));
        self
    }

    /// Build the ISO image and write it to `output`.
    ///
    /// # Errors
    ///
    /// Returns an error if staging population, ISO creation, or cleanup fails.
    pub fn build(&self, output: &Path) -> Result<()> {
        let staging = output.with_extension("staging");

        // Clean and create staging directory
        if staging.exists() {
            std::fs::remove_dir_all(&staging).context("cleaning staging directory")?;
        }

        self.populate_staging(&staging)?;
        Self::create_iso(&staging, output)?;

        // Clean up staging
        std::fs::remove_dir_all(&staging).context("cleaning up staging directory")?;

        Ok(())
    }

    /// Populate the staging directory with all files.
    fn populate_staging(&self, staging: &Path) -> Result<()> {
        let boot_dir = staging.join("boot");
        std::fs::create_dir_all(&boot_dir).context("creating boot directory in staging")?;

        // limine.conf
        if !self.config_content.is_empty() {
            std::fs::write(boot_dir.join("limine.conf"), &self.config_content)
                .context("writing limine.conf")?;
        }

        // Kernel binary -> boot/<filename>
        let kernel_name = self
            .kernel_binary
            .file_name()
            .context("kernel binary has no filename")?;
        std::fs::copy(&self.kernel_binary, boot_dir.join(kernel_name))
            .with_context(|| format!("copying kernel binary {}", self.kernel_binary.display()))?;

        // Limine BIOS files -> root of ISO (not under boot/)
        // The El Torito boot image path references files relative to ISO root.
        for file in ["limine-bios.sys", "limine-bios-cd.bin"] {
            let src = self.limine_dir.join(file);
            if !src.exists() {
                bail!("{file} not found in {}", self.limine_dir.display());
            }
            std::fs::copy(&src, staging.join(file)).with_context(|| format!("copying {file}"))?;
        }

        // Extra files (e.g. initrd.cpio)
        for (iso_path, host_path) in &self.extra_files {
            let dest = staging.join(iso_path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating directory for {iso_path}"))?;
            }
            std::fs::copy(host_path, &dest)
                .with_context(|| format!("copying extra file {}", host_path.display()))?;
        }

        Ok(())
    }

    /// Create the ISO from the populated staging directory.
    fn create_iso(staging: &Path, output: &Path) -> Result<()> {
        // Remove existing output
        if output.exists() {
            std::fs::remove_file(output).context("removing existing ISO")?;
        }

        // Configure El Torito BIOS boot
        let boot_options = BootOptions {
            write_boot_catalog: true,
            default: BootEntryOptions {
                boot_image_path: "limine-bios-cd.bin".to_string(),
                load_size: None,
                emulation: EmulationType::NoEmulation,
                boot_info_table: true,
                grub2_boot_info: false,
            },
            entries: vec![],
        };

        // Scan staging directory into hadris-iso file tree
        let iso_files = InputFiles::from_fs(staging, PathSeparator::ForwardSlash)
            .context("reading staging directory")?;

        let features = CreationFeatures {
            filenames: BaseIsoLevel::Level2 {
                supports_lowercase: true,
                supports_rrip: true,
            },
            long_filenames: true,
            joliet: Some(JolietLevel::Level3),
            rock_ridge: Some(RripOptions::default()),
            el_torito: Some(boot_options),
            hybrid_boot: Some(HybridBootOptions::mbr()),
        };

        let format_options = FormatOptions {
            volume_name: "HADRON".to_string(),
            system_id: None,
            volume_set_id: None,
            publisher_id: None,
            preparer_id: None,
            application_id: None,
            sector_size: 2048,
            path_separator: PathSeparator::ForwardSlash,
            features,
        };

        // Pre-allocate ISO file: content size + 1 MiB overhead, sector-aligned
        let content_size = calculate_dir_size(staging);
        let iso_size = (content_size + 1024 * 1024).div_ceil(2048) * 2048;

        let rw_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(output)
            .with_context(|| format!("creating ISO file {}", output.display()))?;
        rw_file
            .set_len(iso_size)
            .context("pre-allocating ISO file")?;

        IsoImageWriter::format_new(rw_file, iso_files, format_options)
            .context("creating ISO image")?;

        Ok(())
    }
}

/// Recursively calculate total file size in a directory.
fn calculate_dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                total += calculate_dir_size(&path);
            } else if let Ok(meta) = path.metadata() {
                total += meta.len();
            }
        }
    }
    total
}
