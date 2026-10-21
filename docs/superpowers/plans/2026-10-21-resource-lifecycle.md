# DaVinci resource lifecycle implementation plan

Status: approved by the user; Native execution in progress on `perf/resource-lifecycle`. See [execution evidence and rulings](../../perf/resource-lifecycle-progress.md).

**Goal:** Implement the approved local-resource improvements and create one PR targeting main from `perf/resource-lifecycle`.

**Design:** [Approved design](../specs/2026-10-21-resource-lifecycle-design.md). Base source: `d44028fbe822cd71975e91ceda69c7c901a82693`. Work in this session; preserve the installed baseline release binary for comparison.

**Architecture:** Extend the current scheduler, runtime handles, native host, transcript renderer and process owners. Use parent-owned admission leases, bounded output frames, bounded render/memo caches, and existing caller-scoped language sessions. Keep isolated Graph executables. No new crate dependency is planned.

## Global constraints

- Preserve model, effort, service tier, prompts, tool semantics, permission revalidation, verification, durable publication, full session history and recoverable evidence.
- Preserve existing concurrency defaults (workflow 16, Graph 3, one-shot 4), cache defaults, and teammate default timeout. Validate configuration; explicit overrides remain authoritative.
- New bounds are local implementation limits, not task-success or measured RSS claims. Admission reservations are estimates, not an OS memory cap.
- Do not store credentials, permission grants or mutable project trust in shared immutable configuration snapshots.
- Do not run live provider/coding benchmarks without separately bounded authorization. Use deterministic local subprocess fixtures for this PR's resource measurements.
- Do not replace the installed baseline binary, push main, change unrelated source, or weaken tests. Commit and push only the feature branch and create the requested PR.
- Use the existing toolchain environment (`source /workspace/.davinci-tools/env.sh`), Rust 1.88.0, locked dependencies and RTK. All Cargo commands below run through `rtk proxy`.

## Review focus

1. Cancellation while a queue or admission budget is full must release owned resources and settle promptly; test both paths in tasks 1 and 5.
2. A single huge newline-free/protocol message must not defeat byte bounds or appear as successful complete evidence; test in task 1.
3. Idle expiry must not kill an active extension stream or language request; test active ownership during maintenance in tasks 2 and 4.
4. Permission/trust/configuration changes and different workspaces must not reuse broader authority; test in tasks 4, 6 and 7.
5. Resize, distant history jumps and one enormous conversation block must keep exact visible content with bounded retained rows; test in task 8.

## Task 1 — Bounded Graph output with recoverable overflow

**Files:** Modify `crates/davinci-coding-agent/src/native_extensions/graph/{process.rs,worker.rs,controller.rs}`; create `graph/process_output.rs` and register it in `graph/mod.rs`; extend tests in `process.rs` and worker event tests.

**Consumes:** Existing `WorkerDeadline`, `run_child_with_deadline`, `parse_worker_event`, attempt-owned artifact paths, process-tree termination and the execution boundary.

**Produces:** Internal `OutputLimits { frame_bytes, queued_frames, max_line_bytes }` with defaults 16 KiB/64/256 KiB. Add `OutputRecord::{Line, SpooledLine}` and `OutputSpoolRef { path, stream, bytes }`. The Graph process caller supplies a private output-artifact directory; `ChildOutcome` reports explicit capture errors. Oversized worker protocol records set an incomplete-protocol error and cannot yield worker success; ordinary verification output can reference its retained artifact. These are Rust-internal changes, not provider wire changes.

- [ ] Add focused failing tests: a slow callback sees all normal lines including a final artifact event; a 10 MiB no-newline stream stays within frame/line bounds and reconstructs exactly from the spool; split UTF-8 is preserved; failed spool writes return an error; cancellation while the bounded queue is full terminates owned children. Assert queue high-water <=64 and line buffer <=256 KiB via test instrumentation.
- [ ] Run `cargo test -p davinci-coding-agent --lib native_extensions::graph::process --locked` and retain the failing evidence for new cases.
- [ ] Implement fixed-size reads and bounded `sync_channel` frames. Assemble lines only to the byte limit, then spool losslessly. Use unique private artifacts under the attempt owner, and make write/flush failures explicit. Update stdout/stderr callbacks and callers to carry spool references. Make final drain bounded and preserve the existing slow-callback success contract; do not collect an unbounded pending Vec or wait forever for inherited pipes.
- [ ] Update `run_worker` to classify oversized/unparseable required protocol evidence as incomplete, with the original bytes recoverable. A spool notice must not be silently ignored by JSON parsing or turn into a false successful verification/publication.
- [ ] Run new tests plus all existing deadline, early-pipe-close, descendant-pipe and cancellation tests in `process.rs`; run `cargo check -p davinci-coding-agent --locked`. Commit this coherent output change.

