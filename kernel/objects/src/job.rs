//! Job object — process group container with hierarchy.
//!
//! Jobs form a tree rooted at a single root job. Each job can contain child
//! jobs and processes. Jobs define resource policies and limits for their
//! descendants. Killing a job kills all its children and processes recursively.

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};

use hadron_core::sync::SpinLock;

use crate::object::{KernelObject, Koid, ObjectType, Signals};
use crate::observer::{ObserverList, PortDispatch, signal_update};
use crate::process::Process;

/// Resource limits for a job and its descendants.
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum number of processes allowed (0 = unlimited).
    pub max_processes: u32,
    /// Maximum total memory in bytes (0 = unlimited).
    pub max_memory: u64,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_processes: 0,
            max_memory: 0,
        }
    }
}

/// Policy flags for a job.
#[derive(Debug, Clone)]
pub struct JobPolicy {
    /// Whether new processes can be created in this job.
    pub allow_new_process: bool,
    /// Whether ambient VMO exec rights are granted.
    pub allow_ambient_exec: bool,
}

impl Default for JobPolicy {
    fn default() -> Self {
        Self {
            allow_new_process: true,
            allow_ambient_exec: true,
        }
    }
}

/// Errors from job operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobError {
    /// The job has been killed; no new children/processes can be added.
    Dead,
}

/// A job — a container of processes and child jobs.
///
/// The job tree defines the resource and policy hierarchy. The root job is
/// the ancestor of all processes in the system.
pub struct Job {
    /// Unique identifier.
    koid: Koid,
    /// Human-readable name (for debugging).
    name: SpinLock<String>,
    /// Parent job (weak to avoid cycles). `None` for the root job.
    parent: Option<Weak<Job>>,
    /// Child jobs.
    children: SpinLock<Vec<Arc<Job>>>,
    /// Processes directly owned by this job.
    processes: SpinLock<Vec<Weak<Process>>>,
    /// Policy governing this job's descendants.
    policy: SpinLock<JobPolicy>,
    /// Resource limits for this job subtree.
    limits: SpinLock<ResourceLimits>,
    /// Current signal state.
    signals: AtomicU32,
    /// Registered observers.
    observers: ObserverList,
}

impl Job {
    /// Create the root job (no parent).
    #[must_use]
    pub fn create_root(name: String) -> Arc<Self> {
        Arc::new(Self {
            koid: Koid::alloc(),
            name: SpinLock::new(name),
            parent: None,
            children: SpinLock::new(Vec::new()),
            processes: SpinLock::new(Vec::new()),
            policy: SpinLock::new(JobPolicy::default()),
            limits: SpinLock::new(ResourceLimits::default()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        })
    }

    /// Create a child job under this job.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::Dead`] if this job has been killed.
    pub fn create_child(self: &Arc<Self>, name: String) -> Result<Arc<Job>, JobError> {
        if self.get_signals().contains(Signals::TERMINATED) {
            return Err(JobError::Dead);
        }

        let child = Arc::new(Job {
            koid: Koid::alloc(),
            name: SpinLock::new(name),
            parent: Some(Arc::downgrade(self)),
            children: SpinLock::new(Vec::new()),
            processes: SpinLock::new(Vec::new()),
            policy: SpinLock::new(self.policy.lock().clone()),
            limits: SpinLock::new(self.limits.lock().clone()),
            signals: AtomicU32::new(0),
            observers: ObserverList::new(),
        });

        self.children.lock().push(Arc::clone(&child));
        Ok(child)
    }

    /// Add a process to this job.
    ///
    /// # Errors
    ///
    /// Returns [`JobError::Dead`] if this job has been killed.
    pub fn add_process(&self, process: &Arc<Process>) -> Result<(), JobError> {
        if self.get_signals().contains(Signals::TERMINATED) {
            return Err(JobError::Dead);
        }
        self.processes.lock().push(Arc::downgrade(process));
        Ok(())
    }

    /// Remove a process from this job by koid.
    pub fn remove_process(&self, koid: Koid) {
        self.processes
            .lock()
            .retain(|p| p.upgrade().is_some_and(|proc| proc.koid() != koid));
    }

    /// Set the job policy.
    pub fn set_policy(&self, policy: JobPolicy) {
        *self.policy.lock() = policy;
    }

    /// Get the current job policy.
    #[must_use]
    pub fn policy(&self) -> JobPolicy {
        self.policy.lock().clone()
    }

    /// Set resource limits.
    pub fn set_limits(&self, limits: ResourceLimits) {
        *self.limits.lock() = limits;
    }

    /// Kill this job and all descendants (depth-first).
    ///
    /// Sets TERMINATED on this job and recursively kills all child jobs.
    /// Processes owned by this job are signaled to terminate.
    pub fn kill(&self) {
        // Kill child jobs depth-first.
        let children = self.children.lock().clone();
        for child in &children {
            child.kill();
        }

        // Signal processes to terminate.
        let processes = self.processes.lock().clone();
        for weak_proc in &processes {
            if let Some(proc) = weak_proc.upgrade() {
                proc.exit(-1);
            }
        }

        signal_update(
            &self.signals,
            Signals::TERMINATED,
            Signals::empty(),
            &self.observers,
            self.koid,
        );
    }

    /// The job name.
    #[must_use]
    pub fn name(&self) -> String {
        self.name.lock().clone()
    }

    /// Number of direct child jobs.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.lock().len()
    }

