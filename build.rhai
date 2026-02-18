// Hadron kernel build configuration.
//
// This script is evaluated by hadron-build to produce the build model.
// It replaces the old hadron.toml + crates.toml configuration files.

// ===========================================================================
// Project
// ===========================================================================

project("hadron", "0.1.0");

// ===========================================================================
// Targets
// ===========================================================================

target("x86_64-unknown-hadron", "targets/x86_64-unknown-hadron.json");
target("aarch64-unknown-hadron", "targets/aarch64-unknown-hadron.json");
target("x86_64-unknown-hadron-user", "targets/x86_64-unknown-hadron-user.json");

// ===========================================================================
// Kconfig-style options
// ===========================================================================

config_bool("serial_log", true)
    .menu("General")
    .help("Enable serial port logging");

config_bool("smp", true)
    .menu("SMP")
    .depends_on(["acpi"])
    .selects(["apic"])
    .help("Enable symmetric multiprocessing");

config_bool("acpi", true)
    .menu("Hardware")
    .help("Enable ACPI table parsing");

config_bool("apic", false)
    .menu("Hardware")
    .help("Enable APIC interrupt controller");

config_u32("MAX_CPUS", 256)
    .menu("SMP")
    .range(1, 256)
    .depends_on(["smp"])
    .help("Maximum number of CPUs supported");

config_choice("LOG_LEVEL", "debug", ["error", "warn", "info", "debug", "trace"])
    .menu("General")
    .help("Kernel log level");

config_u64("KERNEL_HEAP_SIZE", "0x10_0000")
    .menu("General")
    .help("Kernel heap size in bytes");

// ===========================================================================
// Build profiles
// ===========================================================================

profile("default")
    .target("x86_64-unknown-hadron")
    .opt_level(0)
    .debug_info(true)
    .qemu_cores(4)
    .boot_binary("hadron-boot-limine");

profile("release")
    .target("x86_64-unknown-hadron")
    .opt_level(2)
    .lto("thin")
    .boot_binary("hadron-boot-limine");

