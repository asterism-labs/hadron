// Quon build for Hadron.
//
//     quon build         build every crate
//     quon clippy        lint every crate
//     quon run           boot the kernel under QEMU through UEFI and Limine
//     quon run -p test   boot the kernel and run the kernel tests
//     quon test          run the host tests, the integration tests and
//                         the kernel tests

use quon_config::{Config, ConfigValue, Result};

/// Workspace library crates. The kernel package is declared on its own.
const LIBRARIES: &[&str] = &[
    "acpi", "arch-x86_64", "dwarf", "elf", "fdt", "ktest", "limine", "linkset", "mm", "uefi",
];

/// Libraries whose unit tests run on the host.
const HOST_TESTS: &[&str] = &["acpi", "arch-x86_64", "dwarf", "elf", "fdt", "limine", "mm"];

pub fn config(cfg: &mut Config) -> Result<()> {
    cfg.project("hadron", "0.1.0").default_profile("dev");
    let x86_64 = cfg.target("x86_64-unknown-none");
    cfg.cargo_workspace(".");

    cfg.profile("dev")
        .runner("qemu")
        .target(x86_64)
        .opt_level(0)
        .debug_info(true)
        .run_binary("kernel");

    // The kernel reads these as `hadron_config::KTEST` and
    // `hadron_config::KTEST_IGNORED`. The `test` profile turns on KTEST.
    // `QUON_KTEST_IGNORED=true` also runs the `#[ignore]` tests.
    cfg.config_bool("KTEST")
        .default_value(ConfigValue::Bool(false))
        .help("Run the kernel tests after boot instead of exiting");
    cfg.config_bool("KTEST_IGNORED")
        .default_value(ConfigValue::Bool(false))
        .help("Also run the kernel tests marked #[ignore]");
    // The kernel reads this as `hadron_config::LOCKDEP`. It is on by
    // default; a release profile will turn it off.
    cfg.config_bool("LOCKDEP")
        .default_value(ConfigValue::Bool(true))
        .help("Check lock order at run time");
    cfg.preset("test").set("KTEST", ConfigValue::Bool(true));
    cfg.profile("test").inherits("dev").preset("test");
    cfg.boot_test("ktest").profile("test");

    // x86_64-unknown-none links a position-independent executable by
    // default. Limine loads one only with a PT_DYNAMIC segment, so link a
    // plain executable at the address kernel.ld sets.
    cfg.group("kernel")
        .target(x86_64)
        .config(true)
        .package("kernel")
        .linker_script("kernel/kernel/kernel.ld")
        .rustc_flag("-Crelocation-model=static");

    // Quon builds a package for one target only, so the libraries do not
    // build for aarch64-unknown-none yet.
    let mut libraries = cfg.group("libraries").target(x86_64);
    for name in LIBRARIES {
        libraries.package(name);
    }
    drop(libraries);
    let mut host_tests = cfg.host_tests();
    for name in HOST_TESTS {
        host_tests = host_tests.package(name);
    }
    drop(host_tests);

    // To upgrade Limine, change the version and the hash. An empty hash
    // makes the build fail and print the hash of the file it fetched.
    cfg.limine("11.4.1").file(
        "BOOTX64.EFI",
        "333f7a69379b1f47e019be215885cdc078b9edd6e9255cbe2f706d6c5ec0ef06",
    );

    // UEFI firmware starts EFI/BOOT/BOOTX64.EFI. Limine reads limine.conf
    // from the ESP root, and limine.conf names boot/kernel.
    cfg.esp("default")
        .add_download("limine/BOOTX64.EFI", "EFI/BOOT/BOOTX64.EFI")
        .add_file("limine.conf", "limine.conf")
        .add("kernel", "boot/kernel");

    // The kernel writes 0x10 to isa-debug-exit on success, and QEMU exits
    // with (0x10 << 1) | 1 = 33.
    cfg.runner("qemu").qemu("qemu-system-x86_64")
        .machine("q35")
        .memory(256)
        .cores(1)
        .boot_mode("uefi")
        // OVMF puts its console, which Limine also writes through, on COM1
        // and COM2. The kernel writes to COM3. Only COM3 reaches the
        // terminal.
        .extra_args(&[
            "-serial",
            "null",
            "-serial",
            "null",
            "-serial",
            "stdio",
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x04",
        ])
        .success_exit_code(33);

    Ok(())
}
