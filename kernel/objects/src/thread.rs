//! Thread object.
//!
//! A thread is a schedulable execution context that belongs to a process.
//! Each thread maps to a kernel async task on the per-CPU executor.

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};
use crate::process::Process;

/// Thread execution state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadState {
    /// Thread has been created but not yet started.
    Initial,
    /// Thread is running or ready to run.
    Running,
    /// Thread is suspended (not schedulable until resumed).
    Suspended,
    /// Thread is blocked waiting on a kernel object.
    Blocked,
    /// Thread has exited.
    Dead,
}

/// A thread — a schedulable execution context within a process.
///
/// Threads share the address space and handle table of their owning process.
/// Each thread has its own:
/// - User-mode register state (saved/restored on context switch)
/// - Kernel stack
/// - Execution state (running, blocked, dead)
pub struct Thread {
    /// Unique identifier.
    koid: Koid,
    /// Human-readable name (for debugging).
    name: SpinLock<String>,
    /// The process this thread belongs to (weak to avoid cycles).
    process: Weak<Process>,
    /// Current execution state.
    state: SpinLock<ThreadState>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers for signal notifications.
    observers: ObserverList,
}

impl Thread {
    /// Create a new thread belonging to the given process.
    ///
    /// The thread starts in [`ThreadState::Initial`] and must be explicitly
    /// started via the `thread_start` syscall.
    #[must_use]
    pub fn new(name: String, process: &Arc<Process>) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            name: SpinLock::new(name),
            process: Arc::downgrade(process),
            state: SpinLock::new(ThreadState::Initial),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// The process this thread belongs to.
    ///
    /// Returns `None` if the process has been dropped (should not happen
    /// while the thread is alive).
    #[must_use]
    pub fn process(&self) -> Option<Arc<Process>> {
        self.process.upgrade()
    }

    /// The current execution state.
    #[must_use]
    pub fn state(&self) -> ThreadState {
        *self.state.lock()
    }

    /// Transition to a new state.
    pub fn set_state(&self, new_state: ThreadState) {
        *self.state.lock() = new_state;
    }

    /// Get the thread name.
    #[must_use]
    pub fn name(&self) -> String {
        self.name.lock().clone()
    }

    /// Mark the thread as dead and set the TERMINATED signal.
    pub fn exit(&self) {
        *self.state.lock() = ThreadState::Dead;
        signal_update(
            &self.signals,
            Signals::TERMINATED,
            Signals::empty(),
            &self.observers,
            self.koid,
        );
    }
}

impl KernelObject for Thread {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Thread
    }

    fn koid(&self) -> Koid {
        self.koid
    }

    fn get_signals(&self) -> Signals {
        Signals::from_bits_truncate(self.signals.load(Ordering::Relaxed))
    }

    fn add_observer(&self, port: Arc<dyn PortDispatch>, key: u64, signals: Signals) {
        self.observers.add(port, key, signals);
    }

    fn remove_observer(&self, port: &Arc<dyn PortDispatch>) {
        self.observers.remove_by_port(port);
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;
    use crate::vmar::Vmar;

    const USER_BASE: u64 = 0x0000_1000_0000_0000;
    const USER_SIZE: u64 = 0x0000_7FFF_0000_0000;

    fn make_process() -> Arc<Process> {
        let root_vmar = Vmar::new_root(USER_BASE, USER_SIZE);
        Process::new("test".to_string(), root_vmar)
    }

    #[test]
    fn thread_initial_state() {
        let proc = make_process();
        let thread = Thread::new("main".to_string(), &proc);
        assert_eq!(thread.state(), ThreadState::Initial);
        assert_eq!(thread.object_type(), ObjectType::Thread);
        assert_eq!(thread.name(), "main");
    }

    #[test]
    fn thread_state_transitions() {
        let proc = make_process();
        let thread = Thread::new("worker".to_string(), &proc);

        thread.set_state(ThreadState::Running);
        assert_eq!(thread.state(), ThreadState::Running);

        thread.set_state(ThreadState::Blocked);
        assert_eq!(thread.state(), ThreadState::Blocked);
    }

    #[test]
    fn thread_exit_signals() {
        let proc = make_process();
        let thread = Thread::new("exit-test".to_string(), &proc);

        thread.exit();
        assert_eq!(thread.state(), ThreadState::Dead);
        assert!(thread.get_signals().contains(Signals::TERMINATED));
    }

    #[test]
    fn thread_process_ref() {
        let proc = make_process();
        let proc_koid = proc.koid();
        let thread = Thread::new("ref-test".to_string(), &proc);

        let proc_ref = thread.process().expect("process should be alive");
        assert_eq!(proc_ref.koid(), proc_koid);
    }

    #[test]
    fn process_thread_registration() {
        let proc = make_process();
        let thread = Thread::new("registered".to_string(), &proc);
        let thread_koid = thread.koid();

        proc.add_thread(Arc::clone(&thread));
        assert_eq!(proc.thread_count(), 1);

        proc.remove_thread(thread_koid);
        assert_eq!(proc.thread_count(), 0);
    }
}
