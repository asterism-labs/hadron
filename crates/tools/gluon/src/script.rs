//! Script execution engine for QEMU automation.
//!
//! Boots the kernel in QEMU with a PTY serial port and QMP socket, then
//! evaluates a Rhai script with VM automation bindings. Supports both
//! file-based scripts and an interactive REPL.

use std::io::Write;
use std::sync::Arc;

use anyhow::{Context, Result};

use hadron_runner::scripting::ScriptableVm;

use crate::cli::ScriptArgs;
use crate::config::ResolvedConfig;

/// Boots QEMU with serial PTY + QMP and returns a `ScriptableVm`.
fn boot_vm(
    config: &ResolvedConfig,
    kernel_binary: &std::path::Path,
    extra_args: &[String],
) -> Result<ScriptableVm> {
    use hadron_runner::{DisplayConfig, QemuConfig, SerialConfig, SerialPty};

    let serial_pty = SerialPty::new().context("creating serial PTY")?;
    let slave_path = serial_pty.slave_path().to_path_buf();

    let memory = config.profile.qemu_memory.unwrap_or(config.qemu.memory);
    let cores = config.profile.qemu_cores.unwrap_or(1);

    // Build OVMF boot args.
    let ovmf = crate::run::ensure_ovmf_pub(config)?;
    let boot_args = vec![
        "-drive".to_string(),
        format!(
            "if=pflash,format=raw,readonly=on,file={}",
            ovmf.code.display()
        ),
        "-drive".to_string(),
        format!(
            "if=pflash,format=raw,readonly=on,file={}",
            ovmf.vars.display()
        ),
        "-kernel".to_string(),
        kernel_binary.to_string_lossy().into_owned(),
    ];

    // QMP socket path.
    let qmp_socket = config.root.join("target/hadron-runner/qmp-script.sock");
    if qmp_socket.exists() {
        std::fs::remove_file(&qmp_socket).ok();
    }

    let mut qemu_extra_args = config.qemu.extra_args.clone();
    if let Some(ref profile_args) = config.profile.qemu_extra_args {
        qemu_extra_args.extend(profile_args.iter().cloned());
    }
    qemu_extra_args.extend(extra_args.iter().cloned());

    let qemu_config = QemuConfig {
        machine: config.qemu.machine.clone(),
        memory,
        cores,
        cpu: "max".to_string(),
        boot_args,
        serial: vec![SerialConfig::Pty(slave_path)],
        qmp_socket: Some(qmp_socket.clone()),
        display: DisplayConfig::None,
        extra_args: qemu_extra_args,
        test_mode: None,
    };

    let qemu = qemu_config.spawn().context("spawning QEMU for scripting")?;

    // Give QEMU time to start and open QMP socket.
    std::thread::sleep(std::time::Duration::from_millis(500));

    let qmp = hadron_runner::QmpClient::connect(&qmp_socket).ok();

    Ok(ScriptableVm::new(qemu, serial_pty, qmp))
}

/// Registers VM automation functions on the Rhai engine.
fn register_vm_bindings(engine: &mut rhai::Engine, vm: &Arc<ScriptableVm>) {
    engine.register_type_with_name::<Arc<ScriptableVm>>("Vm");

    engine.register_fn("wait_serial", {
        let vm = Arc::clone(vm);
        move |pattern: &str, timeout: i64| -> Result<String, Box<rhai::EvalAltResult>> {
            vm.wait_serial(pattern, timeout)
                .map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("send_serial", {
        let vm = Arc::clone(vm);
        move |data: &str| -> Result<(), Box<rhai::EvalAltResult>> {
            vm.send_serial(data).map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("screenshot", {
        let vm = Arc::clone(vm);
        move |path: &str| -> Result<(), Box<rhai::EvalAltResult>> {
            vm.screenshot(path).map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("send_key", {
        let vm = Arc::clone(vm);
        move |keys: &str| -> Result<(), Box<rhai::EvalAltResult>> {
            vm.send_key(keys).map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("quit", {
        let vm = Arc::clone(vm);
        move || -> Result<(), Box<rhai::EvalAltResult>> {
            vm.quit().map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("wait_exit", {
        let vm = Arc::clone(vm);
        move |timeout: i64| -> Result<i64, Box<rhai::EvalAltResult>> {
            vm.wait_exit(timeout).map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("assert_exit", {
        let vm = Arc::clone(vm);
        move |expected: i64| -> Result<(), Box<rhai::EvalAltResult>> {
            vm.assert_exit(expected).map_err(|e| e.to_string().into())
        }
    });

    engine.register_fn("serial_log", {
        let vm = Arc::clone(vm);
        move || -> rhai::Array {
            vm.serial_log()
                .into_iter()
                .map(rhai::Dynamic::from)
                .collect()
        }
    });

    engine.register_fn("boot", {
        let vm = Arc::clone(vm);
        move || -> rhai::Dynamic { rhai::Dynamic::from(Arc::clone(&vm)) }
    });

    engine.register_fn("sleep", |secs: i64| {
        std::thread::sleep(std::time::Duration::from_secs(secs.unsigned_abs()));
    });
}

/// Runs the script subcommand: build, boot, evaluate script or REPL.
pub fn cmd_script(
    config: &ResolvedConfig,
    kernel_binary: &std::path::Path,
    args: &ScriptArgs,
) -> Result<()> {
    println!("Booting kernel for scripting...");
    let vm = Arc::new(boot_vm(config, kernel_binary, &args.extra_args)?);
    println!("VM booted. Setting up Rhai engine...");

    let mut engine = rhai::Engine::new();
    register_vm_bindings(&mut engine, &vm);

    if let Some(ref script_path) = args.script {
        let source = std::fs::read_to_string(script_path)
            .with_context(|| format!("reading script {}", script_path.display()))?;
        let _ = engine
            .eval::<rhai::Dynamic>(&source)
            .map_err(|e| anyhow::anyhow!("script error: {e}"))?;
    } else {
        run_repl(&engine);
    }

    Ok(())
}

/// Interactive REPL loop for ad-hoc VM scripting.
fn run_repl(engine: &rhai::Engine) {
    println!("Hadron script REPL. Type Rhai expressions. Ctrl-D to exit.");
    println!("  let vm = boot();");
    println!("  vm.wait_serial(\"pattern\", 10);");
    println!("  vm.quit();");
    println!();

    let mut scope = rhai::Scope::new();

    loop {
        print!("hadron> ");
        std::io::stdout().flush().ok();

        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match engine.eval_with_scope::<rhai::Dynamic>(&mut scope, trimmed) {
                    Ok(result) => {
                        if !result.is_unit() {
                            println!("=> {result}");
                        }
                    }
                    Err(e) => {
                        eprintln!("error: {e}");
                    }
                }
            }
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }
    }
}
