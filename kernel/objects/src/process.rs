//! Process object.
//!
//! A process is an address space container with a handle table and a set of
//! threads. In the Hadron microkernel, processes have no file descriptor table
//! or current working directory — handles replace file descriptors, and
//! namespaces replace cwd.

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::handle::HandleTable;
use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};
use crate::thread::Thread;
use crate::vmar::Vmar;

/// A process — address space + handle table + thread group.
///
/// Processes are the fundamental isolation boundary. Each process has:
/// - A root VMAR defining its virtual address space
/// - A handle table for referencing kernel objects
/// - A set of threads executing within the address space
/// - An optional parent Job for resource accounting
pub struct Process {
    /// Unique identifier.
    koid: Koid,
    /// Human-readable name (for debugging).
    name: SpinLock<String>,
    /// Per-process handle table.
    handle_table: SpinLock<HandleTable>,
    /// Root virtual memory address region.
    root_vmar: Arc<Vmar>,
    /// Threads belonging to this process.
    threads: SpinLock<Vec<Arc<Thread>>>,
    /// Parent job (weak to avoid cycles).
    job: SpinLock<Option<Weak<dyn KernelObject>>>,
    /// Process return code, set on exit.
    return_code: AtomicI64,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers for signal notifications.
    observers: ObserverList,
}

impl Process {
    /// Create a new process with the given root VMAR.
    ///
    /// The process starts with an empty handle table and no threads.
    /// Threads must be created separately and added via [`add_thread`](Self::add_thread).
    #[must_use]
    pub fn new(name: String, root_vmar: Arc<Vmar>) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            name: SpinLock::new(name),
            handle_table: SpinLock::new(HandleTable::new()),
            root_vmar,
            threads: SpinLock::new(Vec::new()),
            job: SpinLock::new(None),
            return_code: AtomicI64::new(0),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// The process's root VMAR (address space root).
    #[must_use]
    pub fn root_vmar(&self) -> &Arc<Vmar> {
        &self.root_vmar
    }

    /// Access the handle table under a lock.
    pub fn with_handle_table<R>(&self, f: impl FnOnce(&mut HandleTable) -> R) -> R {
        f(&mut self.handle_table.lock())
    }

    /// Add a thread to this process.
    pub fn add_thread(&self, thread: Arc<Thread>) {
        self.threads.lock().push(thread);
    }

    /// Remove a thread from this process by koid.
    pub fn remove_thread(&self, koid: Koid) {
        self.threads.lock().retain(|t| t.koid() != koid);
    }

    /// The number of threads in this process.
    #[must_use]
    pub fn thread_count(&self) -> usize {
        self.threads.lock().len()
    }

    /// Set the parent job.
    pub fn set_job(&self, job: Weak<dyn KernelObject>) {
        *self.job.lock() = Some(job);
    }

    /// Get the process name.
    #[must_use]
    pub fn name(&self) -> String {
        self.name.lock().clone()
    }

    /// Set the process name.
    pub fn set_name(&self, name: String) {
        *self.name.lock() = name;
    }

    /// Set the return code and signal termination.
    pub fn exit(&self, code: i64) {
        self.return_code.store(code, Ordering::Release);
        signal_update(
            &self.signals,
            Signals::TERMINATED,
            Signals::empty(),
            &self.observers,
            self.koid,
        );
    }

    /// The process return code (meaningful only after TERMINATED signal).
    #[must_use]
    pub fn return_code(&self) -> i64 {
        self.return_code.load(Ordering::Acquire)
    }
}

impl KernelObject for Process {
    fn as_any(&self) -> &dyn core::any::Any {
        self
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Process
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

    const USER_BASE: u64 = 0x0000_1000_0000_0000;
    const USER_SIZE: u64 = 0x0000_7FFF_0000_0000;

    fn make_process(name: &str) -> Arc<Process> {
        let root_vmar = Vmar::new_root(USER_BASE, USER_SIZE);
        Process::new(name.to_string(), root_vmar)
    }

    #[test]
    fn process_properties() {
        let proc = make_process("test-proc");
        assert_eq!(proc.object_type(), ObjectType::Process);
        assert_eq!(proc.name(), "test-proc");
        assert_eq!(proc.thread_count(), 0);
    }

    #[test]
    fn process_exit_sets_signals() {
        let proc = make_process("exit-test");
        proc.exit(42);
        assert_eq!(proc.return_code(), 42);
        assert!(proc.get_signals().contains(Signals::TERMINATED));
    }

    #[test]
    fn process_handle_table() {
        use crate::handle::{HandleEntry, HandleValue, Rights};
        use crate::vmo::Vmo;

        let proc = make_process("handle-test");
        let vmo = Vmo::new_paged(4096);

        let hv = proc.with_handle_table(|ht| {
            ht.insert(HandleEntry::new(vmo, Rights::VMO_DEFAULT))
                .expect("insert failed")
        });
        assert_ne!(hv, HandleValue::INVALID);

        proc.with_handle_table(|ht| {
            let entry = ht.get(hv).expect("get failed");
            assert_eq!(entry.rights(), Rights::VMO_DEFAULT);
        });
    }

    #[test]
    fn process_name_change() {
        let proc = make_process("original");
        proc.set_name("renamed".to_string());
        assert_eq!(proc.name(), "renamed");
    }
}