profile("stress")
    .inherits("default")
    .config(#{ smp: true, MAX_CPUS: 4 })
    .qemu_memory(1024)
    .qemu_cores(4)
    .test_timeout(120);

profile("debug-gdb")
    .inherits("default")
    .qemu_memory(512)
    .qemu_extra_args(["-s", "-S"])
    .test_timeout(3600);

// ===========================================================================
// Sysroot crates (built first, for kernel target)
// ===========================================================================

let sysroot = group("sysroot").target("x86_64-unknown-hadron").edition("2024").project(false);

sysroot.add("core", "{sysroot}/core")
    .root("src/lib.rs");

sysroot.add("compiler_builtins", "{sysroot}/compiler-builtins/compiler-builtins")
    .root("src/lib.rs")
    .features(["compiler-builtins", "mem", "rustc-dep-of-std"])
    .deps(#{ core: "core" });

sysroot.add("alloc", "{sysroot}/alloc")
    .root("src/lib.rs")
    .deps(#{ core: "core", compiler_builtins: "compiler_builtins" });

// ===========================================================================
// Host crates (proc-macro and its dependencies, built for host triple)
// ===========================================================================

let host = group("host").target("host").edition("2021").project(false);

host.add("unicode-ident", "vendor/unicode-ident")
    .edition("2018");

host.add("proc-macro2", "vendor/proc-macro2")
    .features(["default", "proc-macro"])
    .deps(#{ unicode_ident: "unicode-ident" });

host.add("quote", "vendor/quote")
    .features(["default", "proc-macro"])
    .deps(#{ proc_macro2: "proc-macro2" });

host.add("syn", "vendor/syn")
    .features(["full", "extra-traits", "parsing", "printing", "clone-impls", "proc-macro", "derive"])
    .deps(#{ proc_macro2: "proc-macro2", quote: "quote", unicode_ident: "unicode-ident" });

host.add("hadron-syscall-macros", "crates/hadron-syscall-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

// ===========================================================================
// Vendored external crates (built for kernel target, no_std)
// ===========================================================================

let vendored = group("vendored").target("x86_64-unknown-hadron").edition("2021").project(false);

vendored.add("bitflags", "vendor/bitflags");

vendored.add("cfg-if", "vendor/cfg-if")
    .edition("2018");

vendored.add("bytemuck", "vendor/bytemuck")
    .edition("2018");

vendored.add("noalloc-vendored", "vendor/noalloc")
    .edition("2024");

vendored.add("hadris-io", "vendor/hadris-io")
    .edition("2024")
    .deps(#{ bytemuck: "bytemuck", cfg_if: "cfg-if" });

vendored.add("hadris-common", "vendor/hadris-common")
    .edition("2024")
    .deps(#{ noalloc: "noalloc-vendored", hadris_io: "hadris-io" });

vendored.add("hadris-cpio", "vendor/hadris-cpio")
    .edition("2024")
    .features(["read", "alloc"])
    .deps(#{ hadris_io: "hadris-io", hadris_common: "hadris-common" });

// ===========================================================================
// Project kernel library crates
// ===========================================================================

let kernel_libs = group("kernel-libs").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_libs.add("noalloc", "crates/noalloc");
kernel_libs.add("hadron-elf", "crates/elf");
kernel_libs.add("hadron-dwarf", "crates/dwarf");
kernel_libs.add("hadron-acpi", "crates/acpi");

kernel_libs.add("limine", "crates/limine")
    .deps(#{ bitflags: "bitflags" });

kernel_libs.add("uefi", "crates/uefi")
    .deps(#{ bitflags: "bitflags" });

kernel_libs.add("hadron-syscall", "crates/hadron-syscall")
    .features(["kernel"])
    .deps(#{ hadron_syscall_macros: #{ "crate": "hadron-syscall-macros", proc_macro: true } });

kernel_libs.add("hadron-core", "kernel/hadron-core")
    .deps(#{ bitflags: "bitflags", hadron_syscall: "hadron-syscall", noalloc: "noalloc" });

kernel_libs.add("hadron-driver-api", "kernel/hadron-driver-api")
    .deps(#{ hadron_core: "hadron-core" });

// ===========================================================================
// Project kernel driver crates
// ===========================================================================

let kernel_drivers = group("kernel-drivers").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_drivers.add("hadron-drivers", "kernel/hadron-drivers")
    .deps(#{ bitflags: "bitflags", hadron_core: "hadron-core", hadron_driver_api: "hadron-driver-api" });

// ===========================================================================
// Test dependency crates (compiled on-demand during tests, NOT in pipeline)
// ===========================================================================

let test_deps = group("test-deps").target("x86_64-unknown-hadron").edition("2024").project(false);

test_deps.add("hadron-test", "crates/hadron-test")
    .features(["limine"])
    .deps(#{ limine: "limine" });

// ===========================================================================
// Kernel main (hadron-kernel + boot stub)
// ===========================================================================

let kernel_main = group("kernel-main").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_main.add("hadron-kernel", "kernel/hadron-kernel")
    .deps(#{
        bitflags: "bitflags",
        hadris_cpio: "hadris-cpio",
        hadris_io: "hadris-io",
        hadron_acpi: "hadron-acpi",
        hadron_core: "hadron-core",
        hadron_driver_api: "hadron-driver-api",
        hadron_drivers: "hadron-drivers",
        hadron_elf: "hadron-elf",
        hadron_syscall: "hadron-syscall",
        noalloc: "noalloc"
    })
    .dev_deps(#{
        hadron_test: "hadron-test",
        hadron_core: "hadron-core",
        limine: "limine",
        noalloc: "noalloc"
    });

kernel_main.add("hadron-boot-limine", "kernel/boot/limine")
    .crate_type(BIN)
    .root("src/main.rs")
    .linker_script("targets/x86_64-unknown-hadron.ld")
    .deps(#{
        hadron_core: "hadron-core",
        hadron_drivers: "hadron-drivers",
        hadron_kernel: "hadron-kernel",
        limine: "limine",
        noalloc: "noalloc"
    });

// ===========================================================================
// Userspace crates
// ===========================================================================

let userspace = group("userspace").target("x86_64-unknown-hadron-user").edition("2024");

userspace.add("hadron-syscall-user", "crates/hadron-syscall")
    .features(["userspace"])
    .deps(#{ hadron_syscall_macros: #{ "crate": "hadron-syscall-macros", proc_macro: true } });

userspace.add("hadron-syslib", "userspace/hadron-syslib")
    .deps(#{ hadron_syscall: "hadron-syscall-user" });

userspace.add("hadron-init", "userspace/init")
    .crate_type(BIN)
    .root("src/main.rs")
    .deps(#{ hadron_syslib: "hadron-syslib" });

// ===========================================================================
// Rules (custom artifact generation)
// ===========================================================================

rule("hbtf")
    .inputs(["hadron-boot-limine"])
    .output("build/backtrace.hbtf")
    .handler("hbtf");

rule("initrd")
    .inputs(["hadron-init"])
    .output("build/initrd.cpio")
    .handler("initrd");

// ===========================================================================
// Pipeline
// ===========================================================================

pipeline()
    .stage("sysroot", ["sysroot"])
    .barrier("sysroot-ready")
    .stage("host", ["host"])
    .barrier("host-ready")
    .stage("kernel", ["vendored", "kernel-libs", "kernel-drivers", "kernel-main"])
    .barrier("kernel-ready")
    .stage("userspace", ["userspace"])
    .barrier("userspace-ready")
    .rule("hbtf")
    .rule("initrd");

// ===========================================================================
// QEMU
// ===========================================================================

qemu("q35", 256)
    .extra_args(["-serial", "stdio"])
    .test_success_code(33)
    .test_timeout(30)
    .test_extra_args([
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-display", "none",
        "-no-reboot"
    ]);

// ===========================================================================
// Bootloader
// ===========================================================================

bootloader("limine")
    .config_file("limine.conf");

// ===========================================================================
// Image
// ===========================================================================

image()
    .extra_files(#{
        "boot/initrd.cpio": "build/initrd.cpio",
        "boot/backtrace.hbtf": "build/backtrace.hbtf"
    });

// ===========================================================================
// Tests
// ===========================================================================

tests()
    .host_testable([
        "hadron-dwarf",
        "hadron-elf",
        "noalloc",
        "hadron-core",
        "hadron-driver-api",
        "hadron-drivers"
    ])
    .kernel_tests_dir("kernel/hadron-kernel/tests")
    .kernel_tests_crate("hadron-kernel")
    .kernel_tests_linker_script("targets/x86_64-unknown-hadron.ld");
