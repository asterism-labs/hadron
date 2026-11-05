//! Per-CPU data and the guards that grant access to it.
//!
//! Each CPU has one [`PerCpu`] block, and its GS base points at it. The
//! block's first field holds the block's own address, so [`current`]
//! finds it with one `gs:`-relative load.
//!
//! Only the CPU that owns a block uses it, so its fields need no locks or
//! atomic read-modify-writes. Most are `Cell`s, which is sound only while
//! nothing else on this CPU can run in between. An interrupt handler
//! could otherwise run halfway through an update. So [`current`] takes an
//! [`IrqGuard`], which keeps interrupts masked.
//!
//! The preempt count is the exception. [`PreemptGuard`] changes it with a
//! single `gs:`-relative `inc` or `dec`, which an interrupt cannot split,
//! so it needs no guard.
//!
//! The boot CPU's block is a static. Other CPUs will get theirs from the
//! heap when SMP starts.

use core::cell::Cell;
use core::marker::PhantomData;
use core::mem::offset_of;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::arch::x86_64::cpu;
use crate::sched::{self, Thread};
use crate::sync::{HeldLocks, Mask};

/// One CPU's data. It sits in its own cache lines, so CPUs do not slow
/// each other down by writing to neighboring blocks.
#[repr(C, align(64))]
pub(crate) struct PerCpu {
    /// The block's own address. It must stay the first field: [`current`]
    /// reads it at `gs:[0]`.
    this: *const PerCpu,
    /// The kernel's index for this CPU. The boot CPU is 0.
    id: u32,
    /// How many [`PreemptGuard`]s are alive on this CPU. The scheduler
    /// switches threads only when it is 0.
    preempt_depth: AtomicU32,
    /// Set when the running thread should give up the CPU as soon as the
    /// preempt count reaches 0.
    need_resched: AtomicBool,
    /// How many [`IrqGuard`]s are alive on this CPU.
    irq_depth: Cell<u32>,
    /// Whether interrupts were enabled when the outermost [`IrqGuard`] was
    /// made. The last guard to drop restores this.
    irq_enabled: Cell<bool>,
    /// Spin locks this CPU holds, for the run-time lock checker.
    pub(crate) held: HeldLocks,
    /// The running thread, from `Arc::into_raw`, or null before the
    /// scheduler starts. The scheduler owns this and the next two fields.
    pub(crate) current: Cell<*const Thread>,
    /// The thread that ran before the last switch. The thread switched to
    /// drops it, since a thread cannot free the stack it runs on.
    pub(crate) previous: Cell<*const Thread>,
    /// The thread that runs when no other is ready.
    pub(crate) idle: Cell<*const Thread>,
}

// SAFETY: a block is shared only in the sense that a static holds it. Its
// `Cell`s are used only through `current`, which returns the running CPU's
// own block while interrupts are masked, so no two contexts touch them at
// once. The preempt count and the reschedule flag are atomics.
unsafe impl Sync for PerCpu {}

impl PerCpu {
    const fn new(this: *const PerCpu, id: u32) -> Self {
        Self {
            this,
            id,
            preempt_depth: AtomicU32::new(0),
            need_resched: AtomicBool::new(false),
            irq_depth: Cell::new(0),
            irq_enabled: Cell::new(false),
            held: HeldLocks::new(),
            current: Cell::new(core::ptr::null()),
            previous: Cell::new(core::ptr::null()),
            idle: Cell::new(core::ptr::null()),
        }
    }

    /// The kernel's index for this CPU. The boot CPU is 0.
    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    /// The running thread, or `None` before the scheduler starts.
    pub(crate) fn current_thread(&self) -> Option<&Thread> {
        // SAFETY: a non-null `current` came from `Arc::into_raw`, and the
        // block keeps that reference until the scheduler switches away.
        // The scheduler switches only with the preempt count at 0, and the
        // `IrqGuard` behind `&self` keeps it above 0.
        unsafe { self.current.get().as_ref() }
    }

