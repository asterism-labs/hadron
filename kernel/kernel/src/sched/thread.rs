//! Kernel threads and their stacks.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::task::Wake;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use arch_x86_64::VirtAddr;
use arch_x86_64::structures::paging::PAGE_SIZE;

use crate::arch::x86_64::context;
use crate::memory::{self, KERNEL_STACK_SIZE, THREAD_STACKS};
use crate::sync::{HeldLocks, IrqSpinLock, WaitQueue, levels};

/// The thread is on a CPU.
pub(super) const RUNNING: u8 = 0;
/// The thread is in the run queue.
pub(super) const READY: u8 = 1;
/// The thread sleeps until something unparks it.
pub(super) const PARKED: u8 = 2;
/// The thread finished and never runs again.
pub(super) const EXITED: u8 = 3;

/// What a new thread runs.
type Entry = Box<dyn FnOnce() + Send>;

/// A kernel thread.
pub(crate) struct Thread {
    id: u64,
    name: &'static str,
    /// [`RUNNING`], [`READY`], [`PARKED`] or [`EXITED`].
    pub(super) state: AtomicU8,
    /// Set by an unpark. The next park consumes it and returns at once, so
    /// an unpark that comes before the park is not lost.
    pub(super) notified: AtomicBool,
    /// The saved stack pointer while the thread is switched out.
    pub(super) rsp: UnsafeCell<u64>,
    /// Whether interrupts were enabled when the thread was switched out,
    /// before the scheduler masked them. A thread switched out from an
    /// interrupt handler had them masked.
    pub(super) irq_enabled: UnsafeCell<bool>,
    /// The next thread in the run queue. Only the run queue lock's holder
    /// touches it.
    pub(super) next: UnsafeCell<Option<Arc<Thread>>>,
    /// What the thread runs, taken once when it starts.
    entry: UnsafeCell<Option<Entry>>,
    /// Set when the thread exits.
    finished: AtomicBool,
    /// Threads waiting in [`JoinHandle::join`].
    exited: WaitQueue,
    /// Sleeping locks the thread holds, for the run-time lock checker.
    held: HeldLocks,
    /// `None` for the boot thread, which runs on the boot stack.
    _stack: Option<KernelStack>,
}

// SAFETY: `state`, `notified` and `finished` are atomics and `exited` is a
// lock. `rsp`, `irq_enabled` and `next` are touched only under the run
// queue lock.
// `entry` and `held` are touched only by the thread itself while it runs.
unsafe impl Send for Thread {}
// SAFETY: as for `Send`.
unsafe impl Sync for Thread {}

/// The next thread ID. The boot thread is 0.
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

impl Thread {
    /// The thread that boot runs on. It runs on the boot stack and becomes
    /// the idle thread.
    pub(super) fn boot() -> Self {
        Self::new(0, "idle", 0, None, None, RUNNING)
    }

    fn new(
        id: u64,
        name: &'static str,
        rsp: u64,
        entry: Option<Entry>,
        stack: Option<KernelStack>,
        state: u8,
    ) -> Self {
        Self {
            id,
            name,
            state: AtomicU8::new(state),
            notified: AtomicBool::new(false),
            rsp: UnsafeCell::new(rsp),
            // A new thread starts with interrupts enabled.
            irq_enabled: UnsafeCell::new(true),
            next: UnsafeCell::new(None),
            entry: UnsafeCell::new(entry),
            finished: AtomicBool::new(false),
            exited: WaitQueue::new(),
            held: HeldLocks::new(),
            _stack: stack,
        }
    }

    /// The thread's ID. The boot thread is 0.
    pub(crate) fn id(&self) -> u64 {
        self.id
    }

    /// The name given to [`spawn`].
    pub(crate) fn name(&self) -> &'static str {
        self.name
    }

    /// Sleeping locks the thread holds. Only the thread itself may use
    /// this.
    pub(crate) fn held(&self) -> &HeldLocks {
        &self.held
    }
}

impl Wake for Thread {
    fn wake(self: Arc<Self>) {
        super::unpark(&self);
    }

    fn wake_by_ref(self: &Arc<Self>) {
        super::unpark(self);
    }
}

