# Phase 12: SMP & Per-CPU Executors

Previously Phase 14, moved earlier because the executor already assumes per-CPU operation. Getting SMP online at this stage catches concurrency bugs in all subsequent phases.

## Goal

Boot Application Processors (APs), give each CPU its own executor instance with a local run queue, implement work stealing between CPUs, and route cross-CPU wakeups via IPI. After this phase, kernel async tasks run in parallel across all available cores.

## Why Moved Earlier

The Phase 6 executor already provides `CpuLocal<T>` and a per-CPU design with `MAX_CPUS` set to 1. SMP does not change the executor architecture -- it scales it horizontally. Bringing SMP online before filesystems and networking means those subsystems are developed and tested under true concurrency from the start, rather than retrofitting SMP correctness later.

## Files to Create/Modify

| File | Description |
|------|-------------|
| `hadron-kernel/src/arch/x86_64/smp.rs` | AP bootstrap via Limine MP protocol |
| `hadron-kernel/src/percpu.rs` | `PerCpu` struct, `PerCpu::current()` via GS base |
| `hadron-kernel/src/sched/smp.rs` | Work stealing, cross-CPU wakeup routing |
| `hadron-kernel/src/sched/executor.rs` | Replace `LazyLock<Executor>` with `CpuLocal<Executor>` |
| `hadron-kernel/src/arch/x86_64/gdt.rs` | Per-CPU GDT/TSS initialization for APs |
| `hadron-kernel/src/boot.rs` | Increase `MAX_CPUS` to detected CPU count |

## Key Design

### AP Bootstrap via Limine MP Protocol

Detection and initialization of APs follows this sequence:

1. Read the Limine MP response to discover all CPUs.
2. For each non-BSP CPU: allocate a `PerCpu` struct and a kernel stack.
3. Write the `PerCpu` pointer into the AP's `extra_argument` field.
4. Write `ap_entry` into the AP's `goto_address` to start execution.
5. The AP entry function sets GS base, initializes GDT/IDT/APIC, and enters the executor loop.

```rust
/// Boot all Application Processors using Limine's MP response.
pub fn boot_aps(mp_response: &MpResponse) {
    let cpu_count = mp_response.cpu_count();
    log::info!("SMP: {} CPUs detected", cpu_count);

    for i in 0..cpu_count {
        let cpu = mp_response.cpu(i);
        if cpu.lapic_id == bsp_lapic_id() {
            continue; // Skip BSP
        }

        // Allocate per-CPU data and kernel stack
        let percpu = allocate_percpu(cpu.lapic_id, i);
        let stack = KernelStack::allocate(/* ... */).unwrap();

        // Pass PerCpu pointer to the AP
        cpu.extra_argument = percpu as *const PerCpu as u64;
        cpu.goto_address.write(ap_entry as u64);
    }
}

/// Entry point for Application Processors.
extern "C" fn ap_entry(cpu_info: &limine::Cpu) -> ! {
    let percpu = cpu_info.extra_argument as *mut PerCpu;

    // Set GS base to per-CPU data
    unsafe { wrmsr(MSR_GS_BASE, percpu as u64); }

    // Initialize this CPU's hardware state
    gdt::init_ap(&mut (*percpu).tss);
    idt::load();
    apic::init_ap();

    log::info!("SMP: CPU {} online (APIC ID {})",
        (*percpu).cpu_index, (*percpu).apic_id);

    // Enter this CPU's executor loop
    executor().run();
}
```

### PerCpu Struct

```rust
/// Per-CPU data, accessed via the GS segment base register.
/// The first field must be a self-pointer so that `mov gs:0` yields
/// a pointer to the entire struct.
#[repr(C)]
pub struct PerCpu {
    /// Self-pointer: PerCpu::current() reads gs:0 to obtain this.
    pub self_ptr: *const PerCpu,
    /// APIC ID of this CPU.
    pub apic_id: u32,
    /// Logical CPU index (0 = BSP, 1.. = APs).
    pub cpu_index: u32,
    /// Kernel RSP for syscall entry.
    pub kernel_rsp: u64,
    /// Saved user RSP on syscall entry.
    pub user_rsp: u64,
    /// This CPU's Task State Segment.
    pub tss: TaskStateSegment,
}

impl PerCpu {
    /// Get a reference to the current CPU's PerCpu data.
    /// Reads the self-pointer stored at GS:0.
    #[inline]
    pub fn current() -> &'static PerCpu {
        unsafe {
            let ptr: *const PerCpu;
            core::arch::asm!("mov {}, gs:0", out(reg) ptr);
            &*ptr
        }
    }
}
```

### Per-CPU Executor

