// Hadron kernel build configuration.
//
// This script is evaluated by gluon to produce the build model.
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
// Kconfig-style options (loaded from distributed Kconfig files)
// ===========================================================================

kconfig("Kconfig");

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
// External dependencies (resolved + vendored by `gluon vendor`)
// ===========================================================================

// Host proc-macro ecosystem (syn pulls in proc-macro2, quote, unicode-ident).
dependency("syn").version("2.0.116")
    .features(["full", "extra-traits", "parsing", "printing", "clone-impls", "proc-macro", "derive"]);
dependency("proc-macro2").version("1.0.106")
    .cfg_flags(["wrap_proc_macro"]);

// Kernel no_std dependencies.
dependency("bitflags").version("2.11.0");
dependency("cfg-if").version("1.0.4");
dependency("bytemuck").version("1.25.0").no_default_features();

// Hadris ecosystem crates (from crates.io).
dependency("hadris-cpio").version("1.0.1")
    .features(["sync", "read", "alloc"]).no_default_features();
dependency("hadris-io").version("1.0.1")
    .features(["sync"]).no_default_features();
dependency("hadris-common").version("1.0.1")
    .no_default_features();
dependency("hadris-fat").version("1.0.1")
    .features(["sync", "read", "write", "alloc", "lfn"]).no_default_features();
dependency("hadris-iso").version("1.0.1")
    .features(["sync", "read", "alloc"]).no_default_features();
dependency("hadris-part").version("1.0.1")
    .no_default_features();

// ===========================================================================
// Host crates (project proc-macros, built for host triple)
// ===========================================================================

let host = group("host").target("host").edition("2021").project(false);

host.add("hadron-syscall-macros", "crates/hadron-syscall-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-driver-macros", "crates/hadron-driver-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("bytemuck_derive", "vendor/bytemuck_derive")
    .edition("2018")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-binparse-macros", "crates/hadron-binparse-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

// ===========================================================================
// Project kernel library crates
// ===========================================================================

let kernel_libs = group("kernel-libs").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_libs.add("noalloc", "crates/noalloc");
kernel_libs.add("hadron-linkset", "crates/linkset");
kernel_libs.add("hadron-binparse", "crates/binparse")
    .deps(#{ hadron_binparse_macros: #{ "crate": "hadron-binparse-macros", proc_macro: true } });
kernel_libs.add("hadron-elf", "crates/elf");
kernel_libs.add("hadron-dwarf", "crates/dwarf");
kernel_libs.add("hadron-acpi", "crates/acpi")
    .deps(#{ hadron_binparse: "hadron-binparse" });

kernel_libs.add("limine", "crates/limine")
    .deps(#{ bitflags: "bitflags" });

kernel_libs.add("uefi", "crates/uefi")
    .deps(#{ bitflags: "bitflags" });

kernel_libs.add("hadron-syscall", "crates/hadron-syscall")
    .features(["kernel"])
    .deps(#{ hadron_syscall_macros: #{ "crate": "hadron-syscall-macros", proc_macro: true } });


// ===========================================================================
// Project kernel driver crates
// ===========================================================================

let kernel_drivers = group("kernel-drivers").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_drivers.add("hadron-drivers", "kernel/hadron-drivers")
    .deps(#{
        bitflags: "bitflags",
        hadris_cpio: "hadris-cpio",
        hadris_fat: "hadris-fat",
        hadris_io: "hadris-io",
        hadris_iso: "hadris-iso",
        hadron_kernel: "hadron-kernel",
        hadron_linkset: "hadron-linkset",
        hadron_driver_macros: #{ "crate": "hadron-driver-macros", proc_macro: true }
    });

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
        hadris_io: "hadris-io",
        hadron_acpi: "hadron-acpi",
        hadron_elf: "hadron-elf",
        hadron_linkset: "hadron-linkset",
        hadron_syscall: "hadron-syscall",
        noalloc: "noalloc"
    })
    .dev_deps(#{
        hadron_test: "hadron-test",
        limine: "limine",
        noalloc: "noalloc"
    });

kernel_main.add("hadron-boot-limine", "kernel/boot/limine")
    .crate_type(BIN)
    .root("src/main.rs")
    .linker_script("targets/x86_64-unknown-hadron.ld")
    .deps(#{
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

userspace.add("lepton-syslib", "userspace/lepton-syslib")
    .deps(#{ hadron_syscall: "hadron-syscall-user" });

userspace.add("lepton-init", "userspace/init")
    .crate_type(BIN)
    .root("src/main.rs")
    .deps(#{ lepton_syslib: "lepton-syslib" });

userspace.add("lepton-shell", "userspace/shell")
    .crate_type(BIN).root("src/main.rs")
    .deps(#{ lepton_syslib: "lepton-syslib" });

userspace.add("lepton-coreutils", "userspace/coreutils")
    .crate_type(BIN).root("src/main.rs")
    .deps(#{ lepton_syslib: "lepton-syslib" });

// ===========================================================================
// Rules (custom artifact generation)
// ===========================================================================

rule("hkif")
    .inputs(["hadron-boot-limine"])
    .output("build/hkif.bin")
    .handler("hkif");

rule("initrd")
    .inputs(["lepton-init", "lepton-shell", "lepton-coreutils"])
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
    .rule("hkif")
    .stage("userspace", ["userspace"])
    .barrier("userspace-ready")
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
        "boot/initrd.cpio": "build/initrd.cpio"
    });

// ===========================================================================
// Tests
// ===========================================================================

tests()
    .host_testable([
        "hadron-binparse",
        "hadron-dwarf",
        "hadron-elf",
        "noalloc",
        "hadron-drivers",
        "hadron-kernel"
    ])
    .kernel_tests_dir("kernel/hadron-kernel/tests")
    .kernel_tests_crate("hadron-kernel")
    .kernel_tests_linker_script("targets/x86_64-unknown-hadron.ld");