## Task 2 — JS runner expiry and bounded metadata memoization

**Files:** Modify `crates/davinci-coding-agent/src/js_host.rs`, `settings.rs`; create `src/js_host/lifecycle.rs` for the bounded owner and limits. Extend existing JS host tests and the inline `settings.rs::tests` module.

**Consumes:** Existing `PersistentJsSession`, `PersistentJsPool`, `NODE_LOCK`, `JS_POOL`, `LOAD_CACHE`, `run_js_extension`, `shutdown_js_pool`, child cleanup and execution-boundary checks.

**Produces:** `ExtensionRuntimeConfig` serialized as settings `extensionRuntime`, with `maxPersistentSessions: 8`, `idleTimeoutMs: 300000`, `loadCacheMaxEntries: 128`, `loadCacheMaxBytes: 8388608`. Capacity must be 1–8; zero idle timeout disables idle expiry for stateful-extension compatibility; zero memo entries/bytes disables memo retention. Validate bounds through existing settings diagnostics. Keep existing pool-disable behavior.

- [ ] Add failing tests with an injected monotonic clock: cold start then warm reuse, LRU capacity eviction, idle eviction without another request, active/streaming session survival, reaper shutdown, and entry+byte limits including keys. Test stale module revisions and oversized load results are not retained. Reuse the existing shared test lock to avoid global-state races.
- [ ] Run `cargo test -p davinci-coding-agent --lib js_host --locked` to demonstrate new failures.
- [ ] Add last-used timestamps, a single bounded maintenance owner, and a stopped-owner signal. The owner may only inspect inactive sessions; use nonblocking pool access and drop/kill/wait removed children outside broad locks. Join shutdown outside the pool lock and prevent maintenance threads from strongly retaining an otherwise dead owner.
- [ ] Replace the memo BTreeMap with entry/byte-accounted LRU retention; include serialized payload/key/result sizes, skip oversized entries, remove stale revisions, and keep execution-boundary checks before memo access. Wire configuration through host settings without caching authority.
- [ ] Run JS host/settings tests, including sandbox transitions, provider-stream recovery, UI-wait and one-shot behavior. Commit this lifecycle change.

## Task 3 — Cache eviction index and throttled Governor maintenance

**Files:** Modify `crates/davinci-agent/src/runtime/cache/memory.rs`, `crates/davinci-agent/tests/cache_runtime.rs`; modify `crates/davinci-coding-agent/src/native_extensions/{token_governor.rs,mod.rs}`; create `native_extensions/governor_maintenance.rs` for storage-root coordination.

**Consumes:** `CacheRuntime::lookup`, `insert_memory`, `invalidate`, `clear_memory`, `remove`, current byte accounting and access counters; `TokenGovernor::bind_session`, `sweep_stale_outputs`, existing retention/active-output rules.

**Produces:** One `BTreeSet<(u64, String)>` access-order entry per live memory-cache entry, with helpers to update/remove the index. A Governor sweep coordinator keyed by canonical storage root, using `davinci_sys::lock::ExclusiveFileLock::try_acquire` for cross-process ownership. Throttle successful sweeps to once per hour per root; keep failed scans retriable. Retention age is unchanged. Marker/lock files are host-owned, never model-selected. Keep the lock file in place after unlocking, as required by the existing lock implementation.

- [ ] Add failing cache tests for recency refresh, replacement, expired entries, namespace clearing, dependency invalidation, oversized objects and access-sequence rollover. Assert index cardinality equals live entry cardinality after churn.
- [ ] Implement ordered victim selection, charging index overhead to weight accounting. Rebuild/rebase counters on rollover rather than allowing saturated access timestamps to destroy LRU order. Preserve dependency, negative-cache and durable-store semantics.
- [ ] Add failing maintenance tests with injected time/storage: repeated hosts scan once, separate roots are independent, concurrent processes elect one sweeper, failed sweeps retry, a crashed owner releases its OS lock, future timestamps cannot disable cleanup indefinitely, and current/recoverable outputs survive.
- [ ] Route host creation and session binding through the root coordinator. Keep direct library sweep available for explicit maintenance/tests. Use atomic successful-sweep metadata and release leases on error/unwind; do not hold the coordinator lock during unrelated host initialization.
- [ ] Run `cargo test -p davinci-agent --test cache_runtime --locked`, `cargo test -p davinci-coding-agent --lib token_governor --locked`, and the new maintenance module tests. Commit the cache and maintenance changes.