    /// Asks the scheduler to switch threads once the preempt count drops
    /// to 0.
    pub(crate) fn set_need_resched(&self) {
        self.need_resched.store(true, Ordering::Relaxed);
    }

    /// Clears the reschedule request. The scheduler calls this as it
    /// picks the next thread.
    pub(crate) fn clear_need_resched(&self) {
        self.need_resched.store(false, Ordering::Relaxed);
    }

    /// Replaces the interrupt flag the outermost [`IrqGuard`] restores
    /// when it drops, and returns the old one. The scheduler swaps it on
    /// each switch, since the thread switched to drops the guard the old
    /// thread made.
    ///
    /// # Panics
    ///
    /// Panics unless exactly one [`IrqGuard`] is alive on this CPU.
    pub(crate) fn replace_irq_enabled(&self, enabled: bool) -> bool {
        assert_eq!(self.irq_depth.get(), 1, "one IrqGuard is alive");
        self.irq_enabled.replace(enabled)
    }
}

/// The boot CPU's block.
static BOOT_CPU: PerCpu = PerCpu::new(&raw const BOOT_CPU, 0);

/// Points the boot CPU's GS base at its block. It must run before anything
/// makes a guard, so `kmain` calls it first.
pub(crate) fn init_boot_cpu() {
    // SAFETY: `BOOT_CPU` is a static, so it never moves, and its first
    // field holds its own address, as `current` expects.
    unsafe { cpu::set_gs_base(BOOT_CPU.this as u64) };
}

/// The running CPU's block, read through GS.
///
/// # Safety
///
/// The GS base must point at a [`PerCpu`]: [`init_boot_cpu`] has run on
/// this CPU.
unsafe fn this_cpu() -> *const PerCpu {
    // SAFETY: the caller guarantees GS points at a `PerCpu`, whose first
    // field is the block's address.
    unsafe { cpu::gs_read_u64(offset_of!(PerCpu, this)) as *const PerCpu }
}

/// The running CPU's block. The guard keeps interrupts masked, so nothing
/// else on this CPU uses the block while the reference lives.
pub(crate) fn current(_irq: &IrqGuard) -> &PerCpu {
    // SAFETY: a guard exists only after `init_boot_cpu` ran, and a block
    // lives for good once installed.
    unsafe { &*this_cpu() }
}

/// The preempt count of this CPU.
pub(crate) fn preempt_depth() -> u32 {
    // SAFETY: `kmain` runs `init_boot_cpu` first. A single load reads the
    // whole count.
    unsafe { cpu::gs_read_u32(offset_of!(PerCpu, preempt_depth)) }
}

/// Whether this CPU has a reschedule pending. Reads one byte, with no
/// guard.
pub(crate) fn need_resched() -> bool {
    // SAFETY: `kmain` runs `init_boot_cpu` first. The flag is one byte.
    unsafe { cpu::gs_read_u8(offset_of!(PerCpu, need_resched)) != 0 }
}

/// Whether the scheduler has given this CPU a current thread. Reads one
/// word, with no guard.
pub(crate) fn has_current_thread() -> bool {
    // SAFETY: `kmain` runs `init_boot_cpu` first. A single load reads the
    // whole pointer.
    unsafe { cpu::gs_read_u64(offset_of!(PerCpu, current)) != 0 }
}

/// The scheduler does not switch threads on this CPU while any guard is
/// alive. Dropping the last one switches if a reschedule is pending and
/// interrupts are enabled.
///
/// A guard is not `Send`, since it stands for this CPU's state.
pub(crate) struct PreemptGuard {
    _not_send: PhantomData<*const ()>,
}

impl Mask for PreemptGuard {
    fn new() -> Self {
        // SAFETY: `kmain` runs `init_boot_cpu` first, and `inc` changes the
        // count in one instruction, so an interrupt cannot split it.
        unsafe { cpu::gs_inc_u32(offset_of!(PerCpu, preempt_depth)) };
        Self {
            _not_send: PhantomData,
        }
    }

