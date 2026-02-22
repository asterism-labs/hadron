//! Scheduler and executor tests — async tasks, priorities, and work stealing.

extern crate alloc;

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use hadron_ktest::kernel_test;

// ── Migrated from sample tests ──────────────────────────────────────────

#[kernel_test(stage = "with_executor")]
async fn test_async_yield() {
    crate::sched::primitives::yield_now().await;
}

#[kernel_test(stage = "with_executor", instances = 0..=1)]
async fn test_barrier_sync(ctx: &hadron_ktest::TestContext) {
    ctx.barrier().await;
}

// ── New: spawn and complete ─────────────────────────────────────────────

#[kernel_test(stage = "with_executor")]
async fn test_spawn_and_complete() {
    static COMPLETED: AtomicBool = AtomicBool::new(false);
    COMPLETED.store(false, Ordering::Release);

    crate::sched::spawn(async {
        COMPLETED.store(true, Ordering::SeqCst);
    });

    // Yield many times to give the spawned task a chance to run.
    for _ in 0..10_000 {
        if COMPLETED.load(Ordering::SeqCst) {
            return;
        }
        crate::sched::primitives::yield_now().await;
    }
    assert!(
        COMPLETED.load(Ordering::SeqCst),
        "spawned task did not complete after 10,000 yields"
    );
}

#[kernel_test(stage = "with_executor")]
async fn test_spawn_multiple_tasks() {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    COUNTER.store(0, Ordering::Release);

    for _ in 0..10 {
        crate::sched::spawn(async {
            COUNTER.fetch_add(1, Ordering::SeqCst);
        });
    }

    // Yield many times to give spawned tasks a chance to run.
    for _ in 0..10_000 {
        if COUNTER.load(Ordering::SeqCst) >= 10 {
            return;
        }
        crate::sched::primitives::yield_now().await;
    }
    assert_eq!(
        COUNTER.load(Ordering::SeqCst),
        10,
        "all 10 spawned tasks should have completed"
    );
}

// ── Priority ordering ───────────────────────────────────────────────────

#[kernel_test(stage = "with_executor")]
async fn test_priority_ordering() {
    use crate::sched::{Priority, TaskMeta};

    static SEQUENCE: AtomicU32 = AtomicU32::new(0);
    static NORMAL_SEQ: AtomicU32 = AtomicU32::new(u32::MAX);
    static BG_SEQ: AtomicU32 = AtomicU32::new(u32::MAX);
    SEQUENCE.store(0, Ordering::Release);
    NORMAL_SEQ.store(u32::MAX, Ordering::Release);
    BG_SEQ.store(u32::MAX, Ordering::Release);

    // Spawn Background first, then Normal — Normal should run first.
    crate::sched::spawn_with(
        async {
            BG_SEQ.store(SEQUENCE.fetch_add(1, Ordering::AcqRel), Ordering::Release);
        },
        TaskMeta::new("bg-test").with_priority(Priority::Background),
    );
    crate::sched::spawn_with(
        async {
            NORMAL_SEQ.store(SEQUENCE.fetch_add(1, Ordering::AcqRel), Ordering::Release);
        },
        TaskMeta::new("normal-test").with_priority(Priority::Normal),
    );

    // Yield until both spawned tasks record their sequence numbers.
    for _ in 0..10_000 {
        let normal = NORMAL_SEQ.load(Ordering::SeqCst);
        let bg = BG_SEQ.load(Ordering::SeqCst);
        if normal != u32::MAX && bg != u32::MAX {
            break;
        }
        crate::sched::primitives::yield_now().await;
    }

    let normal = NORMAL_SEQ.load(Ordering::Acquire);
    let bg = BG_SEQ.load(Ordering::Acquire);
    assert!(
        normal < bg,
        "Normal ({normal}) should complete before Background ({bg})"
    );
}

// ── Sleep ticks ─────────────────────────────────────────────────────────

