//! Persistent workers with a rolling, bounded ordered-delivery window.
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
pub struct SchedulerProfile {
    pub dex: usize,
    pub tasks: usize,
    pub capacity: usize,
    pub reorder_capacity: usize,
    pub wall: Duration,
    pub task_time: Duration,
    /// Time workers actually spent waiting for ordered-delivery admission.
    pub queue_wait: Duration,
    /// Worker lifetime minus task time and admission waits (startup/dispatch/exit).
    pub dispatch_idle: Duration,
    pub worker_time: Duration,
    /// Maximum completed-but-not-yet-delivered results retained for ordering.
    pub max_pending: usize,
}

const ADMISSION_WINDOW_PER_WORKER: usize = 256;

pub(super) fn run<T: Send, E: Send>(
    count: usize,
    workers: usize,
    task: impl Fn(usize) -> T + Sync,
    mut deliver: impl FnMut(usize, T) -> Result<(), E>,
    panic_error: impl Fn() -> E + Sync,
) -> (Result<(), E>, SchedulerProfile) {
    let workers = workers.max(1).min(count);
    // Preserve ordered delivery while letting workers pass a slow early class.
    // This bounds admitted class results, not their byte size.
    let window = workers.saturating_mul(ADMISSION_WINDOW_PER_WORKER).max(1);
    let start = Instant::now();
    let mut stats = SchedulerProfile {
        capacity: workers,
        reorder_capacity: window,
        ..Default::default()
    };
    let next = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let frontier = Mutex::new(0usize);
    let wake = Condvar::new();
    let result = std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();
        for _ in 0..workers {
            let tx = tx.clone();
            let (task, next, cancelled, frontier, wake, panic_error) =
                (&task, &next, &cancelled, &frontier, &wake, &panic_error);
            handles.push(scope.spawn(move || {
                let life = Instant::now();
                let mut active = Duration::ZERO;
                let mut waits = Duration::ZERO;
                let mut completed = 0;
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    if index >= count {
                        break;
                    }
                    let mut first = frontier.lock().unwrap_or_else(|e| e.into_inner());
                    if index >= first.saturating_add(window) {
                        let wait = Instant::now();
                        while index >= first.saturating_add(window)
                            && !cancelled.load(Ordering::Acquire)
                        {
                            first = wake.wait(first).unwrap_or_else(|e| e.into_inner());
                        }
                        waits += wait.elapsed();
                    }
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    drop(first);
                    let begin = Instant::now();
                    let value =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(index)))
                            .map_err(|_| panic_error());
                    active += begin.elapsed();
                    completed += 1;
                    let failed = value.is_err();
                    if tx.send((index, value)).is_err() {
                        break;
                    }
                    if failed {
                        break;
                    }
                }
                (life.elapsed(), active, waits, completed)
            }));
        }
        drop(tx);
        let mut pending = std::collections::BTreeMap::new();
        let mut first = 0;
        let result = (|| {
            while first < count {
                let (index, value) = rx.recv().map_err(|_| panic_error())?;
                // A panic aborts immediately, even if an earlier class is still running.
                let value = value?;
                pending.insert(index, value);
                stats.max_pending = stats.max_pending.max(pending.len());
                while let Some(value) = pending.remove(&first) {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        deliver(first, value)
                    }))
                    .map_err(|_| panic_error())??;
                    first += 1;
                    *frontier.lock().unwrap_or_else(|e| e.into_inner()) = first;
                    wake.notify_all();
                }
            }
            Ok(())
        })();
        // Take the admission mutex when cancelling so notification cannot be lost.
        let guard = frontier.lock().unwrap_or_else(|e| e.into_inner());
        cancelled.store(true, Ordering::Release);
        wake.notify_all();
        drop(guard);
        drop(rx);
        let mut result = result;
        for handle in handles {
            match handle.join() {
                Ok((life, active, waits, completed)) => {
                    stats.tasks += completed;
                    stats.worker_time += life;
                    stats.task_time += active;
                    stats.queue_wait += waits;
                    stats.dispatch_idle += life.saturating_sub(active + waits);
                }
                Err(_) => {
                    if result.is_ok() {
                        result = Err(panic_error());
                    }
                }
            }
        }
        result
    });
    stats.wall = start.elapsed();
    (result, stats)
}

/// Dispatch `task` over `order` (a permutation of `0..order.len()`) without any
/// ordered-delivery window: results are delivered the moment they finish, so a
/// slow ordinal never holds completed predecessors. Callers keep per-ordinal
/// state and reassemble logical order afterwards. An error or panic still stops
/// the run; delivery order is the only relaxation.
pub(super) fn run_unordered<T: Send, E: Send>(
    order: &[usize],
    workers: usize,
    task: impl Fn(usize) -> T + Sync,
    mut deliver: impl FnMut(usize, T) -> Result<(), E>,
    panic_error: impl Fn() -> E + Sync,
) -> (Result<(), E>, SchedulerProfile) {
    let count = order.len();
    let workers = workers.max(1).min(count.max(1));
    let start = Instant::now();
    let mut stats = SchedulerProfile {
        capacity: workers,
        ..Default::default()
    };
    let next = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let result = std::thread::scope(|scope| {
        let (tx, rx) = mpsc::channel();
        let mut handles = Vec::new();
        for _ in 0..workers {
            let tx = tx.clone();
            let (order, task, next, cancelled, panic_error) =
                (&order, &task, &next, &cancelled, &panic_error);
            handles.push(scope.spawn(move || {
                let life = Instant::now();
                let mut active = Duration::ZERO;
                let mut completed = 0;
                loop {
                    let slot = next.fetch_add(1, Ordering::Relaxed);
                    if slot >= count {
                        break;
                    }
                    if cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    let ordinal = order[slot];
                    let begin = Instant::now();
                    let value =
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(ordinal)))
                            .map_err(|_| panic_error());
                    active += begin.elapsed();
                    completed += 1;
                    let failed = value.is_err();
                    if tx.send((ordinal, value)).is_err() {
                        break;
                    }
                    if failed {
                        break;
                    }
                }
                (life.elapsed(), active, completed)
            }));
        }
        drop(tx);
        let result = (|| {
            for _ in 0..count {
                let (ordinal, value) = rx.recv().map_err(|_| panic_error())?;
                // A panic aborts immediately; delivery order is the only relaxation.
                let value = value?;
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| deliver(ordinal, value)))
                    .map_err(|_| panic_error())??;
            }
            Ok(())
        })();
        cancelled.store(true, Ordering::Release);
        drop(rx);
        let mut result = result;
        for handle in handles {
            match handle.join() {
                Ok((life, active, completed)) => {
                    stats.tasks += completed;
                    stats.worker_time += life;
                    stats.task_time += active;
                    stats.dispatch_idle += life.saturating_sub(active);
                }
                Err(_) => {
                    if result.is_ok() {
                        result = Err(panic_error());
                    }
                }
            }
        }
        result
    });
    stats.wall = start.elapsed();
    (result, stats)
}
