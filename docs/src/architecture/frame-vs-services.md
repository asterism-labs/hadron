# Frame vs Services

This chapter explains the dividing line between the unsafe **frame** (`hadron-core`) and the safe **services** (`hadron-kernel`), with concrete examples of what goes where and why.

## The Rule

> If it touches hardware, raw pointers, or inline assembly, it belongs in the frame.
> Everything else is a safe service.

The frame's job is to wrap every piece of `unsafe` code in a safe API. Once the frame provides safe abstractions, services never need to reach for `unsafe`.

## Classification Table

| Component | Layer | Why |
|-----------|-------|-----|
| I/O port access (`inb`/`outb`) | Frame | Raw hardware I/O, inline assembly |
| Serial UART driver (init, write byte) | Frame | Uses I/O port access |
| GDT / TSS / IDT setup | Frame | Writes to CPU control registers |
| Exception handlers (register dumps) | Frame | Reads/writes CPU state |
| Page table manipulation | Frame | Writes raw page table entries, CR3 |
| Physical frame allocator | Frame | Manages raw physical memory |
| Kernel heap (`GlobalAlloc`) | Frame | Implements unsafe `GlobalAlloc` trait |
| Context switch | Frame | Naked assembly function, saves/restores registers |
| APIC / I/O APIC | Frame | MMIO to hardware registers |
| SYSCALL/SYSRET entry stub | Frame | Assembly, `swapgs`, MSR programming |
| SpinLock | Frame | Atomic operations, interrupt disable |
| Logging macros (`kprintln!`) | Frame | Wraps SpinLock + serial driver |
| Framebuffer console rendering | **Service** | Operates on safe `&mut [u8]` slice via HHDM |
| Round-robin scheduler | **Service** | Uses safe `Task` and `SpinLock` APIs |
| VFS / filesystem layer | **Service** | Pure data structure management |
| Syscall dispatch table | **Service** | Match on syscall number, call handlers |
| Process management (fork, exec) | **Service** | Uses safe address space / page table APIs |
| PCI enumeration | **Service** | Uses safe I/O port wrappers |
| VirtIO drivers | **Service** | Uses safe MMIO abstractions |
| TCP/IP stack | **Service** | Pure protocol implementation |
| Signal delivery | **Service** | Modifies task state through safe APIs |
| ext2 filesystem | **Service** | Reads/writes blocks through safe `BlockDevice` trait |

## Example: Serial Port

The serial port illustrates the frame/service split perfectly.

### Frame Layer (`hadron-core`)

```rust
// arch/x86_64/io.rs — raw I/O port access (unsafe)
#[inline]
pub unsafe fn outb(port: u16, value: u8) {
    core::arch::asm!("out dx, al", in("dx") port, in("al") value);
}

#[inline]
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    core::arch::asm!("in al, dx", in("dx") port, out("al") value);
    value
}

// arch/x86_64/serial.rs — UART driver (uses unsafe internally, safe API)
pub struct SerialPort {
    base: u16,
}

impl SerialPort {
    /// Initialize the serial port. Unsafe because it programs hardware.
    pub(crate) unsafe fn init(base: u16) -> Self { /* ... */ }

    /// Write a byte. Safe because init guarantees the port is valid.
    pub fn write_byte(&self, byte: u8) {
        // Wait for transmit buffer empty, then write
        unsafe { outb(self.base, byte); }
    }
}

impl core::fmt::Write for SerialPort {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        for byte in s.bytes() {
            self.write_byte(byte);
        }
        Ok(())
    }
}
```

### Service Layer (`hadron-kernel`)

```rust
// console/serial.rs — safe serial console (no unsafe!)
use hadron_core::log::writer;

pub fn init() {
    // Serial port already initialized by frame during boot
    kprintln!("Serial console ready");
}

pub fn print(args: core::fmt::Arguments) {
    use core::fmt::Write;
    writer().write_fmt(args).unwrap();
}
```

## Example: Page Tables

### Frame Layer

```rust
// arch/x86_64/paging/mapper.rs
pub struct PageTableMapper { /* ... */ }

impl PageTableMapper {
    /// Map a virtual page to a physical frame.
    /// Safe API — all unsafe is internal.
    pub fn map(
        &mut self,
        page: VirtAddr,
        frame: PhysFrame,
        flags: PageTableFlags,
        allocator: &mut impl FrameAllocator,
    ) -> Result<(), MapError> {
        // Internally walks page tables using unsafe pointer arithmetic
        // via HHDM, but the API is safe
    }
}
```

### Service Layer

```rust
// task/process.rs — process creation (no unsafe!)
pub fn exec(elf_data: &[u8]) -> Result<Process, ExecError> {
    let mut address_space = AddressSpace::new_user();

    for segment in elf::parse_load_segments(elf_data) {
        let frame = frame_allocator.alloc()?;
        address_space.mapper().map(
            segment.vaddr,
            frame,
            PageTableFlags::USER | PageTableFlags::WRITABLE,
            &mut frame_allocator,
        )?;
        // Copy segment data...
    }

    Ok(Process::new(address_space, entry_point))
}
```

## Example: Scheduler

### Frame Layer

```rust
// arch/x86_64/context.rs
/// Switch CPU context from `old` task to `new` task.
/// This is a naked assembly function — pure unsafe frame code.
#[naked]
pub unsafe extern "C" fn switch_context(
    old: *mut CpuContext,
    new: *const CpuContext,
) {
    // Save callee-saved registers to old context
    // Load callee-saved registers from new context
    // ret (returns to new task's saved RIP)
}
```

### Service Layer

```rust
// sched/round_robin.rs — entirely safe!
pub struct RoundRobinScheduler {
    ready_queue: VecDeque<Task>,
    current: Option<Task>,
}

impl RoundRobinScheduler {
    pub fn schedule(&mut self) {
        if let Some(next) = self.ready_queue.pop_front() {
            let old = self.current.replace(next);
            if let Some(old_task) = old {
                self.ready_queue.push_back(old_task);
                // switch_context is exposed through a safe wrapper
                // that takes &mut Task references
                Task::switch(&old_task, &self.current.as_ref().unwrap());
            }
        }
    }
}
```

## Unsafe Budget

The framekernel approach aims for roughly **84% safe code**. The `unsafe` percentage varies by phase:

| Component | Approximate `unsafe` % |
|-----------|----------------------|
| Boot stubs | ~15% |
| CPU init (GDT/IDT) | ~25% |
| Memory management | ~20-25% |
| APIC / timers | ~30% |
| Context switch | ~40% |
| Scheduler | ~10% |
| VFS / filesystems | ~5% |
| Syscall dispatch | ~5% |
| Drivers | ~10% |
| Networking | ~5% |

The frame stays small and auditable while services can grow without increasing the `unsafe` surface area.