    unsafe fn assume() -> Self {
        Self {
            _not_send: PhantomData,
        }
    }
}

impl PreemptGuard {
    /// Turns preemption off on this CPU.
    pub(crate) fn new() -> Self {
        <Self as Mask>::new()
    }
}

impl Drop for PreemptGuard {
    fn drop(&mut self) {
        // SAFETY: as in `new`.
        let zero = unsafe { cpu::gs_dec_u32(offset_of!(PerCpu, preempt_depth)) };
        if !zero {
            return;
        }
        let pending = need_resched();
        // An interrupt or exception handler runs with interrupts masked. It
        // must not switch threads, since it runs on the stack of the
        // thread it interrupted.
        if pending && cpu::interrupts_enabled() {
            sched::preempt();
        }
    }
}

/// Interrupts stay masked on this CPU while any guard is alive. A guard
/// also holds a [`PreemptGuard`], so a thread with interrupts masked is
/// never switched out.
///
/// Guards nest through a per-CPU count, and only the last one to drop
/// restores the interrupt flag the first one found. So guards may drop in
/// any order. A guard is not `Send`, since it stands for this CPU's state.
pub(crate) struct IrqGuard {
    /// Dropped after interrupts are restored, so a pending reschedule runs
    /// with interrupts on.
    _preempt: PreemptGuard,
}

impl Mask for IrqGuard {
    fn new() -> Self {
        let preempt = PreemptGuard::new();
        let enabled = cpu::save_and_disable_interrupts();
        // SAFETY: `kmain` runs `init_boot_cpu` before anything makes a
        // guard. Interrupts are masked, so this CPU's block is ours until
        // the guard exists. An NMI or exception handler in between makes
        // and drops its own guards, which leaves the count as it was.
        let cpu = unsafe { &*this_cpu() };
        let depth = cpu.irq_depth.get();
        if depth == 0 {
            cpu.irq_enabled.set(enabled);
        }
        cpu.irq_depth.set(depth + 1);
        Self { _preempt: preempt }
    }

    unsafe fn assume() -> Self {
        Self {
            // SAFETY: the caller guarantees the counts include this guard.
            _preempt: unsafe { PreemptGuard::assume() },
        }
    }
}

impl IrqGuard {
    /// Masks interrupts on this CPU.
    pub(crate) fn new() -> Self {
        <Self as Mask>::new()
    }
}

impl Drop for IrqGuard {
    fn drop(&mut self) {
        let cpu = current(self);
        let depth = cpu.irq_depth.get() - 1;
        cpu.irq_depth.set(depth);
        if depth == 0 {
            cpu::restore_interrupts(cpu.irq_enabled.get());
        }
    }
}

ktest::ktest! {
    fn gs_points_at_the_boot_cpu_block() {
        let irq = IrqGuard::new();
        let cpu = current(&irq);
        assert!(core::ptr::eq(cpu, &raw const BOOT_CPU));
        assert_eq!(cpu::gs_base(), cpu.this as u64);
        assert_eq!(cpu.id(), 0);
    }
}

ktest::ktest! {
    fn irq_guards_nest_and_drop_in_any_order() {
        let depth = || current(&IrqGuard::new()).irq_depth.get() - 1;
        let before = depth();
        let outer = IrqGuard::new();
        let inner = IrqGuard::new();
        assert_eq!(depth(), before + 2);
        drop(outer);
        assert_eq!(depth(), before + 1);
        assert!(!cpu::interrupts_enabled());
        drop(inner);
        assert_eq!(depth(), before);
    }
}

ktest::ktest! {
    fn irq_guards_count_as_preempt_guards() {
        let before = preempt_depth();
        let preempt = PreemptGuard::new();
        let irq = IrqGuard::new();
        assert_eq!(preempt_depth(), before + 2);
        drop(preempt);
        drop(irq);
        assert_eq!(preempt_depth(), before);
    }
}
