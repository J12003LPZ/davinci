//! Tool scheduling: which calls in one assistant message may overlap, and
//! the fan-out/fan-in that runs them.
//!
//! Mirrors `executeToolCallsParallel` in
//! `vendor/pi/packages/agent/src/agent-loop.ts` (real concurrency via
//! `Promise.all`, results finalized in source order), with one refinement
//! the TypeScript runtime leaves to each tool's `executionMode`: calls are
//! placed in a *lane*. Read-only calls (`read`, `grep`, `find`, `ls`, the
//! web tools, `mcp_read`, read-only MCP tools) share the parallel lane and
//! overlap. `agent` calls use per-call routing so writable workers in the
//! shared checkout become barriers. Anything that mutates or has unknown
//! side effects (`write`, `edit`, shell commands, extension tools) is a
//! barrier that runs alone, after everything before it has finished and
//! before anything after it starts. Source order still means what the model
//! thinks it means (`edit A` then `read A` sees the edit) while a burst of
//! independent reads costs one round of latency instead of N.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use crate::permission::ToolClass;
use crate::runtime::{ConcurrencyPolicy, RuntimeCapability};

/// How many tool calls of one message run at once. Matches the reference
/// subagent extension's `MAX_CONCURRENCY`-style cap: enough to hide I/O
/// latency, not enough to thrash a laptop or an MCP server.
pub const MAX_TOOL_PARALLELISM: usize = 8;

/// Where a call runs relative to its neighbours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLane {
    /// May overlap with adjacent `Parallel` calls.
    Parallel,
    /// Runs alone; a barrier for the calls on either side.
    Serial,
}

/// The lane a tool belongs to, given the class the permission policy
/// assigned it (`PermissionPolicy::class_of`, which knows about MCP
/// `readOnlyHint`).
pub fn lane_for(tool: &str, class: ToolClass) -> ToolLane {
    match tool {
        // Agent::lane_for_call makes writable shared-checkout workers serial.
        "agent" => ToolLane::Parallel,
        // A batch is a barrier: it schedules its own operations, and it
        // runs on the calling thread so the permission approver is asked
        // from one place at a time.
        "batch" => ToolLane::Serial,
        // The ledger and the job book are shared state behind a mutex, but
        // two `todo` writes in one message would race for last-wins; keep
        // them ordered.
        "todo" | "update_plan" | "propose_plan" | "ask_user_question" | "job_kill"
        | "agent_message" | "agent_stop" | "task_create" | "task_update" => ToolLane::Serial,
        _ => match class {
            ToolClass::Read | ToolClass::Network => ToolLane::Parallel,
            ToolClass::Edit | ToolClass::Shell | ToolClass::Other => ToolLane::Serial,
        },
    }
}

/// Resolve a scheduler lane from an authoritative runtime capability.
/// Missing metadata fails closed; callers without a runtime registry can keep
/// using [`lane_for`] for the legacy class-based behavior.
pub fn lane_for_capability(
    capability: Option<&RuntimeCapability>,
    _tool: &str,
    _class: ToolClass,
) -> ToolLane {
    match capability.map(|capability| capability.concurrency_policy) {
        Some(ConcurrencyPolicy::ParallelSafe) => ToolLane::Parallel,
        Some(ConcurrencyPolicy::SerialBarrier) => ToolLane::Serial,
        // A runtime registry is authoritative when it is installed. A missing
        // capability therefore cannot inherit the legacy class-based
        // parallel lane and must fail closed.
        None => ToolLane::Serial,
    }
}

/// One unit of work handed to the scheduler.
pub struct ScheduledCall<'a, T> {
    pub lane: ToolLane,
    /// The work itself. Runs on a worker thread when the call is in a
    /// parallel group of two or more; otherwise on the caller's thread.
    pub run: Box<dyn FnOnce() -> T + Send + 'a>,
}

/// What the scheduler tells the caller about how a batch ran.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScheduleReport {
    /// Groups of two or more calls that actually overlapped.
    pub parallel_groups: usize,
    /// The widest group that overlapped.
    pub max_group_width: usize,
    /// Calls that were never started because `abort` was set.
    pub skipped: usize,
}

