//! Scheduling primitives.
//!
//! Contains priority-aware ready queues and related scheduling logic.
//! These types are host-testable and used by the kernel's async executor.

extern crate alloc;

use alloc::collections::VecDeque;

use crate::task::{Priority, TaskId};

/// How many consecutive Normal polls before forcing one Background poll.
const BACKGROUND_STARVATION_LIMIT: u64 = 100;

/// Priority-aware ready queues.
///
/// Maintains one FIFO queue per priority tier. Pops from the highest
/// priority (lowest ordinal) non-empty queue first.
pub struct ReadyQueues {
    queues: [VecDeque<TaskId>; Priority::COUNT],
    /// Counter for background starvation prevention.
    /// Incremented each time a Normal task is popped while Background tasks wait.
    normal_streak: u64,
}

impl ReadyQueues {
    /// Creates empty ready queues.
    pub fn new() -> Self {
        Self {
            queues: [VecDeque::new(), VecDeque::new(), VecDeque::new()],
            normal_streak: 0,
        }
    }

    /// Pushes a task into the queue for the given priority.
    pub fn push(&mut self, priority: Priority, id: TaskId) {
        self.queues[priority as usize].push_back(id);
    }

    /// Pops the highest-priority ready task.
    ///
    /// Always drains Critical first. Between Normal and Background,
    /// applies starvation prevention: if Normal has run for
    /// `BACKGROUND_STARVATION_LIMIT` consecutive pops and Background
    /// has tasks, pop one Background task instead.
    pub fn pop(&mut self) -> Option<(Priority, TaskId)> {
        // Critical always first.
        if let Some(id) = self.queues[Priority::Critical as usize].pop_front() {
            self.normal_streak = 0;
            return Some((Priority::Critical, id));
        }

        // Starvation prevention: if Normal has been running too long
        // and Background has work, give Background a turn.
        let has_background = !self.queues[Priority::Background as usize].is_empty();
        let has_normal = !self.queues[Priority::Normal as usize].is_empty();

        if has_normal && has_background && self.normal_streak >= BACKGROUND_STARVATION_LIMIT {
            self.normal_streak = 0;
            if let Some(id) = self.queues[Priority::Background as usize].pop_front() {
                return Some((Priority::Background, id));
            }
        }

        // Normal next.
        if let Some(id) = self.queues[Priority::Normal as usize].pop_front() {
            if has_background {
                self.normal_streak += 1;
            } else {
                self.normal_streak = 0;
            }
            return Some((Priority::Normal, id));
        }

        // Background last.
        self.normal_streak = 0;
        self.queues[Priority::Background as usize]
            .pop_front()
            .map(|id| (Priority::Background, id))
    }

    /// Returns `true` if any priority queue has tasks.
    pub fn has_ready(&self) -> bool {
        self.queues.iter().any(|q| !q.is_empty())
    }

    /// Steals one task from the back of the queue for work stealing.
    ///
    /// Returns a Normal or Background task (never Critical). Steals from
    /// the back to preserve locality — the victim keeps its hot (front)
    /// tasks while the thief gets the coldest (most recently enqueued) one.
    ///
    /// **One-task rule**: refuses to steal if the victim has only 1 stealable
    /// task (Normal + Background combined). This prevents the bouncing
    /// livelock where a sole task is stolen back and forth between CPUs
    /// without making forward progress.
    pub fn steal_one(&mut self) -> Option<(Priority, TaskId)> {
        // One-task rule: never steal the victim's only runnable task.
        // This prevents the bouncing livelock where idle CPUs endlessly
        // steal a single task from each other, each polling it once before
        // the next steal. The victim needs at least 1 task to guarantee
        // local forward progress.
        let stealable = self.queues[Priority::Normal as usize].len()
            + self.queues[Priority::Background as usize].len();
        if stealable <= 1 {
            return None;
        }

        // Prefer stealing Normal over Background.
        if let Some(id) = self.queues[Priority::Normal as usize].pop_back() {
            return Some((Priority::Normal, id));
        }
        if let Some(id) = self.queues[Priority::Background as usize].pop_back() {
            return Some((Priority::Background, id));
        }
        None
    }
}

