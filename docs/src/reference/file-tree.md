# Target File Tree

This is the target file tree after all remaining phases are implemented. Files are annotated with the phase that introduces them. Files without a phase annotation already exist.

```
hadron/
├── Cargo.toml                                    # Workspace root
├── Cargo.lock
├── rust-toolchain.toml                           # Nightly + custom target
├── targets/
│   └── x86_64-unknown-hadron.json                # Custom target spec
│
├── crates/
│   ├── limine/                                   # Limine boot protocol bindings
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── request.rs
│   │       ├── response.rs
│   │       ├── file.rs
│   │       ├── firmware_type.rs
│   │       ├── framebuffer.rs
│   │       ├── memory_map.rs
│   │       ├── modules.rs
│   │       ├── mp.rs
│   │       └── paging.rs
│   │
│   ├── noalloc/                                  # Allocation-free data structures
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   │
│   ├── hadron-drivers/                           # Hardware drivers
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   │
│   ├── hadron-test/                              # Test framework (QEMU exit)
│   │   ├── Cargo.toml
│   │   └── src/lib.rs
│   │
│   ├── hadron-elf/                               # ELF64 parser
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs
│   │
│   └── uefi/                                     # UEFI bindings (future)
│       ├── Cargo.toml
│       └── src/lib.rs
│
├── kernel/
│   ├── hadron-core/                              # The Frame (unsafe core)
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── addr.rs                            # PhysAddr, VirtAddr newtypes
│   │       ├── cell.rs                            # Cell primitives
│   │       ├── log.rs                             # kprint!/kprintln! macros
│   │       ├── percpu.rs                          # Per-CPU data, CpuLocal<T>
│   │       ├── static_assert.rs
│   │       ├── task.rs                            # TaskId, task types
│   │       │
│   │       ├── arch/
│   │       │   └── x86_64/
│   │       │       ├── mod.rs
│   │       │       ├── io.rs                      # inb/outb port I/O
│   │       │       ├── serial.rs                  # UART 16550 driver
│   │       │       ├── registers/
│   │       │       │   └── model_specific.rs      # MSR wrappers
│   │       │       ├── syscall.rs                 # SYSCALL/SYSRET setup [Phase 7]
│   │       │       ├── userspace.rs               # jump_to_userspace [Phase 9]
│   │       │       └── smp.rs                     # AP bootstrap [Phase 12]
│   │       │
│   │       ├── mm/
│   │       │   ├── mod.rs
│   │       │   └── hhdm.rs                        # HHDM translation
│   │       │
│   │       ├── paging/
│   │       │   ├── mod.rs
│   │       │   └── ...                            # Page table types
│   │       │
│   │       ├── sync/
│   │       │   ├── mod.rs
│   │       │   ├── spinlock.rs                    # SpinLock<T>
│   │       │   ├── lazy.rs                        # LazyLock<T>
│   │       │   └── waitqueue.rs                   # WaitQueue
│   │       │
│   │       └── syscall/
│   │           ├── mod.rs                         # Syscall numbers, error codes
│   │           └── userptr.rs                     # UserPtr<T>, UserSlice [Phase 7]
│   │
│   ├── boot/
│   │   └── limine/
│   │       ├── Cargo.toml
│   │       ├── build.rs
│   │       └── src/
│   │           └── main.rs                        # Limine entry point
│   │
│   └── hadron-kernel/                             # Safe Services
│       ├── Cargo.toml
│       └── src/
│           ├── lib.rs
│           ├── boot.rs                            # BootInfo, kernel_init
│           ├── log.rs                             # Logger with sinks
│           │
│           ├── arch/
│           │   ├── mod.rs
│           │   └── x86_64/
│           │       ├── gdt.rs                     # GDT + TSS
│           │       ├── idt.rs                     # IDT + exception handlers
│           │       └── interrupts/
│           │           └── handlers.rs            # IRQ handlers
│           │
│           ├── mm/
│           │   ├── pmm.rs                         # Bitmap frame allocator
│           │   ├── vmm.rs                         # Virtual memory manager
│           │   └── heap.rs                        # Kernel heap
│           │
│           ├── sched/
│           │   ├── mod.rs                         # Executor access, spawn APIs
│           │   ├── executor.rs                    # Priority async executor
│           │   └── waker.rs                       # Waker encoding
│           │
│           ├── drivers/
│           │   ├── early_fb.rs                    # Early framebuffer
│           │   ├── pci/                           # [Phase 10]
│           │   │   ├── mod.rs
│           │   │   ├── config.rs
│           │   │   └── device.rs
│           │   ├── block/                         # [Phase 10]
│           │   │   ├── mod.rs                     # AsyncBlockDevice trait
│           │   │   └── ramdisk.rs
│           │   └── virtio/                        # [Phase 10]
│           │       ├── mod.rs                     # VirtIO transport
│           │       ├── block.rs                   # VirtIO-blk
│           │       └── net.rs                     # VirtIO-net [Phase 14]
│           │
│           ├── syscall/
│           │   ├── mod.rs                         # Dispatch table [Phase 7]
│           │   ├── io.rs                          # read/write/open/close [Phase 7]
│           │   ├── process.rs                     # exit/getpid [Phase 7]
│           │   ├── memory.rs                      # mmap/munmap/brk [Phase 7]
│           │   └── time.rs                        # clock_gettime [Phase 7]
│           │
│           ├── fs/                                # [Phase 8]
│           │   ├── mod.rs                         # FileSystem, Inode traits
│           │   ├── vfs.rs                         # Mount table, path resolution
│           │   ├── file.rs                        # FileDescriptor table
│           │   ├── ramfs.rs                       # Heap-backed FS
│           │   ├── initramfs.rs                   # CPIO unpacker
│           │   ├── devfs.rs                       # /dev nodes
│           │   ├── procfs.rs                      # /proc
│           │   └── ext2/                          # [Phase 13]
│           │       ├── mod.rs
│           │       ├── superblock.rs
│           │       ├── block_group.rs
│           │       ├── inode.rs
│           │       └── dir.rs
│           │
│           ├── task/                              # [Phase 9]
│           │   └── process.rs                     # Process struct, exec()
│           │
│           ├── ipc/                               # [Phase 11]
│           │   ├── mod.rs                         # Async channels
│           │   └── pipe.rs                        # Pipe (byte-oriented channel)
│           │
│           ├── signal/                            # [Phase 11]
│           │   └── mod.rs                         # Minimal signal handling
│           │
│           ├── net/                               # [Phase 14]
│           │   ├── mod.rs                         # smoltcp integration
│           │   └── socket.rs                      # Socket syscall wrappers
│           │
│           ├── vdso/                              # [Phase 15]
│           │   ├── mod.rs                         # vDSO generation, mapping
│           │   └── data.rs                        # VVAR page
│           │
│           └── sync.rs                            # KernelServices sync
│
├── xtask/                                         # Build automation
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs
│       ├── build.rs
│       ├── config.rs
│       ├── iso.rs
│       ├── limine.rs
│       ├── run.rs
│       └── test.rs
│
└── docs/                                          # This book
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
| `task/context.rs` (CpuContext) | Not needed (no context switch) |
| `task/stack.rs` (KernelStack) | Not needed (no per-task stacks) |
| `arch/x86_64/context.rs` (switch_context) | Not needed |
| `task/fork.rs` (fork + CoW) | Not planned (use sys_spawn) |
| `net/ethernet.rs`, `arp.rs`, `tcp.rs`, etc. | Replaced by `smoltcp` crate |
| `syscall/userptr.rs` not planned | Added for pointer validation |