/// Starts a kernel thread that runs `entry`. It goes to the back of the run
/// queue.
///
/// # Panics
///
/// Panics if there are no frames for a new stack.
pub(crate) fn spawn<F>(name: &'static str, entry: F) -> JoinHandle
where
    F: FnOnce() + Send + 'static,
{
    let stack = KernelStack::new();
    // SAFETY: the stack is mapped, writable, 16-byte aligned at the top and
    // not used by anyone yet.
    let rsp = unsafe { context::initial_stack(stack.top().as_u64(), thread_start) };
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let thread = Arc::new(Thread::new(
        id,
        name,
        rsp,
        Some(Box::new(entry)),
        Some(stack),
        READY,
    ));
    super::enqueue(thread.clone());
    JoinHandle { thread }
}

/// Where a new thread starts, through the frame `initial_stack` built.
extern "C" fn thread_start() -> ! {
    super::finish_first_switch();
    let thread = super::current();
    // SAFETY: only the thread itself takes its entry, and only here.
    let entry = unsafe { (*thread.entry.get()).take() };
    drop(thread);
    if let Some(entry) = entry {
        entry();
    }
    exit()
}

/// Ends the current thread and wakes the threads joining it.
fn exit() -> ! {
    let thread = super::current();
    thread.finished.store(true, Ordering::Release);
    thread.exited.wake_all();
    drop(thread);
    super::schedule(super::Switch::Exit);
    unreachable!("an exited thread was switched back to");
}

/// A handle to a spawned thread.
pub(crate) struct JoinHandle {
    thread: Arc<Thread>,
}

impl JoinHandle {
    /// Sleeps until the thread exits.
    ///
    /// # Panics
    ///
    /// Panics if the calling thread may not sleep (see
    /// [`super::might_sleep`]).
    #[track_caller]
    pub(crate) fn join(self) {
        let thread = &self.thread;
        thread
            .exited
            .block_until(|| thread.finished.load(Ordering::Acquire));
    }

    /// The thread.
    pub(crate) fn thread(&self) -> &Thread {
        &self.thread
    }
}

/// Kernel stack slots. Slot `n` is a guard page at
/// `THREAD_STACKS + n * STACK_SLOT`, then [`KERNEL_STACK_SIZE`] bytes of
/// stack. A freed slot stays mapped and goes on the free list.
struct StackSlots {
    /// The lowest slot never used.
    next: u64,
    /// Freed slots, still mapped.
    free: Vec<u64>,
}

/// Bytes between the starts of two stack slots.
const STACK_SLOT: u64 = PAGE_SIZE + KERNEL_STACK_SIZE;

/// The stack slots. Dropping a thread frees its slot, and the last
/// reference to a thread can drop in an interrupt handler, so the lock is
/// IRQ-safe.
static STACKS: IrqSpinLock<levels::Stacks, StackSlots> = IrqSpinLock::new(StackSlots {
    next: 0,
    free: Vec::new(),
});

/// A kernel thread's stack. Dropping it returns the slot to the free list.
struct KernelStack {
    slot: u64,
}

impl KernelStack {
    /// Takes a free slot, or maps a new one.
    fn new() -> Self {
        let mut slots = STACKS.lock();
        if let Some(slot) = slots.free.pop() {
            return Self { slot };
        }
        let slot = slots.next;
        slots.next += 1;
        let stack = Self { slot };
        memory::map_new_pages(stack.bottom(), KERNEL_STACK_SIZE)
            .expect("frames for a kernel stack");
        stack
    }

    fn bottom(&self) -> VirtAddr {
        THREAD_STACKS + self.slot * STACK_SLOT + PAGE_SIZE
    }

    fn top(&self) -> VirtAddr {
        self.bottom() + KERNEL_STACK_SIZE
    }
}

impl Drop for KernelStack {
    fn drop(&mut self) {
        STACKS.lock().free.push(self.slot);
    }
}

/// How many stack slots have ever been mapped.
pub(super) fn mapped_stacks() -> u64 {
    STACKS.lock().next
}

ktest::ktest! {
    fn thread_stacks_have_guard_pages() {
        let rsp = crate::arch::x86_64::cpu::stack_pointer();
        let slot = (rsp - THREAD_STACKS.as_u64()) / STACK_SLOT;
        let guard = THREAD_STACKS + slot * STACK_SLOT;
        assert_eq!(memory::translate(guard), None);
        assert!(memory::translate(guard + PAGE_SIZE).is_some());
        assert!(rsp > (guard + PAGE_SIZE).as_u64());
    }
}

ktest::ktest! {
    #[ignore]
    fn lockdep_reports_locks_taken_out_of_order() {
        use crate::sync::{IrqSpinLock, levels::KtestSpin};
        // Shows the run-time checker's report. Run it with
        // `QUON_KTEST_IGNORED=true quon run -p test`.
        static LATE: IrqSpinLock<KtestSpin, ()> = IrqSpinLock::new(());
        let _late = LATE.lock();
        let _early = STACKS.lock();
    }
}
