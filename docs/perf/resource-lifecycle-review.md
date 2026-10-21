# Final independent resource lifecycle review

Reviewed base `d44028fbe822cd71975e91ceda69c7c901a82693` against immutable candidate tree `b6067416da62c28b3c48f63e5bad371be3eabde6`. Read-only review; no product edits, provider calls, additional agents, or duplicate broad tests. `rtk` was unavailable on the reviewer command PATH, so inspection used direct Git/rg. Findings below refer to that candidate, before any subsequent fixes.

Requirements consulted: root AGENTS.md; approved resource lifecycle design; implementation plan (especially shared admission and bounded single-block rendering); progress ledger including explicit synchronous cold layout/resize ruling. Existing full-workspace validation was reported passing by the coordinator; it was not independently rerun. Reviewed the actual admission/worker/tool paths, Graph process capture, LSP idle ownership, cache index changes, TUI index/window paths, and benchmark runner/fixtures. This was a risk-focused review, not a claim of exhaustive inspection of every unchanged caller or platform.

## Important — Nested Graph launch can wait on its caller's held reservation

**Primary location:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:533–537`.

`SubagentRunner::run` acquires an ordinary worker reservation and holds it throughout `(self.inner)(request)` (`crates/davinci-agent/src/subagent.rs:318–329`). A Full-access isolated worktree worker can explicitly request `graph_run`: Full scoping only excludes `agent` (`subagent.rs:423–427`). The worker receives its parent-owned runtime via `main.rs:2637–2641`; Graph deps choose that runtime's configured admission owner (`graph/mod.rs:523–528`). A synchronous `graph_run` then calls `run_graph_owned` and waits for its workers (`graph/mod.rs:682–718`). Those workers try to acquire another reservation from the same owner.

**Reproducible trigger:** configure positive `workerResources` with `memoryBudgetBytes == estimatedWorkerBytes`; launch an ordinary Auto/AlwaysApprove worktree worker with explicit authorized `graph_run`; have it execute a non-dry-run Graph. The outer worker holds the only reservation while waiting for Graph, and Graph cannot start a child until the outer worker returns. Multiple outer workers can form the same cycle at larger saturated budgets.

**Consequence:** the operation cannot make progress until explicit cancellation. The Graph runner's admission closure checks only `abort`, not `spec.run_deadline`/role timeout. Although `graph_run` supplies a default two-hour run deadline, child-process deadline enforcement runs only after successful admission; the controller abort watcher mirrors explicit abort rather than enforcing that deadline during reservation wait. Ordinary sibling saturation can also cause reservation wait to outlast a Graph run deadline.

**Required correction:** carry explicit already-admitted worker context and prevent nested synchronous launches from entering a wait that depends on the caller releasing its own reservation. A prompt explicit resource-limit/nested-launch error is safe; an independent unaccounted owner or blindly releasing the caller's lease is not. Make Graph reservation waiting honor its existing absolute run deadline as well as cancellation. Add one focused regression for a saturated inherited nested launch and one bounded deadline assertion if that behavior is changed.

## Important — Shell blocks still allocate all styled rows before viewport filtering

**Primary location:** `crates/davinci-tui/src/davinci/views/transcript.rs:513–515` (fallback), with implementation at `1280–1303`.

Prose, user text and reasoning use incremental visitation, but `Entry::Shell` reaches `FlowBlock::Rows(entry_lines(...))`; `entry_lines` dispatches to `shell_lines` (`transcript.rs:770–772`). `shell_lines` clones/truncates every output row into a single `Vec<Line>` before the height counter or requested-row filter sees it. Both index measurement and viewport rendering therefore materialize the complete styled block. The 8 MiB/4096-row retained viewport ceiling cannot constrain this temporary allocation.

**Reproducible trigger:** construct/load a public `Entry::Shell` containing hundreds of thousands of output lines and call `conversation_rows`, even if the shell entry is offscreen; alternatively request a small slice intersecting that entry. Indexing allocates a styled line for every source output row, and slicing repeats the full-block materialization before filtering.

**Consequence:** transient styled memory remains proportional to one oversized Shell block, potentially exhausting memory, and repeats for live/window redraws. This is not the explicitly accepted synchronous resize CPU limitation. The approved Task 8 separately requires bounds on live allocations and forbids a full-render-and-discard fallback for an oversized block. Repository searches found Shell construction in renderer tests and the public model; this finding does not assert that the current ordinary tool reducer produces Shell entries.

**Required correction:** introduce an incremental Shell FlowBlock visitor that emits the exact existing header/no-output/output rows one at a time; keep existing text and truncation behavior. Add focused parity and oversized-block allocation/visitor coverage. Generic Tool output is capped to a small number of rows, so the evidence does not establish the same unbounded-row concern for that fallback.

## Measurement audit and limits

The component fixture uses the same production modules/crates for each revision, release settings, explicit baseline/candidate binaries, offline workloads and binary hashes. Linux sampling checks PID start identity and excludes zombies. The runner correctly records RSS as sampled process-tree RSS with shared pages counted more than once; short peaks can be missed. Five samples support raw observations, not a meaningful p95 claim. The transcript fixture checks exact warm viewport parity and measures retained-history behavior; it uses many small Prose entries and does not establish an oversized Shell bound. The host fixture intentionally compares full baseline bootstrap to candidate Graph-role bootstrap, so its result must be described as that narrower role case rather than a general lead-host startup improvement. Idle fixtures use accelerated one-second settings, and do not establish production default expiry latency.

No performance result is inferred from a source bound or an unavailable measurement placeholder. No cross-platform compile claim is made by this review. No Critical finding was established. The two Important findings should be addressed before delivery; additional broad workspace reruns are unnecessary unless subsequent shared-contract changes justify them.

## Lead resolution

Both Important findings are being addressed in the follow-up commit. Shell now has a streaming FlowBlock visitor; the collected public shell_lines compatibility path remains only for callers requesting all rows. The 10,000-line Shell regression failed before the change and checks distant viewport output after it.

Admission now carries a weak marker tied to the actual parent lease through host-only runtime and workflow launch context. Nested saturated launches return an explicit local resource error; they retain the parent's reservation and do not bypass budget checks. The marker expires when the parent exits, allowing later background work to use normal waiting semantics. Graph reservation waits also observe absolute run deadlines and report deadline exhaustion.

Additional lead finding: with_worker_state_from omitted the admission owner, so a for_worker-created handle could use a fresh standalone budget instead of the root owner. This qualifies the original nested-Graph reproduction: owner propagation first needed correction on that path, after which the self-wait became reachable. The follow-up copies both owner and weak lease context. A focused regression checks inherited reserved bytes, nested refusal, workflow launch propagation, release accounting and expired parent context. No permission grants or model-authored values enter these fields.

Benchmark cautions accepted: host results compare the full baseline constructor with a candidate classifier Graph-role constructor; the original transcript fixture measures many small Prose entries; Shell bounded allocation is established by its targeted regression. Sampled RSS and five runs support descriptive medians/ranges only. Cold resize CPU remains explicitly synchronous.
