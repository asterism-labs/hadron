//! QEMU invocation for running and testing the kernel.
//!
//! Supports both Limine ISO boot (legacy) and UEFI direct boot via OVMF.
//! Uses `hadron-runner` for ISO creation and QEMU execution.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};

use hadron_runner::{
    DisplayConfig, IsoBuilder, LimineCache, QemuConfig, QemuExit, SerialConfig, TestConfig,
};

use crate::config::ResolvedConfig;

/// Limine binary release version.
const LIMINE_VERSION: &str = "v10.7.0-binary";

/// OVMF firmware paths (CODE + VARS) for UEFI boot.
struct OvmfFirmware {
    code: PathBuf,
    vars: PathBuf,
}

/// Fetch OVMF firmware, downloading prebuilt binaries if needed.
///
/// Uses the `ovmf-prebuilt` crate to download and cache OVMF firmware.
/// Set `OVMF_CODE` and `OVMF_VARS` environment variables to override.
fn ensure_ovmf(config: &ResolvedConfig) -> Result<OvmfFirmware> {
    // Allow environment variable overrides.
    if let (Ok(code), Ok(vars)) = (std::env::var("OVMF_CODE"), std::env::var("OVMF_VARS")) {
        let code = PathBuf::from(&code);
        let vars = PathBuf::from(&vars);
        if !code.exists() {
            bail!("OVMF_CODE={} does not exist", code.display());
        }
        if !vars.exists() {
            bail!("OVMF_VARS={} does not exist", vars.display());
        }
        return Ok(OvmfFirmware { code, vars });
    }

    let cache_dir = config.root.join("target/ovmf");
    let prebuilt = ovmf_prebuilt::Prebuilt::fetch(ovmf_prebuilt::Source::LATEST, &cache_dir)
        .map_err(|e| anyhow::anyhow!("failed to fetch OVMF prebuilt: {e}"))?;

    let code = prebuilt
        .get_file(ovmf_prebuilt::Arch::X64, ovmf_prebuilt::FileType::Code)
        .to_path_buf();
    let vars = prebuilt
        .get_file(ovmf_prebuilt::Arch::X64, ovmf_prebuilt::FileType::Vars)
        .to_path_buf();

    Ok(OvmfFirmware { code, vars })
}

/// Ensure Limine binaries are cached and return the path.
fn ensure_limine(config: &ResolvedConfig) -> Result<PathBuf> {
    let cache_dir = config.root.join("target/hadron-runner/cache/bootloaders");
    let cache = LimineCache::new(&cache_dir, LIMINE_VERSION);
    cache.ensure_available()
}

/// Read the limine.conf content, applying template substitution.
fn read_limine_conf(config: &ResolvedConfig) -> Result<String> {
    let config_path = if let Some(ref path) = config.bootloader.config_file {
        config.root.join(path)
    } else {
        config.root.join("limine.conf")
    };

    std::fs::read_to_string(&config_path)
        .with_context(|| format!("reading {}", config_path.display()))
}

/// Build an ISO image for the kernel.
///
/// Returns the path to the created ISO.
fn build_iso(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    limine_conf: &str,
    extra_files: &[(String, PathBuf)],
) -> Result<PathBuf> {
    let limine_dir = ensure_limine(config)?;

    let output_dir = config.root.join("target/hadron-runner/output");
    std::fs::create_dir_all(&output_dir).context("creating output directory")?;
    let iso_path = output_dir.join("image.iso");

    let mut builder = IsoBuilder::new(&limine_dir, kernel_binary);
    builder.config(limine_conf);

    // Add extra files from config (e.g. initrd.cpio)
    for (iso_path_str, host_path_str) in &config.image.extra_files {
        let host_path = config.root.join(host_path_str);
        builder.extra_file(iso_path_str, &host_path);
    }

    // Add caller-provided extra files
    for (iso_path_str, host_path) in extra_files {
        builder.extra_file(iso_path_str, host_path);
    }

    builder.build(&iso_path).context("building ISO image")?;
    Ok(iso_path)
}

/// Build a [`QemuConfig`] from hadron's [`ResolvedConfig`] with ISO boot.
fn build_qemu_config_iso(
    config: &ResolvedConfig,
    iso_path: PathBuf,
    is_test: bool,
    extra_args: &[String],
) -> QemuConfig {
    let boot_args = vec![
        "-cdrom".to_string(),
        iso_path.to_string_lossy().into_owned(),
    ];
    build_qemu_config_common(config, boot_args, is_test, extra_args)
}

