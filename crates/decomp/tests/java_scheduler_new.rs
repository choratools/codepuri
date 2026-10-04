//! Standalone persistent scheduler ordering, accounting, and cancellation checks.
#![allow(clippy::unwrap_used)]
#[path = "../src/java_scheduler.rs"]
#[allow(dead_code, missing_docs)] // Private production module compiled as a test fixture.
mod scheduler;

#[test]
fn one_and_thirty_preserve_order_and_metrics() {
    let run = |workers| {
        let mut output = Vec::new();
        let caller = std::thread::current().id();
        let (result, stats) = scheduler::run(
            1000,
            workers,
            |i| (i, i * 13),
            |i, value| {
                assert_eq!(std::thread::current().id(), caller);
                assert_eq!(value.0, i);
                output.push(value);
                Ok::<_, &'static str>(())
            },
            || "panic",
        );
        result.unwrap();
        assert_eq!(stats.tasks, 1000);
        assert_eq!(stats.capacity, workers);
        assert_eq!(
            stats.worker_time,
            stats.task_time + stats.queue_wait + stats.dispatch_idle
        );
        output
    };
    assert_eq!(run(1), run(30));
}

#[test]
fn skew_does_not_introduce_a_thirty_task_barrier_and_reorder_is_bounded() {
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    let gate = (Mutex::new(false), Condvar::new());
    let started = AtomicUsize::new(0);
    let expected_window = 2 * 256;
    let (result, stats) = scheduler::run(
        700,
        2,
        |i| {
            started.fetch_max(i + 1, Ordering::SeqCst);
            if i == 0 {
                let (lock, wake) = &gate;
                let guard = lock.lock().unwrap();
                let (guard, timeout) = wake
                    .wait_timeout_while(guard, std::time::Duration::from_secs(3), |v| !*v)
                    .unwrap();
                assert!(
                    *guard && !timeout.timed_out(),
                    "scheduler retained a batch barrier or exceeded the admission bound"
                );
            }
            if i == expected_window - 1 {
                *gate.0.lock().unwrap() = true;
                gate.1.notify_one();
            }
            i
        },
        |i, value| {
            assert_eq!(i, value);
            assert!(started.load(Ordering::SeqCst) <= i + expected_window);
            Ok::<_, &'static str>(())
        },
        || "panic",
    );
    result.unwrap();
    assert_eq!(stats.tasks, 700);
    assert_eq!(stats.reorder_capacity, expected_window);
}

#[test]
fn sink_failure_counts_completed_tasks_separately_from_delivery() {
    let gate = std::sync::Barrier::new(30);
    let mut delivered = 0;
    let (result, stats) = scheduler::run(
        1000,
        30,
        |i| {
            if i < 30 {
                gate.wait();
            }
            i
        },
        |i, value| {
            assert_eq!((i, value), (0, 0));
            delivered += 1;
            Err::<(), _>("sink failure")
        },
        || "panic",
    );
    assert_eq!(result, Err("sink failure"));
    assert_eq!(delivered, 1);
    assert!(stats.tasks >= 30);
    assert!(stats.tasks <= stats.reorder_capacity);
    assert_eq!(
        stats.worker_time,
        stats.task_time + stats.queue_wait + stats.dispatch_idle
    );
}

#[test]
fn failures_and_panics_cancel_waiting_workers() {
    for mode in 0..3 {
        let (result, stats) = scheduler::run(
            10000,
            30,
            |i| {
                if mode == 1 && i == 3 {
                    panic!("task failure");
                }
                i
            },
            |_, _| {
                if mode == 2 {
                    panic!("callback failure");
                }
                Err::<(), _>("sink failure")
            },
            || "panic",
        );
        assert!(result.is_err());
        assert!(stats.tasks <= stats.reorder_capacity);
    }
    let (result, stats) = scheduler::run(0, 30, |_| (), |_, _| Ok::<_, ()>(()), || ());
    assert!(result.is_ok());
    assert_eq!(stats.tasks, 0);
}

#[test]
fn expanded_window_preserves_output_and_admits_large_frontier_skew() {
    use std::sync::{
        Condvar, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use std::time::Duration;

    const COUNT: usize = 1200;
    const WORKERS: usize = 30;
    const EXPECTED_WINDOW: usize = WORKERS * 256;

    let run = || {
        let completed_non_frontier = AtomicUsize::new(0);
        let gate = (Mutex::new(false), Condvar::new());
        let mut output = Vec::new();
        let (result, stats) = scheduler::run(
            COUNT,
            WORKERS,
            |i| {
                if i == 0 {
                    let (lock, wake) = &gate;
                    let guard = lock.lock().unwrap();
                    let (guard, timeout) = wake
                        .wait_timeout_while(guard, Duration::from_secs(3), |v| !*v)
                        .unwrap();
                    assert!(
                        *guard && !timeout.timed_out(),
                        "expanded window did not admit all non-frontier work"
                    );
                } else if completed_non_frontier.fetch_add(1, Ordering::SeqCst) + 1 == COUNT - 1 {
                    *gate.0.lock().unwrap() = true;
                    gate.1.notify_one();
                }
                (i, i.wrapping_mul(17))
            },
            |i, value| {
                assert_eq!(value.0, i);
                output.push(value);
                Ok::<_, &'static str>(())
            },
            || "panic",
        );
        result.unwrap();
        (output, stats)
    };

    let expected = (0..COUNT)
        .map(|i| (i, i.wrapping_mul(17)))
        .collect::<Vec<_>>();
    let (parallel, many) = run();
    assert_eq!(parallel, expected);
    assert_eq!(many.reorder_capacity, EXPECTED_WINDOW);
    assert!(many.max_pending <= many.reorder_capacity);
    // At most WORKERS-1 completed tasks can still be between the work closure
    // and sending their result when the frontier wakes. Do not assume that the
    // closure's completion signal also orders channel delivery from every worker.
    assert!(many.max_pending >= COUNT - WORKERS);
    assert_eq!(
        many.worker_time,
        many.task_time + many.queue_wait + many.dispatch_idle
    );
}
