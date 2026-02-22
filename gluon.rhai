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

profile("lock-stress")
    .inherits("stress")
    .config(#{
        lock_stress: true,
        LOCK_STRESS_MAX_US: 10,
        lockdep: true,
        lock_debug: true,
        lock_stat: true,
    });

profile("debug-gdb")
    .inherits("default")
    .qemu_memory(512)
    .qemu_extra_args(["-s", "-S"])
    .test_timeout(3600);

profile("debug-sanitizers")
    .inherits("default")
    .config(#{
        debug_heap_poison: true,
        debug_pmm_poison: true,
        debug_alloc_track: true,
        trace_mm: true,
    });

profile("profile")
    .inherits("default")
    .config(#{
        profile_sample: true,
        PROFILE_SAMPLE_RATE: 100,
        PROFILE_SAMPLE_DEPTH: 16,
        PROFILE_SAMPLE_BUFFER: 1024,
        PROFILE_DURATION_SECS: 10,
    });

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

// Host proc-macro ecosystem — syn/quote/proc-macro2 are direct deps of project
// proc-macros. Transitive deps (unicode-ident) are auto-resolved.
dependency("syn").version("2.0.116")
    .features(["full", "extra-traits", "parsing", "printing", "clone-impls", "proc-macro", "derive"]);
dependency("proc-macro2").version("1.0.106")
    .cfg_flags(["wrap_proc_macro"]);
dependency("quote").version("1.0.35");

// Kernel no_std dependencies — bitflags is a direct dep of project crates.
// Transitive deps (cfg-if, bytemuck, bytemuck_derive) are auto-resolved.
dependency("bitflags").version("2.11.0");
dependency("planck-noalloc").version("0.1.1").no_default_features();

// Hadris ecosystem — direct deps of hadron-drivers. Transitive deps
// (hadris-common, hadris-part) are auto-resolved from their Cargo.toml.
dependency("hadris-cpio").version("1.0.1")
    .features(["sync", "read", "alloc"]).no_default_features();
dependency("hadris-io").version("1.0.1")
    .features(["sync"]).no_default_features();
dependency("hadris-fat").version("1.0.1")
    .features(["sync", "read", "write", "alloc", "lfn"]).no_default_features();
dependency("hadris-iso").version("1.0.1")
    .features(["sync", "read", "alloc"]).no_default_features();

// ===========================================================================
// Host crates (project proc-macros, built for host triple)
// ===========================================================================

let host = group("host").target("host").edition("2021").project(false);

host.add("hadron-syscall-macros", "crates/syscall/hadron-syscall-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-driver-macros", "crates/driver/hadron-driver-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-binparse-macros", "crates/parse/binparse-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-mmio-macros", "crates/driver/hadron-mmio-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

// ===========================================================================
// Project kernel library crates
// ===========================================================================

let kernel_libs = group("kernel-libs").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_libs.add("hadron-core", "crates/core/hadron-core")
    .deps(#{ planck_noalloc: "planck-noalloc" });
kernel_libs.add("hadron-linkset", "crates/core/linkset");
kernel_libs.add("hadron-binparse", "crates/parse/binparse")
    .deps(#{ hadron_binparse_macros: #{ "crate": "hadron-binparse-macros", proc_macro: true } });
kernel_libs.add("hadron-elf", "crates/parse/elf");
kernel_libs.add("hadron-dwarf", "crates/parse/dwarf");
kernel_libs.add("hadron-mmio", "crates/driver/hadron-mmio")
    .deps(#{ hadron_mmio_macros: #{ "crate": "hadron-mmio-macros", proc_macro: true } });

kernel_libs.add("hadron-acpi", "crates/parse/acpi")
    .features(["alloc"])
    .deps(#{ hadron_binparse: "hadron-binparse" });

kernel_libs.add("limine", "crates/boot/limine")
    .deps(#{ bitflags: "bitflags" });

kernel_libs.add("uefi", "crates/boot/uefi")
    .deps(#{ bitflags: "bitflags" });

kernel_libs.add("hadron-syscall", "crates/syscall/hadron-syscall")
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
        hadron_mmio: "hadron-mmio",
        hadron_driver_macros: #{ "crate": "hadron-driver-macros", proc_macro: true }
    });

// ===========================================================================
// Test dependency crates (compiled on-demand during tests, NOT in pipeline)
// ===========================================================================

let test_deps = group("test-deps").target("x86_64-unknown-hadron").edition("2024").project(false);

test_deps.add("hadron-test", "crates/test/hadron-test")
    .features(["limine"])
    .deps(#{ limine: "limine" });

// ===========================================================================
// Benchmark dependency crates (compiled on-demand during benchmarks)
// ===========================================================================

let bench_deps = group("bench-deps").target("x86_64-unknown-hadron").edition("2024").project(false);

bench_deps.add("hadron-bench", "crates/test/hadron-bench")
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
        hadron_core: "hadron-core",
        hadron_elf: "hadron-elf",
        hadron_linkset: "hadron-linkset",
        hadron_syscall: "hadron-syscall",
        planck_noalloc: "planck-noalloc"
    })
    .dev_deps(#{
        hadron_bench: "hadron-bench",
        hadron_test: "hadron-test",
        limine: "limine",
        planck_noalloc: "planck-noalloc"
    });

kernel_main.add("hadron-boot-limine", "kernel/boot/limine")
    .crate_type(BIN)
    .root("src/main.rs")
    .linker_script("targets/x86_64-unknown-hadron.ld")
    .deps(#{
        hadron_drivers: "hadron-drivers",
        hadron_kernel: "hadron-kernel",
        limine: "limine",
        planck_noalloc: "planck-noalloc"
    });

// ===========================================================================
// Userspace crates
// ===========================================================================

let userspace = group("userspace").target("x86_64-unknown-hadron-user").edition("2024");

userspace.add("hadron-syscall-user", "crates/syscall/hadron-syscall")
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

benchmarks()
    .benches_dir("kernel/hadron-kernel/benches")
    .benches_crate("hadron-kernel")
    .benches_linker_script("targets/x86_64-unknown-hadron.ld");

tests()
    .host_testable([
        "hadron-acpi",
        "hadron-binparse",
        "hadron-core",
        "hadron-dwarf",
        "hadron-elf",
        "hadron-drivers",
        "hadron-kernel"
    ])
    .kernel_tests_dir("kernel/hadron-kernel/tests")
    .kernel_tests_crate("hadron-kernel")
    .kernel_tests_linker_script("targets/x86_64-unknown-hadron.ld");