/// Build a [`QemuConfig`] from hadron's [`ResolvedConfig`] with UEFI boot.
fn build_qemu_config_uefi(
    config: &ResolvedConfig,
    ovmf: &OvmfFirmware,
    efi_binary: &Path,
    is_test: bool,
    extra_args: &[String],
) -> QemuConfig {
    let boot_args = vec![
        // OVMF CODE (read-only pflash)
        "-drive".to_string(),
        format!(
            "if=pflash,format=raw,readonly=on,file={}",
            ovmf.code.display()
        ),
        // OVMF VARS (writable pflash)
        "-drive".to_string(),
        format!(
            "if=pflash,format=raw,readonly=on,file={}",
            ovmf.vars.display()
        ),
        // EFI binary loaded directly as kernel
        "-kernel".to_string(),
        efi_binary.to_string_lossy().into_owned(),
    ];
    build_qemu_config_common(config, boot_args, is_test, extra_args)
}

/// Common QEMU config builder.
fn build_qemu_config_common(
    config: &ResolvedConfig,
    boot_args: Vec<String>,
    is_test: bool,
    extra_args: &[String],
) -> QemuConfig {
    let memory = config.profile.qemu_memory.unwrap_or(config.qemu.memory);
    let cores = config.profile.qemu_cores.unwrap_or(1);

    let mut qemu_extra_args = config.qemu.extra_args.clone();
    if let Some(ref profile_args) = config.profile.qemu_extra_args {
        qemu_extra_args.extend(profile_args.iter().cloned());
    }

    // Strip any user-provided -serial flags — we manage serial directly.
    let mut filtered_args = strip_serial_args(&qemu_extra_args);

    // Extra args from the caller
    filtered_args.extend(extra_args.iter().cloned());

    let test_mode = if is_test {
        let test_cfg = &config.qemu.test;
        let timeout = config.profile.test_timeout.unwrap_or(test_cfg.timeout);

        // Add test-specific QEMU args from config
        filtered_args.extend(test_cfg.extra_args.iter().cloned());

        Some(TestConfig {
            success_exit_code: test_cfg.success_exit_code as i32,
            timeout_secs: u64::from(timeout),
        })
    } else {
        None
    };

    let display = if is_test {
        DisplayConfig::None
    } else {
        DisplayConfig::Default
    };

    QemuConfig {
        machine: config.qemu.machine.clone(),
        memory,
        cores,
        cpu: "max".to_string(),
        boot_args,
        serial: vec![SerialConfig::Stdio],
        qmp_socket: None,
        display,
        extra_args: filtered_args,
        test_mode,
    }
}

/// Strip `-serial <arg>` pairs from QEMU args since we manage serial directly.
fn strip_serial_args(args: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    let mut skip_next = false;
    for arg in args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "-serial" {
            skip_next = true;
            continue;
        }
        result.push(arg.clone());
    }
    result
}

/// Run the kernel in QEMU using UEFI direct boot via OVMF.
///
/// Detects OVMF firmware and launches QEMU with UEFI pflash and `-kernel` flags.
pub fn run_kernel(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    let ovmf = ensure_ovmf(config)?;
    println!("Using OVMF: {}", ovmf.code.display());
    println!("Running kernel in QEMU (UEFI)...");

    let qemu_config = build_qemu_config_uefi(config, &ovmf, kernel_binary, false, extra_args);

    let mut qemu = qemu_config.spawn().context("failed to spawn QEMU")?;
    let exit = qemu.wait().context("failed to wait for QEMU")?;

    if !exit.success {
        bail!("QEMU exited with code {}", exit.exit_code);
    }
    Ok(())
}

