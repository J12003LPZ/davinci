//! Tool scheduling: which calls in one assistant message may overlap, and
//! the fan-out/fan-in that runs them.
//!
//! Mirrors `executeToolCallsParallel` in
//! `vendor/pi/packages/agent/src/agent-loop.ts` (real concurrency via
//! `Promise.all`, results finalized in source order), with one refinement
//! the TypeScript runtime leaves to each tool's `executionMode`: calls are
//! placed in a *lane*. Read-only calls (`read`, `grep`, `find`, `ls`, the
//! web tools, `mcp_read`, read-only MCP tools, `agent`) share the parallel
//! lane and overlap; anything that mutates or has unknown side effects
//! (`write`, `edit`, shell commands, extension tools) is a barrier that
//! runs alone, after everything before it has finished and before anything
//! after it starts. Source order therefore still means what the model
//! thinks it means (`edit A` then `read A` sees the edit) while a burst of
//! independent reads costs one round of latency instead of N.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::permission::ToolClass;

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
        // A worker is read-only by construction; several may search at once.
        "agent" => ToolLane::Parallel,
        // A batch is a barrier: it schedules its own operations, and it
        // runs on the calling thread so the permission approver is asked
        // from one place at a time.
        "batch" => ToolLane::Serial,
        // The ledger and the job book are shared state behind a mutex, but
        // two `todo` writes in one message would race for last-wins; keep
        // them ordered.
        "todo" | "job_kill" => ToolLane::Serial,
        _ => match class {
            ToolClass::Read | ToolClass::Network => ToolLane::Parallel,
            ToolClass::Edit | ToolClass::Shell | ToolClass::Other => ToolLane::Serial,
        },
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
/// `abort` is checked before every group: once it is set, nothing more is
/// started and the result vector ends early (the caller reports the
/// missing calls the way it reports an interrupted sequential run).
/// `sequential` forces every group to width one, which is what
/// `ToolExecutionMode::Sequential` means.
pub fn run_lanes<'a, T: Send>(
    calls: Vec<ScheduledCall<'a, T>>,
    sequential: bool,
    max_parallelism: usize,
    abort: Option<&AtomicBool>,
    mut on_group_start: impl FnMut(&[usize]),
) -> (Vec<T>, ScheduleReport) {
    let mut results: Vec<T> = Vec::with_capacity(calls.len());
    let mut report = ScheduleReport::default();
    let total = calls.len();
    let mut pending = calls.into_iter().enumerate().peekable();
    let aborted = || abort.is_some_and(|flag| flag.load(Ordering::SeqCst));

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
            if lane == ToolLane::Serial || group.len() >= width_cap {
                break;
            }
            group.push(pending.next().expect("peeked"));
        }
        let indices: Vec<usize> = group.iter().map(|(index, _)| *index).collect();
        on_group_start(&indices);
        if group.len() == 1 {
            let (_, call) = group.pop().expect("one");
            results.push((call.run)());
            continue;
        }
        report.parallel_groups += 1;
        report.max_group_width = report.max_group_width.max(group.len());
        results.extend(run_group(group.into_iter().map(|(_, call)| call).collect()));
    }
    report.skipped = total.saturating_sub(results.len());
    (results, report)
}

/// Fan a group out over scoped threads and fan the results back in, in the
/// group's own order. One thread per call: the caller already capped the
/// width, and a scoped thread costs less than the I/O it hides.
fn run_group<T: Send>(group: Vec<ScheduledCall<'_, T>>) -> Vec<T> {
    let count = group.len();
    let slots: Mutex<Vec<Option<T>>> = Mutex::new((0..count).map(|_| None).collect());
    let queue = Arc::new(Mutex::new(
        group
            .into_iter()
            .enumerate()
            .map(|(index, call)| (index, call.run))
            .collect::<Vec<_>>(),
    ));
    std::thread::scope(|scope| {
        for _ in 0..count {
            let queue = Arc::clone(&queue);
            let slots = &slots;
            scope.spawn(move || loop {
                let next = queue.lock().unwrap_or_else(|err| err.into_inner()).pop();
                let Some((index, run)) = next else {
                    break;
                };
                let value = run();
                slots.lock().unwrap_or_else(|err| err.into_inner())[index] = Some(value);
            });
        }
    });
    slots
        .into_inner()
        .unwrap_or_else(|err| err.into_inner())
        .into_iter()
        .map(|slot| slot.expect("every scheduled call produced a result"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

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
        assert_eq!(groups, vec![vec![0, 1], vec![2], vec![3]]);
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
        assert_eq!(groups, vec![2, 2, 1]);
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
    }
}