The Phase 6 executor changes from a single global instance to one per CPU. The `BTreeMap` task storage is replaced with a per-CPU slab allocator for O(1) insert/remove and zero cross-CPU lock contention during polls. See [Preemption & Scaling](../design/preemption-and-scaling.md#slab-task-storage-replaces-btreemap-in-phase-12) for the slab design.

```rust
// Before (Phase 6):
static EXECUTOR: LazyLock<Executor> = LazyLock::new(Executor::new);

// After (Phase 12):
// Each CPU has its own Executor, accessed via CpuLocal<Executor>.
// BSP initializes its executor during boot.
// Each AP initializes its own executor in ap_entry().
```

Each CPU runs `executor().run()` as its idle loop. Tasks spawned locally go into the local queue. Tasks woken on a remote CPU are pushed to the target CPU's queue and an IPI is sent.

### Work Stealing

When a CPU's local run queue is empty, it attempts to steal tasks from other CPUs:

```rust
fn try_steal(&self) -> Option<Task> {
    for other_cpu in 0..cpu_count() {
        if other_cpu == PerCpu::current().cpu_index as usize {
            continue;
        }
        // try_lock avoids deadlock: never hold two queue locks at once.
        if let Some(mut queue) = run_queues[other_cpu].try_lock() {
            if queue.len() > 1 {
                // Steal from the back (LIFO end) to preserve cache locality
                // for the victim's hot tasks at the front.
                return queue.pop_back();
            }
        }
    }
    None
}
```

The `try_lock` call is critical: a CPU must never hold its own queue lock while attempting to lock another CPU's queue, as this creates ABBA deadlock potential. By using `try_lock` and accepting failure, the algorithm remains lock-free in the contention case.

Note: cross-CPU wakeups use a lock-free MPSC queue rather than contending on the ready queue lock. Each CPU drains its wake queue into the local ready queue at the start of each `poll_ready_tasks` iteration. See [Preemption & Scaling](../design/preemption-and-scaling.md#phase-12-per-cpu-executors) for the full architecture.

### Cross-CPU Wakeup

The waker encoding already reserves 6 bits (61-56) for CPU ID since Phase 6. When a waker targets a different CPU, it reads the target CPU index from the encoded data and pushes to a lock-free MPSC wake queue on the target CPU. See [Preemption & Scaling](../design/preemption-and-scaling.md#waker-encoding-forward-compatible-from-phase-6) for the encoding layout.

```rust
fn wake(data: *const ()) {
    let (task_id, priority, target_cpu) = decode_waker_data(data);

    // Push task to the target CPU's wake queue (lock-free MPSC).
    wake_queues[target_cpu].push(task_id, priority);

    // Send IPI to wake the target CPU if it is halted.
    if target_cpu != PerCpu::current().cpu_index as usize {
        apic::send_ipi(target_cpu, WAKEUP_VECTOR);
    }
}
```

### Per-CPU GDT/TSS

Each CPU requires its own GDT containing a TSS descriptor that points to a unique TSS. The TSS holds IST stack pointers for that CPU's interrupt handling:

```rust
/// Initialize GDT for an Application Processor.
/// Allocates a CPU-local GDT with a TSS descriptor pointing
/// to the PerCpu TSS.
pub fn init_ap(tss: &'static mut TaskStateSegment) {
    // Allocate IST stacks for this CPU
    tss.ist[0] = allocate_ist_stack();

    let gdt = GDT::new_with_tss(tss);
    gdt.load();
    // Load task register with TSS selector
    unsafe { ltr(gdt.tss_selector()); }
}
```

## Frame vs Service

| Component | Layer | Reason |
|-----------|-------|--------|
| AP bootstrap (`ap_entry`) | Frame | GDT/IDT init, MSR writes, inline assembly |
| GS base setup (`wrmsr`) | Frame | Direct MSR write for per-CPU addressing |
| `PerCpu::current()` | Frame | Inline assembly (`mov gs:0`) |
| Per-CPU data allocation | Frame | Raw memory management |
| IPI sending | Frame | APIC register writes |
| Per-CPU GDT/TSS init | Frame | Segment register and TSS manipulation |
| SMP scheduler logic | Service | Scheduling decisions using safe APIs |
| Work stealing | Service | Lock-based queue operations, no unsafe |
| Cross-CPU wakeup routing | Service | Waker logic using safe executor APIs |
| Load observation | Service | Pure computation over queue lengths |

## Milestone

**Verification**:
```
SMP: 4 CPUs detected
SMP: CPU 1 online (APIC ID 1)
SMP: CPU 2 online (APIC ID 2)
SMP: CPU 3 online (APIC ID 3)
[CPU 0] executor: polling task A
[CPU 1] executor: polling task B
[CPU 2] executor: polling task C
[CPU 1] executor: stole task D from CPU 0
```

Multiple CPUs online, each running its own executor loop. Tasks are distributed across cores. Work stealing is observable when one CPU's queue drains while others are loaded.

## Dependencies

- **Phase 5**: APIC (for IPI delivery and per-CPU timer configuration)
- **Phase 6**: Executor (per-CPU design scaled from 1 to N CPUs)
- **Phase 7**: Syscall interface (per-CPU kernel stack pointer via GS base)
