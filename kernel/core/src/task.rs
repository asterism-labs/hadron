//! Kernel task types.

use crate::id::CpuId;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_from_u8_critical() {
        assert_eq!(Priority::from_u8(0), Priority::Critical);
    }

    #[test]
    fn priority_from_u8_normal() {
        assert_eq!(Priority::from_u8(1), Priority::Normal);
    }

    #[test]
    fn priority_from_u8_background() {
        assert_eq!(Priority::from_u8(2), Priority::Background);
    }

    #[test]
    fn priority_from_u8_unknown_defaults_normal() {
        assert_eq!(Priority::from_u8(255), Priority::Normal);
        assert_eq!(Priority::from_u8(42), Priority::Normal);
    }

    #[test]
    fn priority_count() {
        assert_eq!(Priority::COUNT, 3);
    }

    #[test]
    fn task_meta_default() {
        let meta = TaskMeta::default();
        assert_eq!(meta.name, "<anon>");
        assert_eq!(meta.priority, Priority::Normal);
        assert!(meta.affinity.is_none());
    }

    #[test]
    fn task_meta_builder() {
        let meta = TaskMeta::new("test-task")
            .with_priority(Priority::Critical)
            .with_affinity(CpuId::new(2));
        assert_eq!(meta.name, "test-task");
        assert_eq!(meta.priority, Priority::Critical);
        assert_eq!(meta.affinity, Some(CpuId::new(2)));
    }

    #[test]
    fn task_id_equality() {
        assert_eq!(TaskId(1), TaskId(1));
        assert_ne!(TaskId(1), TaskId(2));
    }

    #[test]
    fn task_id_ordering() {
        assert!(TaskId(1) < TaskId(2));
        assert!(TaskId(100) > TaskId(0));
    }
}

/// Unique task identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TaskId(pub u64);

/// Task priority tier for the kernel executor.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    /// Interrupt bottom-halves, hardware event completion.
    Critical = 0,
    /// Normal kernel services and device drivers.
    Normal = 1,
    /// Housekeeping: memory compaction, log flushing, statistics.
    Background = 2,
}

impl Priority {
    /// Number of priority tiers.
    pub const COUNT: usize = 3;

    /// Converts a raw u8 to a priority, defaulting to Normal.
    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => Self::Critical,
            2 => Self::Background,
            _ => Self::Normal,
        }
    }
}

/// Metadata for a spawned kernel task.
#[derive(Debug, Clone, Copy)]
pub struct TaskMeta {
    /// Human-readable name for debugging.
    pub name: &'static str,
    /// Priority tier.
    pub priority: Priority,
    /// CPU affinity: `None` = any CPU, `Some(id)` = pinned.
    pub affinity: Option<CpuId>,
}

impl Default for TaskMeta {
    fn default() -> Self {
        Self {
            name: "<anon>",
            priority: Priority::Normal,
            affinity: None,
        }
    }
}

impl TaskMeta {
    /// Creates metadata with a name and default priority.
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            priority: Priority::Normal,
            affinity: None,
        }
    }

    /// Sets the priority.
    pub const fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    /// Sets CPU affinity.
    pub const fn with_affinity(mut self, cpu: CpuId) -> Self {
        self.affinity = Some(cpu);
        self
    }
}