## Task 4 — Enforce existing language-server idle timeout

**Files:** Modify `crates/davinci-coding-agent/src/native_extensions/language_intelligence/{manager.rs,config.rs,integration_tests.rs,test_support.rs}`; extract owner maintenance into a sibling module if needed.

**Consumes:** Existing `Manager`/`Slot`, `LanguageIntelligence::{new,fork,with_permissions,for_workspace,shutdown}`, `slot_for`, `idle_timeout_ms`, request budgets, session-key partitioning and per-caller permissions.

**Produces:** A single shared session owner for forked handles, with `reap_idle(now: Instant) -> Vec<SharedSlot>`; session termination happens outside the registry lock. Start one maintenance loop on first actual server use, not construction. It exits on owner shutdown/drop; no background daemon or second pool.

- [ ] Add failing tests: expired idle sessions close without another request; active requests/initialization survive; different trust/workspace/profile keys never collide; a denied caller cannot reuse an allowed caller's grant; dropping the last handle closes servers; dropping a fork does not close another live caller's owner.
- [ ] Run `cargo test -p davinci-coding-agent --lib native_extensions::language_intelligence::manager --locked` for the expected failures.
- [ ] Move shared slots/closed state/maintenance ownership behind an Arc owner with a Weak maintenance reference. Reap only slots that are inactive and older than the configured timeout. Keep current capacity limits and deadlines. Preserve parser validation: idle timeout is 30000–900000 ms, default 300000; zero remains a configuration error rather than silently disabling the feature.
- [ ] Run the complete language-intelligence lib tests with fixture servers, existing request-timeout and shutdown tests, then `cargo check -p davinci-coding-agent --locked`. Commit the idle lifecycle fix.

## Task 5 — Shared worker admission and one-shot concurrency configuration

**Files:** Create `crates/davinci-agent/src/runtime/worker_admission.rs`; modify `runtime/mod.rs`, `subagent.rs`, `runtime/workflow/executor.rs`, and worker-launch tests. Modify coding-agent `settings.rs`, `main.rs`, `native_extensions/graph/{mod.rs,controller.rs}` to bind the same parent budget.

**Consumes:** Existing runtime cancellation, `SubagentRunner`, `SubagentParent`, workflow launch/executor lifecycle, Graph launch and parent-owned worker termination. There is no existing general-purpose local-memory admission ledger in these inspected paths; reuse scheduler execution lanes and add only admission accounting.

**Produces:** Host settings `subagentMaxConcurrentTasks` default 4 (valid 1–8), and optional `workerResources: { memoryBudgetBytes, estimatedWorkerBytes }` requiring both positive values. Missing policy means existing behavior. An explicitly configured estimate greater than total budget is rejected. New `WorkerAdmission` is Arc-shared by parent runtime clones; `acquire(bytes: u64, cancelled: impl Fn() -> bool) -> Result<WorkerLease, AdmissionError>` and RAII `WorkerLease` manage reservations. Diagnostics report reserved bytes/queued workers and label estimates.

- [ ] Add failing tests for one-shot width 1/2/4, invalid limits, preserved default 4, source-order results and maximum eight task arguments. Add admission tests: saturation waits without busy-spinning; cancellation while waiting settles; impossible requests fail immediately; lease release works on error, panic and shutdown.
- [ ] Implement a mutex/condition-variable budget with bounded cancellation polling and checked arithmetic. Acquire once at the concrete worker execution boundary, not both scheduler and runner. Hold Graph leases in the parent until its process exits. Workflow jobs that use an already admitted runner must not reserve twice. Standalone executors get a compatible owner instead of silently bypassing an explicit policy.
- [ ] Bind the owner through `RuntimeHandle` and the Graph controller's existing host-owned state; do not serialize a second enforceable budget into children or trust model-authored policy. One-shot, background and teammate workers retain leases for their actual lifetime. Existing tools, model settings and verification still execute after admission.
- [ ] Run targeted `subagent`, `scheduler`, `worker_admission` and workflow executor tests in `davinci-agent`, plus Graph controller launch/cancellation tests in coding-agent. Commit admission/configuration changes.

## Task 6 — Reuse immutable worker launch configuration safely

**Files:** Modify `crates/davinci-coding-agent/src/main.rs`, `settings.rs`, `runtime_host.rs` and corresponding `main_tests.rs`/worker tests; add `src/worker_bootstrap.rs` to hold immutable launch data if needed.

