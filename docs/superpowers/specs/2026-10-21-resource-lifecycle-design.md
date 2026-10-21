# DaVinci resource lifecycle improvements

Status: approved by the user on 2026-10-21; implementation awaits plan review.

Base: remote main `d44028fbe822cd71975e91ceda69c7c901a82693`, checked 2026-10-21.

## Intended outcome

Implement the user's performance review and deliver one pull request from a feature branch. Reduce retained infrastructure and bound subprocess/rendering memory without changing model selection, reasoning effort, prompt semantics, permissions, verification, complete session history, or recoverable tool evidence. Main must not receive a direct push.

Source inspection establishes opportunities, not measured leaks or speedups. Preserve existing concurrency and cache defaults until a comparable measurement justifies changing them. Offer the review's four-workflow/two-Graph-worker and smaller-cache values as an optional resource profile. Preserve explicit user overrides.

## Verified baseline and corrections

| Concern | Source at base commit | Finding |
| --- | --- | --- |
| Workflow concurrency | `crates/davinci-agent/src/runtime/workflow/limits.rs`, `WorkflowSettings::resolve`, `DEFAULT_MAX_CONCURRENT_AGENTS` | Default 16; setting and environment override already exist. |
| One-shot concurrency | `crates/davinci-agent/src/subagent.rs`, `MAX_PARALLEL_TASKS`, `MAX_TASK_CONCURRENCY`, scheduled parallel execution | Eight tasks per call, four executing concurrently; concurrency is a constant. |
| Worker construction | `crates/davinci-coding-agent/src/main.rs`, `build_worker_agent`, `run_nested_subagent`; `src/native_extensions/mod.rs`, `NativeExtensionHost::new_with_agent_dir` | Worker settings are loaded again; native host assembles many services and invokes stale-output sweeping. Constructor presence alone does not establish expensive eager work for every service. |
| Graph process launch | `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`; `graph/types.rs`, budget defaults | Separate executable; default parallel-worker budget three. Preserve process isolation and authenticated parent authority. |
| Graph output | `crates/davinci-coding-agent/src/native_extensions/graph/process.rs`, `pump`, `run_child_with_deadline` | Unbounded mpsc channel and `read_until` buffer; final drain additionally collects pending lines in a Vec. All three allocations need bounds. |
| Settled transcript | `crates/davinci-tui/src/davinci/views/transcript.rs`, `SettledCache`, `conversation_rows` | Settled rendered rows retained as `Rc<Vec<Line>>`; viewport slicing only bounds copying, not retention. |
| JS processes/metadata | `crates/davinci-coding-agent/src/js_host.rs`, `PersistentJsPool`, `ensure_capacity`, `LOAD_CACHE` | Pool ceiling eight, LRU removal, no idle timeout; load-reply BTreeMap has no explicit entry/byte ceiling. |
| Local cache | `crates/davinci-agent/src/runtime/cache/memory.rs`, `CacheRuntime::shared`, `insert_memory` | In-process hosts already share compatible cache instances through Weak references. Defaults 64 MiB memory/512 MiB persistent; eviction scans access sequence numbers. |
| Governor maintenance | `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`, `TokenGovernor::bind_session`, `sweep_stale_outputs`; native host constructor | Host construction and session rebind call sweeping. Library constructor itself does not delete outputs. |
| Language servers | `crates/davinci-coding-agent/src/native_extensions/language_intelligence/manager.rs`, `new`, `fork`, `slot_for`; `config.rs` | Already demand-created; forked handles share session storage with per-caller permissions and workspace keys; capacity eviction exists. Config already has `idle_timeout_ms` default 300,000. Verify every call site before altering idle lifecycle. |

All baseline findings above are source-verified. Performance effect remains unknown until measured.

## Options and selected approach

1. Change only configuration defaults: small change, but cannot fix unbounded queues, metadata, or rendered history and can reduce throughput without evidence.
2. Incrementally bound and share existing resources (recommended): preserve established runtime authorities and extend current structures. Validate each subsystem separately, then integrate into one PR.
3. Replace worker processes with a shared daemon: much larger authority/process-isolation redesign; unnecessary for the reported problems.

Proceed with option 2 using the following independently testable sections. Reuse existing mechanisms; introduce no new dependency unless the existing standard-library structures cannot meet the requirement.

## A. Worker admission and bootstrap

- Add validated configuration for ordinary subagent concurrency, retaining the default four and maximum eight tasks per call. Wire through the existing scheduler rather than another execution lane.
- Add an optional parent-owned local resource budget shared by workflow, one-shot, and Graph admission. Use admission leases released on success, error, cancellation, and panic. Waiting is cancellation-aware. A task that cannot fit must receive an explicit resource-limit result rather than wait forever. Reservations are accounting estimates, not a guarantee of an operating-system RSS cap; diagnostics must state this.
- Preserve explicit workflow/Graph settings and scheduling dependencies. Do not silently reduce model, effort, tools, or verification when admission is delayed.
- Build immutable model/settings snapshots once per applicable parent configuration generation; keep workspace-specific instruction discovery, trust decisions, credentials and execution-time permission checks outside unsafe global caching. Test configuration reload and workspace changes.
- Select Graph worker host construction using authenticated worker context already established by the parent, never an untrusted environment flag or prompt. Lazily construct optional host services behind existing interfaces; requesting an authorized tool must still initialize its required service. Trace constructors first and avoid claiming savings from structs that already initialize lazily.
- Preserve isolated subprocesses, journal ownership, artifact validation and publication. Prevent nested Graph controllers/repeated global maintenance in worker bootstrap when they are not needed by authorized tools.

