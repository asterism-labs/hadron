# Target File Tree

This is the target file tree after all remaining phases are implemented. Files are annotated with the phase that introduces them. Files without a phase annotation already exist.

```
hadron/
├── gluon.rhai                                    # Build configuration (Rhai scripting)
├── justfile                                      # Primary build interface
├── limine.conf                                   # Limine bootloader config
│
├── targets/
│   ├── x86_64-unknown-hadron.json                # Custom target spec
│   └── x86_64-unknown-hadron.ld                  # Kernel linker script
│
├── crates/
│   ├── parse/
│   │   ├── acpi/                                 # ACPI table parsing (hadron-acpi)
│   │   ├── binparse/                             # Binary format parser (hadron-binparse)
│   │   ├── binparse-macros/                       # Companion proc-macro
│   │   ├── dwarf/                                # DWARF debug info (hadron-dwarf)
│   │   └── elf/                                  # ELF64 parser (hadron-elf)
│   ├── boot/
│   │   ├── limine/                               # Limine boot protocol bindings
│   │   └── uefi/                                 # UEFI bindings
│   ├── core/
│   │   ├── hadron-core/                          # Core kernel abstractions
│   │   └── linkset/                              # Linker-section set collections
│   ├── driver/
│   │   ├── hadron-driver-macros/                 # #[hadron_driver] proc-macro
│   │   ├── hadron-mmio/                          # MMIO register abstraction
│   │   └── hadron-mmio-macros/                   # Companion proc-macro
│   ├── syscall/
│   │   ├── hadron-syscall/                       # Syscall numbers and ABI definitions
│   │   └── hadron-syscall-macros/                # Companion proc-macro
│   ├── test/
│   │   ├── hadron-test/                          # Test framework (QEMU exit)
│   │   └── hadron-bench/                         # Benchmark framework
│   └── tools/
│       ├── hadron-codegen/                       # Code generation utilities
│       └── hadron-perf/                          # Performance analysis tools
│
├── kernel/
│   ├── hadron-kernel/                            # Monolithic kernel crate
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── addr.rs                            # PhysAddr, VirtAddr, PhysFrame
│   │       ├── boot.rs                            # BootInfo trait, kernel_init
│   │       ├── paging.rs                          # Paging abstractions
│   │       ├── percpu.rs                          # Per-CPU data, CpuLocal<T>
│   │       ├── task.rs                            # TaskId, task types
│   │       │
│   │       ├── arch/x86_64/
│   │       │   ├── mod.rs
│   │       │   ├── acpi.rs                        # ACPI table handling
│   │       │   ├── gdt.rs                         # GDT + TSS
│   │       │   ├── idt.rs                         # IDT + exception handlers
│   │       │   ├── smp.rs                         # SMP bootstrap
│   │       │   ├── syscall.rs                     # SYSCALL/SYSRET setup
│   │       │   ├── userspace.rs                   # Userspace entry/exit
│   │       │   ├── instructions/                  # Safe CPU instruction wrappers
│   │       │   ├── registers/                     # Control registers, MSRs
│   │       │   └── interrupts/                    # IRQ handlers, APIC
│   │       │
│   │       ├── mm/
│   │       │   ├── pmm.rs                         # Bitmap frame allocator
│   │       │   ├── vmm.rs                         # Virtual memory manager
│   │       │   ├── heap.rs                        # Kernel heap
│   │       │   ├── hhdm.rs                        # Higher Half Direct Map
│   │       │   ├── address_space.rs               # Per-process address spaces
│   │       │   ├── region.rs                      # Memory regions
│   │       │   ├── zone.rs                        # Memory zones
│   │       │   ├── layout.rs                      # Kernel memory layout
│   │       │   └── mapper.rs                      # Page table mapper
│   │       │
│   │       ├── sync/
│   │       │   ├── spinlock.rs                    # SpinLock, IrqSpinLock
│   │       │   ├── mutex.rs                       # Async-friendly Mutex
│   │       │   ├── rwlock.rs                      # RwLock
│   │       │   ├── waitqueue.rs                   # WaitQueue, HeapWaitQueue
│   │       │   └── lazy.rs                        # Lazy initialization
│   │       │
│   │       ├── sched/
│   │       │   ├── executor.rs                    # Priority async executor
│   │       │   ├── waker.rs                       # Waker encoding
│   │       │   ├── timer.rs                       # Timer integration
│   │       │   ├── smp.rs                         # SMP work stealing [Phase 12]
│   │       │   └── block_on.rs                    # Blocking executor bridge
│   │       │
│   │       ├── fs/
│   │       │   ├── vfs.rs                         # Mount table, path resolution
│   │       │   ├── file.rs                        # File descriptors
│   │       │   ├── devfs.rs                       # /dev nodes
│   │       │   ├── console_input.rs               # Console input device
│   │       │   ├── block_adapter.rs               # Block device to FS bridge
│   │       │   └── path.rs                        # Path utilities
│   │       │
│   │       ├── proc/
│   │       │   ├── mod.rs                         # Process struct
│   │       │   ├── binfmt.rs                      # ELF loader
│   │       │   └── exec.rs                        # Process execution
│   │       │
│   │       ├── syscall/
│   │       │   ├── mod.rs                         # Dispatch table
│   │       │   ├── io.rs                          # read/write/open/close
│   │       │   ├── process.rs                     # exit/getpid/spawn
│   │       │   ├── memory.rs                      # mmap/munmap/brk
│   │       │   ├── time.rs                        # clock_gettime
│   │       │   ├── vfs.rs                         # VFS syscalls
│   │       │   └── query.rs                       # System query syscalls
│   │       │
│   │       ├── ipc/
│   │       │   └── pipe.rs                        # Pipe implementation
│   │       │
│   │       ├── driver_api/                        # Driver trait hierarchy
│   │       │   ├── mod.rs                         # Core driver traits
│   │       │   └── ...                            # Category and interface traits
│   │       │
│   │       ├── drivers/
│   │       │   └── device_registry.rs             # Device discovery and registry
│   │       │
│   │       ├── pci/
│   │       │   ├── cam.rs                         # PCI configuration access
│   │       │   ├── caps.rs                        # PCI capabilities
│   │       │   └── enumerate.rs                   # PCI bus enumeration
│   │       │
│   │       └── vdso/                              # [Phase 15]
│   │           ├── mod.rs                         # vDSO generation, mapping
│   │           └── data.rs                        # VVAR page
│   │
│   ├── hadron-drivers/                            # Pluggable hardware drivers
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── ahci/                              # AHCI (SATA) driver
│   │       ├── virtio/                            # VirtIO (block, PCI transport)
│   │       ├── serial/                            # UART16550, async serial
│   │       ├── input/                             # i8042, keyboard, mouse
│   │       ├── display/                           # Bochs VGA
│   │       ├── block/                             # Ramdisk
│   │       └── fs/                                # FAT, ISO9660, ramfs, initramfs
│   │
│   └── boot/
│       └── limine/
│           └── src/main.rs                        # Limine entry point
│
├── userspace/
│   ├── init/                                      # Init process (lepton-init)
│   ├── lepton-syslib/                             # Userspace syscall library
│   ├── shell/                                     # Interactive shell (lsh)
│   └── coreutils/                                 # Core utilities (lepton-coreutils)
│
├── vendor/                                        # Vendored external dependencies
│
├── tools/
│   └── gluon/                                     # Custom build system
│
└── docs/                                          # This mdbook
    ├── book.toml
    └── src/
        ├── SUMMARY.md
        └── ...
```

## Key Differences from Original Plan

The async executor model changes the file tree in several ways:

| Original Plan | Actual / Current Plan |
|--------------|----------------------|
| `sched/round_robin.rs` | `sched/executor.rs` + `sched/waker.rs` |
| `task/context.rs` (CpuContext) | Not needed (no context switch between kernel tasks) |
| `task/stack.rs` (KernelStack) | Not needed (no per-task stacks) |
| `arch/x86_64/context.rs` (switch_context) | Not needed |
| `task/fork.rs` (fork + CoW) | Not planned (use sys_spawn) |
| `net/ethernet.rs`, `arp.rs`, `tcp.rs`, etc. | Replaced by `smoltcp` crate |
| Separate `hadron-core` crate | Merged into `hadron-kernel` (single monolithic crate) |