impl Default for ReadyQueues {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // ReadyQueues basic behavior
    // -----------------------------------------------------------------------

    #[test]
    fn empty_on_creation() {
        let mut rq = ReadyQueues::new();
        assert!(!rq.has_ready());
        assert_eq!(rq.pop(), None);
    }

    #[test]
    fn critical_always_first() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Normal, TaskId(1));
        rq.push(Priority::Critical, TaskId(2));
        rq.push(Priority::Background, TaskId(3));

        assert_eq!(rq.pop(), Some((Priority::Critical, TaskId(2))));
        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(1))));
        assert_eq!(rq.pop(), Some((Priority::Background, TaskId(3))));
        assert_eq!(rq.pop(), None);
    }

    #[test]
    fn fifo_within_priority() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Normal, TaskId(1));
        rq.push(Priority::Normal, TaskId(2));
        rq.push(Priority::Normal, TaskId(3));

        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(1))));
        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(2))));
        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(3))));
    }

    #[test]
    fn has_ready_tracks_state() {
        let mut rq = ReadyQueues::new();
        assert!(!rq.has_ready());

        rq.push(Priority::Normal, TaskId(1));
        assert!(rq.has_ready());

        rq.pop();
        assert!(!rq.has_ready());
    }

    #[test]
    fn starvation_prevention() {
        let mut rq = ReadyQueues::new();

        // Push one background task and BACKGROUND_STARVATION_LIMIT normal tasks.
        rq.push(Priority::Background, TaskId(999));
        for i in 0..BACKGROUND_STARVATION_LIMIT {
            rq.push(Priority::Normal, TaskId(i));
        }

        // Pop BACKGROUND_STARVATION_LIMIT normal tasks.
        for _ in 0..BACKGROUND_STARVATION_LIMIT {
            let (pri, _) = rq.pop().unwrap();
            assert_eq!(pri, Priority::Normal);
        }

        // The next pop should give us the background task (starvation prevention).
        // Push one more normal task so both queues are non-empty.
        rq.push(Priority::Normal, TaskId(1000));
        let (pri, id) = rq.pop().unwrap();
        assert_eq!(pri, Priority::Background);
        assert_eq!(id, TaskId(999));
    }

    // -----------------------------------------------------------------------
    // Work stealing
    // -----------------------------------------------------------------------

    #[test]
    fn steal_takes_from_back() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Normal, TaskId(1)); // front
        rq.push(Priority::Normal, TaskId(2)); // back

        // steal_one takes from the back (LIFO), pop takes from front (FIFO).
        let stolen = rq.steal_one();
        assert_eq!(stolen, Some((Priority::Normal, TaskId(2))));

        let popped = rq.pop();
        assert_eq!(popped, Some((Priority::Normal, TaskId(1))));
    }

    #[test]
    fn steal_never_takes_critical() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Critical, TaskId(1));

        // steal_one only looks at Normal and Background.
        assert_eq!(rq.steal_one(), None);
        // But pop should find it.
        assert_eq!(rq.pop(), Some((Priority::Critical, TaskId(1))));
    }

    #[test]
    fn steal_prefers_normal_over_background() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Background, TaskId(1));
        rq.push(Priority::Normal, TaskId(2));

        // 2 stealable tasks total → steal is allowed.
        let stolen = rq.steal_one();
        assert_eq!(stolen, Some((Priority::Normal, TaskId(2))));
    }

    // -----------------------------------------------------------------------
    // One-task rule: never steal the victim's only runnable task
    // -----------------------------------------------------------------------

    #[test]
    fn steal_refuses_sole_normal_task() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Normal, TaskId(1));

        // Only 1 stealable task → one-task rule prevents steal.
        assert_eq!(rq.steal_one(), None);

        // But the task is still there for local polling.
        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(1))));
    }

    #[test]
    fn steal_refuses_sole_background_task() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Background, TaskId(1));

        assert_eq!(rq.steal_one(), None);
        assert_eq!(rq.pop(), Some((Priority::Background, TaskId(1))));
    }

    #[test]
    fn steal_refuses_with_only_critical() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Critical, TaskId(1));
        rq.push(Priority::Normal, TaskId(2));

        // Critical tasks don't count as stealable.
        // Only 1 stealable (Normal) → one-task rule prevents steal.
        assert_eq!(rq.steal_one(), None);
    }

    #[test]
    fn steal_allowed_with_two_normal() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Normal, TaskId(1));
        rq.push(Priority::Normal, TaskId(2));

        // 2 stealable → steal succeeds, takes from back.
        assert_eq!(rq.steal_one(), Some((Priority::Normal, TaskId(2))));
        // Victim keeps 1 task.
        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(1))));
    }

    #[test]
    fn steal_allowed_cross_priority() {
        let mut rq = ReadyQueues::new();
        rq.push(Priority::Normal, TaskId(1));
        rq.push(Priority::Background, TaskId(2));

        // 1 Normal + 1 Background = 2 stealable → steal allowed.
        // Prefers Normal.
        assert_eq!(rq.steal_one(), Some((Priority::Normal, TaskId(1))));
    }

    #[test]
    fn one_task_rule_prevents_bouncing() {
        // Demonstrates the key fix: with the one-task rule, a sole task
        // cannot be stolen, preventing the bouncing livelock entirely.
        let mut cpu_a = ReadyQueues::new();

        let task = TaskId(42);
        cpu_a.push(Priority::Normal, task);

        // CPU A polls task, yields, re-queues.
        let (_, popped) = cpu_a.pop().unwrap();
        assert_eq!(popped, task);
        cpu_a.push(Priority::Normal, task);

        // CPU B tries to steal — refused because only 1 task.
        assert_eq!(cpu_a.steal_one(), None);

        // Task stays on CPU A for local re-polling.
        assert!(cpu_a.has_ready());
        assert_eq!(cpu_a.pop(), Some((Priority::Normal, task)));
    }

    // -----------------------------------------------------------------------
    // Livelock bug reproduction: stranded tasks after preempt_pending
    //
    // These tests simulate the executor's run loop state transitions to
    // verify the has_ready guard prevents the livelock.
    // -----------------------------------------------------------------------

    /// Simulates the state after poll_ready_tasks breaks early due to
    /// preempt_pending: a task was polled, yielded (waker re-queued it),
    /// but the poll loop broke before processing it again.
    fn simulate_preempt_break(rq: &mut ReadyQueues) -> TaskId {
        let id = TaskId(42);
        rq.push(Priority::Normal, id);

        // Simulate: executor pops task for polling.
        let (pri, popped) = rq.pop().unwrap();
        assert_eq!(popped, id);
        assert_eq!(pri, Priority::Normal);

        // Simulate: task yields → waker re-queues it.
        rq.push(Priority::Normal, id);

        // Simulate: preempt_pending fires → poll loop breaks.
        // Task is now "stranded" in the ready queue.
        id
    }

    #[test]
    fn stranded_task_detected_by_has_ready() {
        let mut rq = ReadyQueues::new();
        simulate_preempt_break(&mut rq);

        // The has_ready guard must detect the stranded task.
        assert!(
            rq.has_ready(),
            "has_ready must return true for stranded tasks"
        );
    }

    #[test]
    fn stranded_task_not_stolen_by_one_task_rule() {
        // With the one-task rule, a stranded sole task can't be stolen.
        // This is the primary defense against the bouncing livelock.
        let mut rq = ReadyQueues::new();
        simulate_preempt_break(&mut rq);

        // Another CPU tries to steal — refused (only 1 stealable task).
        assert_eq!(rq.steal_one(), None);

        // Task remains for local re-polling.
        assert!(rq.has_ready());
    }

    #[test]
    fn stranded_task_stolen_when_multiple_exist() {
        // With 2+ stealable tasks, stealing is allowed. Demonstrates
        // that the one-task rule is not overly conservative.
        let mut rq = ReadyQueues::new();
        simulate_preempt_break(&mut rq); // id=42 stranded
        rq.push(Priority::Normal, TaskId(99)); // second task

        // 2 stealable tasks → steal succeeds (takes from back).
        let stolen = rq.steal_one();
        assert_eq!(stolen, Some((Priority::Normal, TaskId(99))));

        // Victim keeps the stranded task.
        assert!(rq.has_ready());
        assert_eq!(rq.pop(), Some((Priority::Normal, TaskId(42))));
    }

    #[test]
    fn guard_before_steal_prevents_loss() {
        // Demonstrates the fix: checking has_ready BEFORE steal means
        // stranded tasks get re-polled locally.
        let mut rq = ReadyQueues::new();
        simulate_preempt_break(&mut rq);

        // The correct run loop checks has_ready FIRST:
        assert!(
            rq.has_ready(),
            "guard fires: CPU should re-poll, not attempt steal"
        );

        // Because has_ready is true, the run loop continues (re-polls).
        // steal_one is never reached. Task stays local.
        let (pri, id) = rq.pop().unwrap();
        assert_eq!(pri, Priority::Normal);
        assert_eq!(id, TaskId(42));
    }

    #[test]
    fn guard_allows_steal_when_truly_idle() {
        let rq = ReadyQueues::new();

        // No tasks — guard should NOT fire.
        assert!(
            !rq.has_ready(),
            "empty queue: guard doesn't fire, steal proceeds"
        );
    }

    #[test]
    fn multiple_stranded_tasks() {
        let mut rq = ReadyQueues::new();

        // Spawn 3 tasks.
        rq.push(Priority::Normal, TaskId(1));
        rq.push(Priority::Normal, TaskId(2));
        rq.push(Priority::Normal, TaskId(3));

        // Simulate: poll pops id=1, polls it, it yields → re-queued.
        let (_, popped) = rq.pop().unwrap();
        assert_eq!(popped, TaskId(1));
        rq.push(Priority::Normal, TaskId(1));

        // preempt_pending fires — id=2 and id=3 were never popped.
        // All 3 tasks are stranded.
        assert!(rq.has_ready());

        // Verify all 3 IDs are recoverable (order: 2, 3, 1).
        let mut ids = Vec::new();
        while let Some((_, id)) = rq.pop() {
            ids.push(id);
        }
        assert_eq!(ids, vec![TaskId(2), TaskId(3), TaskId(1)]);
    }

    #[test]
    fn bouncing_prevented_by_one_task_rule() {
        // With the one-task rule, a sole task cannot bounce between CPUs
        // because steal_one refuses to take the victim's only task.
        let mut cpu_a = ReadyQueues::new();

        let task = TaskId(42);
        cpu_a.push(Priority::Normal, task);

        // Iteration 1: CPU A polls task, task yields, preempt breaks.
        let (_, popped) = cpu_a.pop().unwrap();
        assert_eq!(popped, task);
        cpu_a.push(Priority::Normal, task); // waker re-queues

        // CPU B tries to steal → refused (only 1 stealable task).
        assert_eq!(cpu_a.steal_one(), None);

        // CPU A's has_ready guard catches the stranded task → re-polls.
        assert!(cpu_a.has_ready());
        let (_, recovered) = cpu_a.pop().unwrap();
        assert_eq!(recovered, task);
        // Task stays on CPU A — no bouncing, guaranteed forward progress.
    }

    #[test]
    fn bouncing_possible_with_multiple_tasks() {
        // With 2+ tasks, stealing IS allowed. Verify the thief gets one
        // and the victim keeps one — both make progress.
        let mut cpu_a = ReadyQueues::new();
        let mut cpu_b = ReadyQueues::new();

        cpu_a.push(Priority::Normal, TaskId(1));
        cpu_a.push(Priority::Normal, TaskId(2));

        // CPU A polls task 1, it yields, preempt breaks.
        let (_, popped) = cpu_a.pop().unwrap();
        assert_eq!(popped, TaskId(1));
        cpu_a.push(Priority::Normal, TaskId(1)); // re-queued

        // CPU B steals → succeeds (2 stealable tasks).
        let stolen = cpu_a.steal_one();
        assert_eq!(stolen, Some((Priority::Normal, TaskId(1)))); // back
        cpu_b.push(Priority::Normal, TaskId(1));

        // CPU A keeps task 2, CPU B has task 1 — both busy.
        assert!(cpu_a.has_ready());
        assert!(cpu_b.has_ready());
    }

    // -----------------------------------------------------------------------
    // Concurrent stress test: simulates the full executor run loop with
    // multiple threads to verify no tasks are lost under contention.
    // -----------------------------------------------------------------------

    #[test]
    fn concurrent_no_task_loss() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::{Duration, Instant};

        const NUM_CPUS: usize = 4;
        const NUM_TASKS: usize = 20;
        const POLLS_TO_COMPLETE: u64 = 50;

        // Per-CPU ready queues (simulating IrqSpinLock<ReadyQueues>).
        let queues: Vec<Arc<Mutex<ReadyQueues>>> = (0..NUM_CPUS)
            .map(|_| Arc::new(Mutex::new(ReadyQueues::new())))
            .collect();

        // Per-task remaining polls before completion.
        let remaining: Arc<Vec<AtomicU64>> = Arc::new(
            (0..NUM_TASKS)
                .map(|_| AtomicU64::new(POLLS_TO_COMPLETE))
                .collect(),
        );

        let completed = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));

        // Spawn all tasks on CPU 0.
        {
            let mut rq = queues[0].lock().unwrap();
            for i in 0..NUM_TASKS {
                rq.push(Priority::Normal, TaskId(i as u64));
            }
        }

        let handles: Vec<_> = (0..NUM_CPUS)
            .map(|cpu| {
                let queues = queues.clone();
                let remaining = remaining.clone();
                let completed = completed.clone();
                let done = done.clone();

                thread::spawn(move || {
                    // Simple xorshift PRNG per thread.
                    let mut rng: u64 = (cpu as u64 + 1).wrapping_mul(6364136223846793005);
                    let mut next_rand = || -> u64 {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        rng
                    };

                    while !done.load(Ordering::Relaxed) {
                        // --- poll_ready_tasks (with simulated preempt_pending) ---
                        loop {
                            let task = { queues[cpu].lock().unwrap().pop() };
                            match task {
                                None => break,
                                Some((pri, id)) => {
                                    let idx = id.0 as usize;
                                    let prev = remaining[idx].fetch_sub(1, Ordering::AcqRel);
                                    if prev <= 1 {
                                        // Task complete.
                                        completed.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        // Task yields → re-queue on current CPU.
                                        queues[cpu].lock().unwrap().push(pri, id);
                                    }

                                    // Simulated preempt_pending (20% chance).
                                    if next_rand() % 5 == 0 {
                                        break;
                                    }
                                }
                            }
                        }

                        // --- has_ready guard ---
                        if queues[cpu].lock().unwrap().has_ready() {
                            continue;
                        }

                        // --- try_steal ---
                        let start = (next_rand() as usize) % NUM_CPUS;
                        for i in 1..NUM_CPUS {
                            let victim = (start + i) % NUM_CPUS;
                            if victim == cpu {
                                continue;
                            }
                            if let Ok(mut rq) = queues[victim].try_lock() {
                                if let Some((pri, id)) = rq.steal_one() {
                                    drop(rq);
                                    queues[cpu].lock().unwrap().push(pri, id);
                                    break;
                                }
                            }
                        }

                        // Simulated HLT.
                        thread::yield_now();
                    }
                })
            })
            .collect();

        // Wait for all tasks to complete with timeout.
        let start = Instant::now();
        while completed.load(Ordering::Relaxed) < NUM_TASKS as u64 {
            if start.elapsed() > Duration::from_secs(10) {
                done.store(true, Ordering::Relaxed);
                for h in handles {
                    h.join().unwrap();
                }
                panic!(
                    "Livelock: only {}/{} tasks completed in 10s",
                    completed.load(Ordering::Relaxed),
                    NUM_TASKS
                );
            }
            thread::sleep(Duration::from_millis(10));
        }

        done.store(true, Ordering::Relaxed);
        for h in handles {
            h.join().unwrap();
        }
    }

    /// Stress test variant: all tasks start on a single CPU with aggressive
    /// stealing. Verifies load balancing happens AND completes.
    #[test]
    fn concurrent_work_distribution() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
        use std::sync::{Arc, Mutex};
        use std::thread;
        use std::time::{Duration, Instant};

        const NUM_CPUS: usize = 4;
        const NUM_TASKS: usize = 40;
        const POLLS_TO_COMPLETE: u64 = 100;

        let queues: Vec<Arc<Mutex<ReadyQueues>>> = (0..NUM_CPUS)
            .map(|_| Arc::new(Mutex::new(ReadyQueues::new())))
            .collect();

        let remaining: Arc<Vec<AtomicU64>> = Arc::new(
            (0..NUM_TASKS)
                .map(|_| AtomicU64::new(POLLS_TO_COMPLETE))
                .collect(),
        );

        let completed = Arc::new(AtomicU64::new(0));
        let done = Arc::new(AtomicBool::new(false));
        let per_cpu_polls: Arc<Vec<AtomicU64>> = Arc::new(
            (0..NUM_CPUS)
                .map(|_| AtomicU64::new(0))
                .collect(),
        );

        // All tasks start on CPU 0.
        {
            let mut rq = queues[0].lock().unwrap();
            for i in 0..NUM_TASKS {
                rq.push(Priority::Normal, TaskId(i as u64));
            }
        }

        let handles: Vec<_> = (0..NUM_CPUS)
            .map(|cpu| {
                let queues = queues.clone();
                let remaining = remaining.clone();
                let completed = completed.clone();
                let done = done.clone();
                let per_cpu_polls = per_cpu_polls.clone();

                thread::spawn(move || {
                    let mut rng: u64 = (cpu as u64 + 1).wrapping_mul(2862933555777941757);
                    let mut next_rand = || -> u64 {
                        rng ^= rng << 13;
                        rng ^= rng >> 7;
                        rng ^= rng << 17;
                        rng
                    };

                    while !done.load(Ordering::Relaxed) {
                        loop {
                            let task = { queues[cpu].lock().unwrap().pop() };
                            match task {
                                None => break,
                                Some((pri, id)) => {
                                    per_cpu_polls[cpu].fetch_add(1, Ordering::Relaxed);
                                    let idx = id.0 as usize;
                                    let prev = remaining[idx].fetch_sub(1, Ordering::AcqRel);
                                    if prev <= 1 {
                                        completed.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        queues[cpu].lock().unwrap().push(pri, id);
                                    }
                                    if next_rand() % 5 == 0 {
                                        break;
                                    }
                                }
                            }
                        }

                        if queues[cpu].lock().unwrap().has_ready() {
                            continue;
                        }

                        let start = (next_rand() as usize) % NUM_CPUS;
                        for i in 1..NUM_CPUS {
                            let victim = (start + i) % NUM_CPUS;
                            if victim == cpu {
                                continue;
                            }
                            if let Ok(mut rq) = queues[victim].try_lock() {
                                if let Some((pri, id)) = rq.steal_one() {
                                    drop(rq);
                                    queues[cpu].lock().unwrap().push(pri, id);
                                    break;
                                }
                            }
                        }

                        thread::yield_now();
                    }
                })
            })
            .collect();

        let start = Instant::now();
        while completed.load(Ordering::Relaxed) < NUM_TASKS as u64 {
            if start.elapsed() > Duration::from_secs(10) {
                done.store(true, Ordering::Relaxed);
                for h in handles {
                    h.join().unwrap();
                }
                panic!(
                    "Livelock: only {}/{} tasks completed in 10s",
                    completed.load(Ordering::Relaxed),
                    NUM_TASKS
                );
            }
            thread::sleep(Duration::from_millis(10));
        }

        done.store(true, Ordering::Relaxed);
        for h in handles {
            h.join().unwrap();
        }

        // Verify work was distributed across CPUs (not all on CPU 0).
        let polls: Vec<u64> = per_cpu_polls
            .iter()
            .map(|a| a.load(Ordering::Relaxed))
            .collect();
        let non_zero_cpus = polls.iter().filter(|&&p| p > 0).count();
        assert!(
            non_zero_cpus > 1,
            "Work should be distributed across CPUs, but only {} CPUs polled tasks: {:?}",
            non_zero_cpus,
            polls
        );
    }
}