**Consumes:** `build_worker_agent`, `run_nested_subagent`, existing parent callback installation (`SubagentRunner::new`), settings/model loading, resolved worker cwd, named profiles, trusted-project instructions, sandbox rebinding and execution-time permission enforcement.

**Produces:** An Arc-backed immutable `WorkerLaunchSnapshot` containing only parsed non-authority settings/model metadata. The launch snapshot is scoped to the parent configuration generation and effective worker workspace; each reload installs a new generation. Parent live provider/model/effort values still come from `SubagentRequest`.

- [ ] Add failing read-count tests proving a batch reuses eligible metadata, then invalidates after settings/model metadata reload or workspace change. Add denied/narrower permissions, profile containment and worktree-cwd tests. Verify no credential object or permission grant is cached in the snapshot.
- [ ] Extract immutable loading from per-worker construction, passing the snapshot through the existing host callback. Resolve the effective worktree cwd before selecting project-scoped configuration. Keep trust/profile/tool checks and request execution authority fresh. A worker may not inherit another workspace's instructions or trust merely because its parent has a snapshot.
- [ ] Run existing `test_subagent_worker_recoverability_and_tool_guarantee`, `build_worker_agent`/nested-worker regression tests, and relevant instruction-scope/permission tests. Commit snapshot sharing.

## Task 7 — Role-aware native bootstrap and lazy optional services

**Files:** Modify `crates/davinci-coding-agent/src/main.rs`, `native_extensions/mod.rs`, `native_extensions/graph/worker_hooks.rs`, and service internals with verified eager work (`native_extensions/vector_memory.rs`, `native_extensions/learning/mod.rs`, `native_tools/visual_snapshot.rs`). Add focused native-host/Graph fixture tests.

**Consumes:** Authenticated `graph_worker_context`, parent coordinator/tool authorization, `NativeExtensionHost::new_with_agent_dir`, existing setup configuration, actual constructor behavior, and task 6's immutable inputs. Do not select this mode from an environment flag alone.

**Produces:** Internal `HostStartupRole::{Lead, GraphWorker}` selected only after current Graph authentication succeeds. An internal constructor accepts already loaded settings and the authenticated role. Optional data loading/discovery moves to service first-use paths, preserving the concrete service handles and their public tool behavior where possible.

- [ ] Add constructor instrumentation/tests identifying actual reads/process starts in a no-tool worker. Pin the current authorized tool set and denied-tool behavior for each Graph role before changing bootstrap.
- [ ] Defer verified eager memory-store loading, learning-store loading and visual discovery behind their existing authorization/first-use boundaries. Prevent unused nested Graph startup/maintenance. Do not turn untrusted project data loading into an authority grant; startup role is a performance hint only.
- [ ] Add first-use parity tests for memory/learning/language/navigation/verification tools and a forged Graph-context negative control. Assert unused services do no repository/disk/process work, while an authorized first use initializes the required service and produces the same result.
- [ ] Run native-host and Graph context/worker tests, followed by `cargo check -p davinci-coding-agent --locked`. Retain before/after constructor counts as evidence; if a constructor already does no expensive work, leave it unchanged and document that result. Commit lightweight bootstrap changes.

## Task 8 — Bounded transcript rendering

**Files:** Modify `crates/davinci-tui/src/davinci/views/transcript.rs`, `davinci/app.rs`, `davinci/model.rs` if scroll-anchor metadata needs extension. Create `davinci/views/transcript_cache.rs` for indexing/retention rather than growing the existing renderer further. Extend transcript and terminal-scroll tests.

**Consumes:** `ConversationRows::{len,slice}`, `conversation_rows`, `rows_before`, `settled_len`, `settled_key`, `rendered_blocks`, existing transcript stamps and app scroll handling.

**Produces:** A settled block-height/prefix-row index plus a byte-accounted viewport cache: 8 MiB retained rendered rows, at most 4096 rows and 128 cached blocks; overscan <=128 rows each side. `conversation_rows` returns a lazy view tied to the model/width, and `slice` materializes only requested rows. Full transcript retention remains in the model/session store. Bounds include live rendering allocations; a single oversized block must use row-window rendering, not a full render-and-discard fallback.

- [ ] Add failing tests comparing visible output to the current renderer for distant history jumps, grouped calls, images, expansion/collapse, streaming/typewriter and search anchors. Include a 100,000-entry conversation and one multi-megabyte block; assert cache/row/byte high-water bounds with test instrumentation.
- [ ] Extract streaming/count-only block layout so index measurement does not allocate all rendered rows. Cache settled block heights using width/theme/expansion/transcript identity, invalidate affected suffixes on mutation, and rebuild height metadata in bounded chunks on resize.
- [ ] Replace settled full-row retention with row-window lookup and LRU viewport blocks; update app consumers and `rows_before` to use the index. Preserve all existing invalidation and live-tail regression tests and ensure repeated warm tail redraws do not rescan history.
- [ ] Run `cargo test -p davinci-tui --lib davinci::views::transcript --locked`, `cargo test -p davinci-tui --test terminal_scroll --locked`, and app scroll tests. Commit virtualization.

