//! Per-CPU data and the interrupt guard that grants access to it.
//!
//! Each CPU has one [`PerCpu`] block, and its GS base points at it. The
//! block's first field holds the block's own address, so [`current`]
//! finds it with one `gs:`-relative load.
//!
//! Only the CPU that owns a block uses it, so its fields need no locks or
//! atomic read-modify-writes. They are `Cell`s instead, which is sound only
//! while nothing else on this CPU can run in between. An interrupt handler
//! could otherwise run halfway through an update. So [`current`] takes an
//! [`IrqGuard`], which keeps interrupts masked.
//!
//! The boot CPU's block is a static. Other CPUs will get theirs from the
//! heap when SMP starts.

use core::cell::Cell;
use core::marker::PhantomData;

use crate::arch::x86_64::cpu;

/// One CPU's data. It sits in its own cache lines, so CPUs do not slow
/// each other down by writing to neighboring blocks.
#[repr(C, align(64))]
pub(crate) struct PerCpu {
    /// The block's own address. It must stay the first field: [`current`]
    /// reads it at `gs:[0]`.
    this: *const PerCpu,
    /// The kernel's index for this CPU. The boot CPU is 0.
    id: u32,
    /// How many [`IrqGuard`]s are alive on this CPU.
    irq_depth: Cell<u32>,
    /// Whether interrupts were enabled when the outermost [`IrqGuard`] was
    /// made. The last guard to drop restores this.
    irq_enabled: Cell<bool>,
}

// SAFETY: a block is shared only in the sense that a static holds it. Its
// `Cell`s are used only through `current`, which returns the running CPU's
// own block while interrupts are masked, so no two contexts touch them at
// once.
unsafe impl Sync for PerCpu {}

impl PerCpu {
    const fn new(this: *const PerCpu, id: u32) -> Self {
        Self {
            this,
            id,
            irq_depth: Cell::new(0),
            irq_enabled: Cell::new(false),
        }
    }

    /// The kernel's index for this CPU. The boot CPU is 0.
    pub(crate) fn id(&self) -> u32 {
        self.id
    }
}

/// The boot CPU's block.
static BOOT_CPU: PerCpu = PerCpu::new(&raw const BOOT_CPU, 0);

/// Points the boot CPU's GS base at its block. It must run before anything
/// makes an [`IrqGuard`], so `kmain` calls it first.
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
    unsafe { cpu::read_gs() as *const PerCpu }
}

/// The running CPU's block. The guard keeps interrupts masked, so nothing
/// else on this CPU uses the block while the reference lives.
pub(crate) fn current(_irq: &IrqGuard) -> &PerCpu {
    // SAFETY: an `IrqGuard` exists only after `init_boot_cpu` ran, and a
    // block lives for good once installed.
    unsafe { &*this_cpu() }
}

/// Interrupts stay masked on this CPU while any guard is alive.
///
/// Guards nest through a per-CPU count, and only the last one to drop
/// restores the interrupt flag the first one found. So guards may drop in
/// any order. A guard is not `Send`, since it stands for this CPU's state.
pub(crate) struct IrqGuard {
    _not_send: PhantomData<*const ()>,
}

impl IrqGuard {
    /// Masks interrupts on this CPU.
    pub(crate) fn new() -> Self {
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
        Self {
            _not_send: PhantomData,
        }
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
        drop(inner);
        assert_eq!(depth(), before);
    }
}