/// Run `calls` honouring lanes, and return their results in source order.
///
/// Consecutive `Parallel` calls form a group that runs on up to
/// `max_parallelism` threads; a `Serial` call runs alone between groups.
/// `abort` is checked before every dispatch: once it is set, nothing more is
/// started and the result vector ends early (the caller reports the
/// missing calls the way it reports an interrupted sequential run).
/// `sequential` forces every group to width one, which is what
/// `ToolExecutionMode::Sequential` means.
pub fn run_lanes<T: Send>(
    calls: Vec<ScheduledCall<'_, T>>,
    sequential: bool,
    max_parallelism: usize,
    abort: Option<&AtomicBool>,
    on_group_start: impl FnMut(&[usize]),
) -> (Vec<T>, ScheduleReport) {
    run_lanes_with_cancel(
        calls,
        sequential,
        max_parallelism,
        || abort.is_some_and(|flag| flag.load(Ordering::SeqCst)),
        on_group_start,
    )
}

/// Check the host's combined cancellation sources before dispatching queued work.
pub(crate) fn run_lanes_with_cancel<'a, T: Send>(
    calls: Vec<ScheduledCall<'a, T>>,
    sequential: bool,
    max_parallelism: usize,
    aborted: impl Fn() -> bool,
    mut on_group_start: impl FnMut(&[usize]),
) -> (Vec<T>, ScheduleReport) {
    let mut results: Vec<T> = Vec::with_capacity(calls.len());
    let mut report = ScheduleReport::default();
    let total = calls.len();
    let mut pending = calls.into_iter().enumerate().peekable();

    while pending.peek().is_some() {
        if aborted() {
            break;
        }
        // Take the next group: one serial call, or a run of parallel calls.
        let mut group: Vec<(usize, ScheduledCall<'a, T>)> = Vec::new();
        let width_cap = if sequential {
            1
        } else {
            max_parallelism.max(1)
        };
        while let Some((_, call)) = pending.peek() {
            let lane = call.lane;
            if group.is_empty() {
                group.push(pending.next().expect("peeked"));
                if lane == ToolLane::Serial {
                    break;
                }
                continue;
            }
            if lane == ToolLane::Serial || sequential {
                break;
            }
            group.push(pending.next().expect("peeked"));
        }
        if group.len() == 1 {
            let (index, call) = group.pop().expect("one");
            on_group_start(&[index]);
            results.push((call.run)());
            continue;
        }
        let active_width = group.len().min(width_cap);
        if active_width > 1 {
            report.parallel_groups += 1;
            report.max_group_width = report.max_group_width.max(active_width);
        }
        results.extend(run_group(group, width_cap, &aborted, &mut on_group_start));
    }
    report.skipped = total.saturating_sub(results.len());
    (results, report)
}

/// A bounded worker pool covers the entire contiguous safe region. The
/// coordinator owns admission, callbacks and cancellation; workers only run
/// admitted calls. A completed read frees its slot without waiting for slower
/// neighbours. Results remain a source-ordered prefix, even on cancellation.
fn run_group<T: Send>(
    group: Vec<(usize, ScheduledCall<'_, T>)>,
    max_parallelism: usize,
    aborted: &impl Fn() -> bool,
    on_start: &mut impl FnMut(&[usize]),
) -> Vec<T> {
    let count = group.len();
    let width = max_parallelism.min(count);
    let (tasks, pending) = std::sync::mpsc::sync_channel::<(usize, ScheduledCall<'_, T>)>(width);
    let pending = Mutex::new(pending);
    let (finished, completions) = std::sync::mpsc::channel();
    let mut slots: Vec<Option<T>> = (0..count).map(|_| None).collect();
    let mut panic = None;
    let mut started = 0;
    std::thread::scope(|scope| {
        for _ in 0..width {
            let pending = &pending;
            let finished = finished.clone();
            scope.spawn(move || loop {
                let next = pending.lock().unwrap_or_else(|err| err.into_inner()).recv();
                let Ok((index, call)) = next else { break };
                let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(call.run));
                if finished.send((index, value)).is_err() {
                    break;
                }
            });
        }
        drop(finished);
        let mut calls = group.into_iter().enumerate();
        let mut active = 0;
        let mut cancelled = false;
        loop {
            while active < width && panic.is_none() && !cancelled {
                cancelled = aborted();
                if cancelled {
                    break;
                }
                let Some((slot, (index, call))) = calls.next() else {
                    break;
                };
                on_start(&[index]);
                tasks
                    .send((slot, call))
                    .expect("workers are scoped to this region");
                started += 1;
                active += 1;
            }
            if active == 0 {
                break;
            }
            let (slot, result) = completions
                .recv()
                .expect("every admitted call reports completion");
            active -= 1;
            match result {
                Ok(value) => slots[slot] = Some(value),
                Err(error) => {
                    panic.get_or_insert(error);
                }
            }
        }
        drop(tasks);
    });
    if let Some(error) = panic {
        std::panic::resume_unwind(error);
    }
    slots
        .into_iter()
        .take(started)
        .map(|slot| slot.expect("every scheduled call produced a result"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn harness_refills_a_free_slot_before_the_slow_read_finishes() {
        let (release, wait) = std::sync::mpsc::channel();
        let calls = vec![
            ScheduledCall {
                lane: ToolLane::Parallel,
                run: Box::new(move || {
                    wait.recv_timeout(Duration::from_secs(2))
                        .expect("third read must start while the first is still running");
                    0
                }),
            },
            ScheduledCall {
                lane: ToolLane::Parallel,
                run: Box::new(|| 1),
            },
            ScheduledCall {
                lane: ToolLane::Parallel,
                run: Box::new(move || {
                    release.send(()).unwrap();
                    2
                }),
            },
        ];
        let (values, report) = run_lanes(calls, false, 2, None, |_| {});
        assert_eq!(values, [0, 1, 2]);
        assert_eq!(report.max_group_width, 2);
    }

    #[test]
    fn harness_cancellation_stops_queued_reads_and_barriers() {
        let flag = AtomicBool::new(false);
        let calls = (0..5)
            .map(|index| ScheduledCall {
                lane: if index == 4 {
                    ToolLane::Serial
                } else {
                    ToolLane::Parallel
                },
                run: Box::new(|| {
                    flag.store(true, Ordering::SeqCst);
                    1
                }),
            })
            .collect();
        let mut starts = Vec::new();
        let (values, report) = run_lanes(calls, false, 2, Some(&flag), |group| {
            starts.extend_from_slice(group)
        });
        assert!(values.len() <= 2);
        assert_eq!(starts.len(), values.len());
        assert_eq!(report.skipped, 5 - values.len());
    }

    fn sleeper(lane: ToolLane, ms: u64, tag: &'static str) -> ScheduledCall<'static, &'static str> {
        ScheduledCall {
            lane,
            run: Box::new(move || {
                std::thread::sleep(Duration::from_millis(ms));
                tag
            }),
        }
    }

    #[test]
    fn parallel_reads_overlap_and_keep_source_order() {
        let calls = vec![
            sleeper(ToolLane::Parallel, 120, "a"),
            sleeper(ToolLane::Parallel, 20, "b"),
            sleeper(ToolLane::Parallel, 60, "c"),
        ];
        let start = Instant::now();
        let (results, report) = run_lanes(calls, false, 8, None, |_| {});
        let elapsed = start.elapsed();
        assert_eq!(results, vec!["a", "b", "c"]);
        assert_eq!(report.parallel_groups, 1);
        assert_eq!(report.max_group_width, 3);
        assert!(
            elapsed < Duration::from_millis(200),
            "three sleeps of 120/20/60 ms took {elapsed:?}; they did not overlap"
        );
    }

    #[test]
    fn sequential_mode_never_overlaps() {
        let calls = vec![
            sleeper(ToolLane::Parallel, 40, "a"),
            sleeper(ToolLane::Parallel, 40, "b"),
        ];
        let start = Instant::now();
        let (results, report) = run_lanes(calls, true, 8, None, |_| {});
        assert_eq!(results, vec!["a", "b"]);
        assert_eq!(report.parallel_groups, 0);
        assert!(start.elapsed() >= Duration::from_millis(80));
    }

    #[test]
    fn a_serial_call_is_a_barrier_between_parallel_groups() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let record = |tag: &'static str, ms: u64, lane: ToolLane| {
            let order = Arc::clone(&order);
            ScheduledCall {
                lane,
                run: Box::new(move || {
                    order.lock().unwrap().push(format!("{tag}:start"));
                    std::thread::sleep(Duration::from_millis(ms));
                    order.lock().unwrap().push(format!("{tag}:end"));
                    tag
                }),
            }
        };
        let calls = vec![
            record("r1", 30, ToolLane::Parallel),
            record("r2", 30, ToolLane::Parallel),
            record("w", 10, ToolLane::Serial),
            record("r3", 10, ToolLane::Parallel),
        ];
        let mut groups = Vec::new();
        let (results, report) = run_lanes(calls, false, 8, None, |g| groups.push(g.to_vec()));
        assert_eq!(results, vec!["r1", "r2", "w", "r3"]);
        assert_eq!(groups, vec![vec![0], vec![1], vec![2], vec![3]]);
        assert_eq!(report.parallel_groups, 1);
        let order = order.lock().unwrap().clone();
        let position = |tag: &str| order.iter().position(|item| item == tag).unwrap();
        // The write starts only after both reads ended, and the trailing
        // read starts only after the write ended.
        assert!(position("w:start") > position("r1:end"));
        assert!(position("w:start") > position("r2:end"));
        assert!(position("r3:start") > position("w:end"));
    }

    #[test]
    fn width_is_capped() {
        let calls: Vec<_> = (0..5)
            .map(|_| sleeper(ToolLane::Parallel, 5, "x"))
            .collect();
        let mut groups = Vec::new();
        let (results, report) = run_lanes(calls, false, 2, None, |g| groups.push(g.len()));
        assert_eq!(results.len(), 5);
        assert_eq!(groups, vec![1; 5]);
        assert_eq!(report.max_group_width, 2);
    }

    #[test]
    fn an_abort_stops_before_the_next_group() {
        let flag = Arc::new(AtomicBool::new(false));
        let setter = Arc::clone(&flag);
        let calls = vec![
            ScheduledCall {
                lane: ToolLane::Serial,
                run: Box::new(move || {
                    setter.store(true, Ordering::SeqCst);
                    "first"
                }),
            },
            sleeper(ToolLane::Parallel, 1, "never"),
        ];
        let (results, report) = run_lanes(calls, false, 8, Some(&flag), |_| {});
        assert_eq!(results, vec!["first"]);
        assert_eq!(report.skipped, 1);
    }

    #[test]
    fn lanes_follow_the_permission_class() {
        assert_eq!(lane_for("read", ToolClass::Read), ToolLane::Parallel);
        assert_eq!(
            lane_for("web_fetch", ToolClass::Network),
            ToolLane::Parallel
        );
        assert_eq!(
            lane_for("mcp__memory__echo", ToolClass::Read),
            ToolLane::Parallel
        );
        assert_eq!(
            lane_for("mcp__memory__store", ToolClass::Other),
            ToolLane::Serial
        );
        assert_eq!(lane_for("edit", ToolClass::Edit), ToolLane::Serial);
        assert_eq!(lane_for("bash", ToolClass::Shell), ToolLane::Serial);
        assert_eq!(lane_for("agent", ToolClass::Other), ToolLane::Parallel);
        assert_eq!(lane_for("batch", ToolClass::Read), ToolLane::Serial);
        assert_eq!(lane_for("todo", ToolClass::Read), ToolLane::Serial);
        assert_eq!(
            lane_for("ask_user_question", ToolClass::Read),
            ToolLane::Serial
        );
    }
}