## B. Transcript rendering

- Replace retained full-history rendered rows with a block-height index and bounded rendered-block cache around the viewport, with a byte ceiling as well as an entry/row ceiling. Keep the full transcript in the existing model/session store.
- Preserve grouping of tools, images, expanded output, selected search results, typewriter state, and scroll anchors. Invalidate heights/rendered blocks on width, theme, expansion, and earlier-transcript mutations using existing revision stamps.
- Cache settled block heights rather than all Line allocations. Render offscreen blocks again on demand. During resize, rebuild height metadata incrementally or with explicitly bounded work; do not merely move the entire render allocation into a measurement pass.
- Validate distant scrolling and very large individual messages as well as many small entries; a viewport cache alone does not bound one oversized block.

## C. Graph subprocess output

- Replace the unbounded queue with bounded frames and backpressure. Keep each frame and the parser's line buffer bounded. Final drain must stream instead of accumulating an unbounded pending Vec.
- Preserve oversized bytes in private attempt-owned artifacts and report a structured artifact reference. Account for writes and flush failures explicitly; no false successful verification if evidence could not be persisted.
- Do not parse a truncated JSON event as a complete event or silently lose usage, stop reason, persistence failures, or artifact publication events. Oversized protocol events need a recoverable spool/parser path or an explicit incomplete-protocol failure.
- Maintain independent supervision of child lifetime, inherited pipes, cancellation, and deadlines. On shutdown, blocked producer threads must observe receiver closure; never join an inherited-pipe reader indefinitely.
- Preserve existing `exited_child_preserves_queued_output_with_slow_callbacks`, early-pipe-close, descendant-pipe, timeout and cancellation contracts in `graph/process.rs`.

## D. JS lifecycle and metadata

- Retain persistent runners and add validated hot-pool capacity plus inactivity timeout. Keep a safe existing default until measured; document two or three hot runners as the optional profile.
- Store monotonic last-use timestamps. Expire only inactive sessions; a periodic bounded owner/reaper must release truly idle children even if no later extension call arrives. Stop the owner on shutdown and kill/wait for evicted owned children outside broad locks.
- Bound extension-load memoization by entries and bytes, including keys/payloads. Never cache oversized results. Remove stale revisions of a module and preserve mtime/payload identity and execution-boundary validation before cache access.
- Preserve UI-wait/one-shot behavior, extension state semantics, active provider streams, error recovery and existing sandbox-transition tests.

## E. Cache and maintenance lifecycle

- Replace linear victim scans with an access-order index maintained on lookup, replacement, expiry, invalidation and clearing. Use the existing access sequence plus an ordered index; guard sequence rollover and ensure index metadata cannot grow independently of live entries.
- Keep byte accounting and namespace/dependency identity unchanged. Retain durable/negative-cache reporting and the existing shared Weak registry.
- Throttle Token Governor stale-output sweeps by canonical storage root. Coordinate across Graph processes with a bounded maintenance lease/marker; process-local throttling alone does not solve repeated worker scans. Failed sweeps must be retriable, and active/recoverable session outputs must retain current safety rules.
- Preserve disk retention semantics. Smaller cache budgets remain optional configuration until measured.

## F. Language-server and teammate lifecycle

- Reuse existing caller-bound shared language-session handles; do not introduce a second server pool or share across incompatible trust/workspace/configuration boundaries.
- Trace `idle_timeout_ms` usage across the complete module before deciding whether enforcement is missing. If enforcement is absent, add bounded periodic idle eviction using existing session ownership, with tests proving active requests cannot be evicted and permissions remain caller-specific.
- Keep existing session caps and teammate default continuity. Expose/document shorter teammate idle time through the existing configuration mechanism where present; do not force ten minutes globally without measured benefit.

## Validation and measurement

- Add focused deterministic regression tests per changed contract; run the existing relevant module/integration tests, cargo fmt, affected checks and final release build.
- Test queue saturation with a slow callback, giant newline-free stdout/stderr, non-ASCII boundaries, disk-write failure, cancellation while producers are blocked, inherited pipes, and normal worker success/failed verification.
- Test viewport retention bounds, jump-to-history correctness, resize/invalidation, long messages and active streaming. Test JS eviction with a controllable clock and fake/fixture children; never sleep through long timeouts in tests.
- Test resource admission release under all exit paths, denied execution, live configuration changes and worker workspace changes.
- Compare baseline and changed release builds using the same deterministic local fixtures. Record process-tree RSS, startup latency, output volume/retention, child counts and warm latency with machine/build details. Report sample count and observed distributions; do not claim p95 from an inadequate sample.
- Live coding-task success/provider-call benchmarks require a separately bounded task set and explicit authorization because they consume subscription allowance. Offline fixtures establish local correctness and resource behavior only.
- PR description must distinguish measured effects, source-level bounds, platform coverage and unmeasured task quality. Do not claim speed/RAM improvements merely because limits exist.

## Delivery

Use a feature branch based on the verified remote main. Commit and push that branch, then create a PR targeting main as requested. Preserve the installed baseline executable unless a later instruction authorizes replacing it with this feature build. Include configuration documentation and measured results with the PR.

Implementation is gated on review of this written design and the subsequent concrete implementation plan, as required by the selected brainstorming and writing-plans skills. No tests, product edits, feature branch push or PR have been performed during this design pass.