#[kernel_test(stage = "with_executor")]
async fn test_sleep_ticks() {
    let before = crate::time::timer_ticks();
    crate::sched::primitives::sleep_ticks(5).await;
    let after = crate::time::timer_ticks();
    assert!(
        after >= before + 5,
        "expected at least 5 ticks elapsed, got {} -> {} (delta {})",
        before,
        after,
        after.saturating_sub(before)
    );
}

// ── Instanced barrier (4 instances) ────────────────────────────────────

#[kernel_test(stage = "with_executor", instances = 0..=3)]
async fn test_concurrent_barrier_four(ctx: &hadron_ktest::TestContext) {
    ctx.barrier().await;
}

// ── Yield fairness ──────────────────────────────────────────────────────

#[kernel_test(stage = "with_executor", instances = 0..=1)]
async fn test_yield_fairness(ctx: &hadron_ktest::TestContext) {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    // Reset counter when instance 0 starts.
    if ctx.instance_id == 0 {
        COUNTER.store(0, Ordering::Release);
    }
    ctx.barrier().await;

    // Each instance yields 50 times, incrementing the shared counter.
    for _ in 0..50 {
        COUNTER.fetch_add(1, Ordering::AcqRel);
        crate::sched::primitives::yield_now().await;
    }

    // Wait until both instances have completed all 50 increments.
    // (The AsyncBarrier's cumulative count makes a second barrier()
    // call a no-op, so we spin-wait on the counter instead.)
    if ctx.instance_id == 0 {
        for _ in 0..10_000 {
            if COUNTER.load(Ordering::Acquire) >= 100 {
                return;
            }
            crate::sched::primitives::yield_now().await;
        }
        let total = COUNTER.load(Ordering::Acquire);
        assert_eq!(total, 100, "expected 100 increments from 2 instances, got {total}");
    }
}

// ── Work stealing tests ─────────────────────────────────────────────────

#[kernel_test(stage = "with_executor")]
async fn test_steal_from_executor() {
    use crate::sched::{Executor, TaskMeta, Priority};

    let victim = Executor::new();
    // Spawn 2 tasks so steal_one doesn't trigger the one-task rule.
    victim.spawn_with_meta(async {}, TaskMeta::new("a").with_priority(Priority::Normal));
    victim.spawn_with_meta(async {}, TaskMeta::new("b").with_priority(Priority::Normal));

    let stolen = victim.steal_task();
    assert!(
        stolen.is_some(),
        "should be able to steal a task from executor with 2 tasks"
    );
}

#[kernel_test(stage = "with_executor")]
async fn test_steal_respects_critical() {
    use crate::sched::{Executor, TaskMeta, Priority};

    let victim = Executor::new();
    // Spawn only Critical tasks — none should be stealable.
    victim.spawn_with_meta(async {}, TaskMeta::new("crit1").with_priority(Priority::Critical));
    victim.spawn_with_meta(async {}, TaskMeta::new("crit2").with_priority(Priority::Critical));

    let stolen = victim.steal_task();
    assert!(
        stolen.is_none(),
        "Critical tasks should not be stealable"
    );
}

#[kernel_test(stage = "with_executor")]
async fn test_steal_one_task_rule() {
    use crate::sched::{Executor, TaskMeta, Priority};

    let victim = Executor::new();
    // Spawn exactly 1 Normal task — one-task rule prevents stealing.
    victim.spawn_with_meta(async {}, TaskMeta::new("only").with_priority(Priority::Normal));

    let stolen = victim.steal_task();
    assert!(
        stolen.is_none(),
        "one-task rule should prevent stealing the only runnable task"
    );
}

#[kernel_test(stage = "with_executor")]
async fn test_steal_allowed_with_two() {
    use crate::sched::{Executor, TaskMeta, Priority};

    let victim = Executor::new();
    victim.spawn_with_meta(async {}, TaskMeta::new("t1").with_priority(Priority::Normal));
    victim.spawn_with_meta(async {}, TaskMeta::new("t2").with_priority(Priority::Normal));

    let stolen = victim.steal_task();
    assert!(
        stolen.is_some(),
        "should be able to steal when there are 2 Normal tasks"
    );
}