    /// Number of processes (including dead weak refs).
    #[must_use]
    pub fn process_count(&self) -> usize {
        self.processes
            .lock()
            .iter()
            .filter(|p| p.strong_count() > 0)
            .count()
    }

    /// The parent job, if any.
    #[must_use]
    pub fn parent(&self) -> Option<Arc<Job>> {
        self.parent.as_ref().and_then(Weak::upgrade)
    }
}

impl KernelObject for Job {
    fn object_type(&self) -> ObjectType {
        ObjectType::Job
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

    fn make_process(name: &str) -> Arc<Process> {
        let root_vmar = Vmar::new_root(USER_BASE, USER_SIZE);
        Process::new(name.to_string(), root_vmar)
    }

    #[test]
    fn job_create_root() {
        let root = Job::create_root("root".to_string());
        assert_eq!(root.object_type(), ObjectType::Job);
        assert_eq!(root.name(), "root");
        assert!(root.parent().is_none());
    }

    #[test]
    fn job_create_hierarchy() {
        let root = Job::create_root("root".to_string());
        let child = root.create_child("child".to_string()).unwrap();
        let grandchild = child.create_child("grandchild".to_string()).unwrap();

        assert_eq!(root.child_count(), 1);
        assert_eq!(child.child_count(), 1);
        assert!(child.parent().is_some());
        assert_eq!(grandchild.parent().unwrap().koid(), child.koid());
    }

    #[test]
    fn job_add_process() {
        let root = Job::create_root("root".to_string());
        let proc = make_process("test");
        root.add_process(&proc).unwrap();
        assert_eq!(root.process_count(), 1);
    }

    #[test]
    fn job_remove_process() {
        let root = Job::create_root("root".to_string());
        let proc = make_process("test");
        let koid = proc.koid();
        root.add_process(&proc).unwrap();
        root.remove_process(koid);
        assert_eq!(root.process_count(), 0);
    }

    #[test]
    fn job_kill_propagates() {
        let root = Job::create_root("root".to_string());
        let child = root.create_child("child".to_string()).unwrap();
        let proc = make_process("test");
        child.add_process(&proc).unwrap();

        root.kill();

        assert!(root.get_signals().contains(Signals::TERMINATED));
        assert!(child.get_signals().contains(Signals::TERMINATED));
        assert!(proc.get_signals().contains(Signals::TERMINATED));
    }

    #[test]
    fn job_dead_rejects_children() {
        let root = Job::create_root("root".to_string());
        root.kill();

        assert!(matches!(
            root.create_child("late".to_string()),
            Err(JobError::Dead)
        ));
    }

    #[test]
    fn job_policy_inherited() {
        let root = Job::create_root("root".to_string());
        root.set_policy(JobPolicy {
            allow_new_process: false,
            allow_ambient_exec: false,
        });

        let child = root.create_child("child".to_string()).unwrap();
        assert!(!child.policy().allow_new_process);
        assert!(!child.policy().allow_ambient_exec);
    }
}
