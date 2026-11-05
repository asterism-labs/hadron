---
name: hadron-architecture
description: Use when exploring kernel architecture, boot flow, crate dependencies, custom target, or writing/running tests
---

# Hadron Architecture & Testing

## Boot Flow

```
Limine bootloader -> kernel/boot/limine (hadron-boot-limine)
    -> hadron_kernel::kernel_init(boot_info)
        -> GDT, IDT, PMM, VMM initialization
        -> kernel main loop
```

## Key Dependencies

- `hadron-kernel` depends on `bitflags`, `hadris-io`, `hadron-acpi`, `hadron-elf`, `hadron-syscall`, `planck-noalloc`
- `hadron-drivers` depends on `hadron-kernel`, `bitflags`, `hadris-cpio`, `hadris-fat`, `hadris-io`, `hadris-iso`
- Boot stub depends on `hadron-kernel`, `hadron-drivers`, `limine`, `planck-noalloc`
- Driver registration uses linker sections (`.hadron_pci_drivers`, `.hadron_platform_drivers`, `.hadron_block_fs`, etc.)
- All `crates/*/*` are standalone no_std libraries

## Custom Target

The kernel uses a custom target `x86_64-unknown-hadron` (not `x86_64-unknown-none`):
- Kernel code model, PIC relocation
- Soft-float (no SSE/AVX in kernel mode)
- Panic = abort, redzone disabled
- Uses `rust-lld` linker

## Testing

- `gluon test --host-only` — Run host unit tests for crates listed in `gluon.rhai` `tests().host_testable()`
- `gluon test --kernel-only` — Build kernel + run integration tests in QEMU
- `gluon test` — Run both host and kernel tests

Integration tests run in QEMU using `hadron-test` crate:
- Tests use `isa-debug-exit` device (iobase=0xf4) to signal pass/fail
- Exit code 33 = success (configured in `gluon.rhai` `qemu()` section)
- Timeout: 30 seconds per test