/// Build a test ISO image for Limine-based boot.
///
/// Returns `(iso_path, qemu_config)`.
pub fn build_test_iso(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<(PathBuf, QemuConfig)> {
    let limine_conf = read_limine_conf(config)?;
    let iso_path = build_iso(config, kernel_binary, &limine_conf, &[])?;
    let qemu_config = build_qemu_config_iso(config, iso_path.clone(), true, extra_args);
    Ok((iso_path, qemu_config))
}

/// Build a UEFI test QEMU config.
///
/// Returns the `QemuConfig` for UEFI boot.
#[allow(dead_code)] // Phase 2: used by kernel integration tests with UEFI boot
pub fn build_test_uefi(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<QemuConfig> {
    let ovmf = ensure_ovmf(config)?;
    Ok(build_qemu_config_uefi(
        config,
        &ovmf,
        kernel_binary,
        true,
        extra_args,
    ))
}

/// Run a kernel integration test in QEMU.
///
/// Builds an ISO image, then runs QEMU with test configuration (isa-debug-exit
/// device, timeout, headless display).
///
/// Returns `Ok(())` if the test exits with the configured success exit code.
pub fn run_kernel_tests(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<()> {
    let (_, qemu_config) = build_test_iso(config, kernel_binary, extra_args)?;

    let timeout = qemu_config
        .test_mode
        .as_ref()
        .map(|t| Duration::from_secs(t.timeout_secs));

    let mut qemu = qemu_config.spawn().context("failed to spawn QEMU")?;

    let exit = if let Some(timeout) = timeout {
        qemu.wait_with_timeout(timeout)
            .context("failed to wait for QEMU")?
    } else {
        qemu.wait().context("failed to wait for QEMU")?
    };

    if exit.timed_out {
        bail!("kernel test timed out");
    }
    if !exit.success {
        bail!("kernel test failed (exit code {})", exit.exit_code);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Userspace test runner
// ---------------------------------------------------------------------------

/// Write a Limine config for a userspace test.
///
/// Reads `limine.conf` from the project root and substitutes:
/// - `cmdline: {{ARGS}}` -> `cmdline: --utest`
/// - `module_path: boot():/boot/initrd.cpio` -> `module_path: boot():/boot/utest.cpio`
///
/// The result is written to `build/utests/limine-<test_name>.conf`.
pub fn write_utest_limine_conf(root: &Path, test_name: &str) -> Result<PathBuf> {
    let src = root.join("limine.conf");
    let contents =
        std::fs::read_to_string(&src).with_context(|| format!("reading {}", src.display()))?;

    let patched = contents
        .replace("cmdline: {{ARGS}}", "cmdline: --utest")
        .replace(
            "module_path: boot():/boot/initrd.cpio",
            "module_path: boot():/boot/utest.cpio",
        );

    let out_dir = root.join("build/utests");
    std::fs::create_dir_all(&out_dir)?;
    let out_path = out_dir.join(format!("limine-{test_name}.conf"));
    std::fs::write(&out_path, patched)
        .with_context(|| format!("writing {}", out_path.display()))?;
    Ok(out_path)
}

/// Run a single userspace test binary in QEMU.
///
/// Builds a minimal ISO containing the utest CPIO (as `boot/utest.cpio`)
/// and a patched Limine config that passes `--utest` on the kernel cmdline.
/// QEMU exits 33 on success (PID 1 exits 0) or 35 on failure.
pub fn run_userspace_test(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    test_name: &str,
    cpio_path: &Path,
    extra_args: &[String],
) -> Result<()> {
    let limine_conf_path = write_utest_limine_conf(&config.root, test_name)?;
    let limine_conf = std::fs::read_to_string(&limine_conf_path)
        .with_context(|| format!("reading {}", limine_conf_path.display()))?;

    // Extra file: the per-test CPIO
    let extra_files = vec![("boot/utest.cpio".to_string(), cpio_path.to_path_buf())];

    let iso_path = build_iso(config, kernel_binary, &limine_conf, &extra_files)?;
    let qemu_config = build_qemu_config_iso(config, iso_path, true, extra_args);

    let timeout = qemu_config
        .test_mode
        .as_ref()
        .map(|t| Duration::from_secs(t.timeout_secs));

    let mut qemu = qemu_config.spawn().context("failed to spawn QEMU")?;

    let exit = if let Some(timeout) = timeout {
        qemu.wait_with_timeout(timeout)
            .context("failed to wait for QEMU")?
    } else {
        qemu.wait().context("failed to wait for QEMU")?
    };

    if exit.timed_out {
        bail!("userspace test '{test_name}' timed out");
    }
    if !exit.success {
        bail!(
            "userspace test '{test_name}' failed (exit code {})",
            exit.exit_code
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Serial capture via IoHandler
// ---------------------------------------------------------------------------

/// IO handler that accumulates raw serial bytes and tees output to stderr.
///
/// Preserves binary data (HPRF/HBENCH) while providing real-time visibility.
struct StderrTeeHandler {
    serial: Vec<u8>,
}

impl hadron_runner::qemu::IoHandler for StderrTeeHandler {
    fn on_output(&mut self, data: &[u8]) -> bool {
        use std::io::Write;
        self.serial.extend_from_slice(data);
        let _ = std::io::stderr().write_all(data);
        true
    }
}

/// Run a kernel binary in QEMU with serial output captured as raw bytes.
///
/// Builds a test ISO and runs QEMU with piped IO, capturing serial data
/// while teeing to stderr for real-time visibility.
///
/// Returns `(QemuExit, captured_serial_bytes)`.
pub fn run_with_serial_capture(
    config: &ResolvedConfig,
    kernel_binary: &Path,
    extra_args: &[String],
) -> Result<(QemuExit, Vec<u8>)> {
    let (_, qemu_config) = build_test_iso(config, kernel_binary, extra_args)?;

    let mut qemu = qemu_config
        .spawn_piped()
        .context("failed to spawn QEMU for serial capture")?;

    let mut handler = StderrTeeHandler { serial: Vec::new() };

    let exit = qemu
        .wait_with_io(&mut handler)
        .context("failed to run QEMU with IO capture")?;

    Ok((exit, handler.serial))
}
