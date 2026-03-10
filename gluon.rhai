// Hadron kernel build configuration.

// ===========================================================================
// Project
// ===========================================================================

project("hadron", "0.1.0");

// ===========================================================================
// Targets
// ===========================================================================

target("x86_64-unknown-hadron", "targets/x86_64-unknown-hadron.json");
target("x86_64-unknown-hadron-user", "targets/x86_64-unknown-hadron-user.json");
target("x86_64-unknown-uefi", "targets/x86_64-unknown-uefi.json");

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
    .preset("debug")
    .qemu_cores(4)
    .boot_binary("hadron-boot-uefi");

profile("release")
    .target("x86_64-unknown-hadron")
    .opt_level(2)
    .lto("thin")
    .preset("release")
    .boot_binary("hadron-boot-uefi");

profile("debug-gdb")
    .inherits("default")
    .qemu_memory(512)
    .qemu_extra_args(["-s", "-S"])
    .test_timeout(3600);

// ===========================================================================
// Sysroot crates — kernel target (built first)
// ===========================================================================

let sysroot = group("sysroot").target("x86_64-unknown-hadron").edition("2024").project(false);

sysroot.add("core", "{sysroot}/core")
    .root("src/lib.rs");

sysroot.add("compiler_builtins", "{sysroot}/compiler-builtins/compiler-builtins")
    .root("src/lib.rs")
    .features(["compiler-builtins", "rustc-dep-of-std"])
    .deps(#{ core: "core" });

sysroot.add("alloc", "{sysroot}/alloc")
    .root("src/lib.rs")
    .deps(#{ core: "core", compiler_builtins: "compiler_builtins" });

// ===========================================================================
// Sysroot crates — UEFI target
// ===========================================================================

let sysroot_uefi = group("sysroot-uefi").target("x86_64-unknown-uefi").edition("2024").project(false);

sysroot_uefi.add("core-uefi", "{sysroot}/core")
    .root("src/lib.rs");

sysroot_uefi.add("compiler_builtins-uefi", "{sysroot}/compiler-builtins/compiler-builtins")
    .root("src/lib.rs")
    .features(["compiler-builtins", "rustc-dep-of-std"])
    .deps(#{ core: "core-uefi" });

sysroot_uefi.add("alloc-uefi", "{sysroot}/alloc")
    .root("src/lib.rs")
    .deps(#{ core: "core-uefi", compiler_builtins: "compiler_builtins-uefi" });

// ===========================================================================
// Sysroot crates — user target
// ===========================================================================

let sysroot_user = group("sysroot-user").target("x86_64-unknown-hadron-user").edition("2024").project(false);

sysroot_user.add("core-user", "{sysroot}/core")
    .root("src/lib.rs");

sysroot_user.add("compiler_builtins-user", "{sysroot}/compiler-builtins/compiler-builtins")
    .root("src/lib.rs")
    .features(["compiler-builtins", "rustc-dep-of-std"])
    .deps(#{ core: "core-user" });

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
dependency("bitflags").version("2.11.0");
dependency("planck-noalloc").version("0.1.1").no_default_features();

// Hadris ecosystem — hadris-io is used by hadron-kernel.
dependency("hadris-io").version("1.0.1")
    .features(["sync"]).no_default_features();

// ===========================================================================
// Host crates (project proc-macros, built for host triple)
// ===========================================================================

let host = group("host").target("host").edition("2021").project(false);

host.add("hadron-binparse-macros", "crates/parse/binparse-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-mmio-macros", "kernel/mmio-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

host.add("hadron-ktest-macros", "crates/test/hadron-ktest-macros")
    .edition("2024")
    .crate_type(PROC_MACRO)
    .deps(#{ syn: "syn", quote: "quote", proc_macro2: "proc-macro2" });

// ===========================================================================
// Project kernel library crates
// ===========================================================================

let kernel_libs = group("kernel-libs").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_libs.add("hadron-boot-info", "crates/boot/boot-info");
kernel_libs.add("hadron-core", "kernel/core")
    .deps(#{ bitflags: "bitflags", hadron_linkset: "hadron-linkset", planck_noalloc: "planck-noalloc" });
kernel_libs.add("hadron-linkset", "crates/core/linkset");
kernel_libs.add("hadron-log", "crates/core/hadron-log")
    .deps(#{ hadron_core: "hadron-core", hadron_linkset: "hadron-linkset" });
kernel_libs.add("hadron-intrinsics", "kernel/intrinsics")
    .rustc_flags(["-Ctarget-feature=+sse,+sse2,-soft-float"]);
kernel_libs.add("hadron-binparse", "crates/parse/binparse")
    .deps(#{ hadron_binparse_macros: #{ "crate": "hadron-binparse-macros", proc_macro: true } });
kernel_libs.add("hadron-elf", "crates/parse/elf");
kernel_libs.add("hadron-dwarf", "crates/parse/dwarf");
kernel_libs.add("hadron-mmio", "kernel/mmio")
    .deps(#{ hadron_mmio_macros: #{ "crate": "hadron-mmio-macros", proc_macro: true } });

kernel_libs.add("hadron-pci", "kernel/pci");

kernel_libs.add("hadron-mm", "kernel/mm")
    .deps(#{
        bitflags: "bitflags",
        hadron_boot_info: "hadron-boot-info",
        hadron_core: "hadron-core",
        planck_noalloc: "planck-noalloc"
    });

kernel_libs.add("hadron-sched", "kernel/sched")
    .deps(#{
        hadron_core: "hadron-core",
        planck_noalloc: "planck-noalloc"
    });

kernel_libs.add("hadron-objects", "kernel/objects")
    .deps(#{
        bitflags: "bitflags",
        hadron_core: "hadron-core"
    });

kernel_libs.add("hadron-acpi", "crates/parse/acpi")
    .features(["alloc"])
    .deps(#{ hadron_binparse: "hadron-binparse" });

kernel_libs.add("hadron-fdt", "crates/parse/fdt")
    .deps(#{ hadron_binparse: "hadron-binparse" });

kernel_libs.add("hadron-ktest", "crates/test/hadron-ktest")
    .deps(#{
        hadron_linkset: "hadron-linkset",
        hadron_ktest_macros: #{ "crate": "hadron-ktest-macros", proc_macro: true }
    });

// ===========================================================================
// Vendored crates for UEFI target (suffixed to avoid name collision)
// ===========================================================================

let vendored_uefi = group("vendored-uefi").target("x86_64-unknown-uefi").edition("2021").project(false);

vendored_uefi.add("bitflags-uefi", "vendor/bitflags");

// ===========================================================================
// UEFI library crates (built for UEFI target)
// ===========================================================================

let uefi_libs = group("uefi-libs").target("x86_64-unknown-uefi").edition("2024");

uefi_libs.add("hadron-boot-info-uefi", "crates/boot/boot-info");
uefi_libs.add("hadron-elf-uefi", "crates/parse/elf");
uefi_libs.add("uefi", "crates/boot/uefi")
    .deps(#{ bitflags: "bitflags-uefi" });

// ===========================================================================
// Kernel main (hadron-kernel)
// ===========================================================================

let kernel_main = group("kernel-main").target("x86_64-unknown-hadron").edition("2024").config(true);

kernel_main.add("hadron-kernel", "kernel/kernel")
    .deps(#{
        bitflags: "bitflags",
        hadron_acpi: "hadron-acpi",
        hadron_boot_info: "hadron-boot-info",
        hadron_core: "hadron-core",
        hadron_elf: "hadron-elf",
        hadron_intrinsics: "hadron-intrinsics",
        hadron_linkset: "hadron-linkset",
        hadron_log: "hadron-log",
        hadron_mm: "hadron-mm",
        hadron_mmio: "hadron-mmio",
        hadron_objects: "hadron-objects",
        hadron_pci: "hadron-pci",
        hadron_sched: "hadron-sched",
        planck_noalloc: "planck-noalloc"
    })
    .dev_deps(#{
        hadron_bench: "hadron-bench",
        hadron_test: "hadron-test",
        planck_noalloc: "planck-noalloc"
    });

kernel_main.add("hadron-kernel-image", "kernel/kernel-image")
    .crate_type(BIN)
    .root("src/main.rs")
    .linker_script("targets/x86_64-unknown-hadron.ld")
    .deps(#{
        hadron_kernel: "hadron-kernel"
    })
    .artifact_deps(["userboot"]);

// ===========================================================================
// Userboot (first userspace binary)
// ===========================================================================

let userboot = group("userboot").target("x86_64-unknown-hadron-user").edition("2024");

userboot.add("userboot", "userspace/userboot")
    .crate_type(BIN)
    .root("src/main.rs");

// ===========================================================================
// UEFI boot binary
// ===========================================================================

let uefi_boot = group("uefi-boot").target("x86_64-unknown-uefi").edition("2024");

uefi_boot.add("hadron-boot-uefi", "kernel/boot/uefi")
    .crate_type(BIN)
    .root("src/main.rs")
    .deps(#{
        uefi: "uefi",
        hadron_boot_info: "hadron-boot-info-uefi",
        hadron_elf: "hadron-elf-uefi"
    })
    .artifact_deps(["hadron-kernel-image"]);

// ===========================================================================
// Test dependency crates (compiled on-demand during tests, NOT in pipeline)
// ===========================================================================

let test_deps = group("test-deps").target("x86_64-unknown-hadron").edition("2024").project(false);

test_deps.add("hadron-test", "crates/test/hadron-test")
    .deps(#{});

// ===========================================================================
// Benchmark dependency crates (compiled on-demand during benchmarks)
// ===========================================================================

let bench_deps = group("bench-deps").target("x86_64-unknown-hadron").edition("2024").project(false);

bench_deps.add("hadron-bench", "crates/test/hadron-bench")
    .deps(#{});

// ===========================================================================
// Pipeline
// ===========================================================================

pipeline()
    .stage("sysroot", ["sysroot", "sysroot-uefi", "sysroot-user"])
    .barrier("sysroot-ready")
    .stage("host", ["host"])
    .barrier("host-ready")
    .stage("kernel", ["vendored", "vendored-uefi", "kernel-libs", "uefi-libs", "userboot"])
    .barrier("kernel-ready")
    .stage("link", ["kernel-main"])
    .barrier("link-ready")
    .stage("boot", ["uefi-boot"]);

// ===========================================================================
// QEMU
// ===========================================================================

qemu("q35", 256)
    .extra_args(["-serial", "stdio", "-no-reboot"])
    .test_success_code(33)
    .test_timeout(30)
    .test_extra_args([
        "-device", "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-display", "none"
    ]);

// ===========================================================================
// Tests
// ===========================================================================

benchmarks()
    .benches_dir("kernel/kernel/benches")
    .benches_crate("hadron-kernel")
    .benches_linker_script("targets/x86_64-unknown-hadron.ld");

tests()
    .host_testable([
        "hadron-acpi",
        "hadron-binparse",
        "hadron-core",
        "hadron-dwarf",
        "hadron-elf",
        "hadron-fdt",
        "hadron-log",
        "hadron-mm",
        "hadron-objects",
        "hadron-sched",
        "hadron-pci"
    ])
    .kani_verifiable(["hadron-core"])
    .shuttle_testable(["hadron-core"])
    .kernel_tests_dir("kernel/kernel/tests")
    .kernel_tests_crate("hadron-kernel")
    .kernel_tests_linker_script("targets/x86_64-unknown-hadron.ld");