## Task 9 — Release measurements and resource-profile documentation

**Files:** Create `scripts/bench/resource_lifecycle.py`, its deterministic parser/fixture tests under `scripts/bench/tests`, and `docs/perf/resource-lifecycle.md`. Modify `scripts/bench/README.md`, relevant settings docs and `README.md` configuration links.

**Consumes:** Baseline installed release at `/workspace/.davinci-tools/cargo/bin/davinci` (SHA-256 `cfcf8e4272ccd6ab581f99da90623fdefb1876c7954de5d72ea6ecf027406b7e`), matching source identity, candidate release built into a separate target path, and deterministic local fixtures from prior tasks.

**Produces:** A no-provider benchmark runner accepting explicit baseline/candidate paths and emitting machine-readable samples plus process-tree RSS/child-count/startup/warm-latency summaries. Linux can use `/proc`; unsupported counters/platforms are recorded as unknown. Benchmark fixtures cannot accidentally submit a live provider request; use explicit fixture-only builds or standalone local component harnesses where the production CLI cannot perform a local operation.

- [ ] Add parser/sampler tests using recorded process-tree fixtures: descendants, exit during sampling, PID reuse, missing counters and units. Distinguish process RSS sums from unique/shared-memory usage; record sampling interval and the chance of missing short peaks.
- [ ] Build candidate with `cargo build --release -p davinci-coding-agent --bin davinci --locked --target-dir /workspace/davinci-resource-target`. Run identical bounded fixture workloads with at least five cold and five warm samples; include output flood, long transcript, active/idle JS and active/idle LSP workloads. Report observed maxima and individual samples; reserve p95 claims for an adequate sample size.
- [ ] Document optional profile examples: workflow 4, Graph 2, cache 32 MiB memory/256 MiB persistent, JS hot pool 3 and teammate timeout 600000 ms via existing `DAVINCI_TEAMMATE_IDLE_TIMEOUT_MS`. Explain the throughput/state-continuity tradeoffs and explicit opt-in resource reservations; do not silently edit the user's settings.
- [ ] Record correctness outcomes, source/fixture identities, toolchain, process counters, sample sizes and material limitations. Live provider calls/task-quality effects remain unknown. Commit benchmark/docs/results.

## Task 10 — Integration review and PR

**Files:** Final changes from tasks 1–9, approved spec and this plan. No unrelated changes.

- [ ] Run `cargo fmt --all -- --check`, `cargo check --workspace --locked`, focused regression suites from each task, then workspace tests because scheduling/lifecycle interfaces cross crate boundaries. Re-run only failures or checks affected by later fixes.
- [ ] Review the complete diff for bypassed admission, lock-order cycles, authority caching, unbounded fallback allocations, missing shutdown paths, platform regressions and false performance claims. Fix discovered issues and rerun the affected checks. Preserve baseline failures as distinct evidence if any exist.
- [ ] Rebuild release if fixes changed candidate source, recheck relevant benchmark samples, and verify installed baseline hash remains unchanged. Check remote main for movement; integrate it only after reviewing conflicts and rerun checks for affected paths.
- [ ] Commit final reviewed changes, push `perf/resource-lifecycle`, and create a PR targeting main with summary, actual measurements, tests, unmeasured/platform limits and configuration compatibility. Prefer a GitHub tool if available; otherwise use authenticated Git and GitHub CLI/API through the existing connection. If PR creation lacks a callable authenticated API, report that concrete blocker without claiming a PR exists.
- [ ] Return the actual PR URL and the brief measured outcome. Do not merge or push main.

## Plan self-review

All six sections of the approved design map to tasks: A→5–7, B→8, C→1, D→2, E→3, F→4/9. Validation and delivery map to 9–10. New interfaces/defaults above are explicit implementation decisions; existing names are source-verified. The existing older harness-optimization plan covers provider/context efficiency; this plan is limited to local resource ownership and retention.

Recommended execution: **Native**, implementing these tasks in order in this session. The tasks share runtime/host interfaces, so keeping one implementation context avoids duplicated exploration. A final independent branch review is allowed by the execution skill after the user selects that method; no agents have been started during planning.
