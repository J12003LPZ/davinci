# Graph runtime bug report

**Scope:** `crates/davinci-coding-agent/src/native_extensions/graph/` (47 files, ~40k lines; production code through each `#[cfg(test)]` boundary) plus the graph entry points in `main.rs`, `native_extensions/mod.rs`, and all eight TUI graph views (`graph_canvas.rs`, `graph_inspector.rs`, `graph_layout.rs`, `graph_nav.rs`, `graph_perf.rs`, `graph_resume.rs`, `graph_run.rs`, `grafo.rs`).

**Method:** read-only audit. Every finding cites the exact current `file:line`, and the twenty-two findings marked **[proved]** were reproduced with temporary probes (integration/unit tests against the crate internals and public graph/TUI APIs, or shell probes for the Git behaviour). The baseline suites are green (`cargo test -p davinci-coding-agent --lib native_extensions::graph`: 481 passed, 0 failed, 5 ignored; `cargo test -p davinci-tui --lib graph`: 58 passed, 0 failed, 1 ignored), so none of these are compiler/clippy findings.

**Production coverage (final pass):** `types.rs` lines 1–1184, `controller.rs` 1–3919, `operations.rs` 1–1543, `definitions.rs` 1–1483, `coordinator_handler.rs` 1–133, and the eight TUI graph views were read end-to-end; `#[cfg(test)]` bodies were used only to locate invariants. The 14 findings 39–52 and the new finding 53 now have executed probes (22 total).

**Severity counts:** High 5 · Medium 25 · Low 23.

> Caveat: findings marked *(latent)* or *(defense-in-depth)* need a specific configuration or a code path with no production caller today. They are still defects in the invariant the code claims to enforce.

---

## High (findings 1–4, plus 53)

### 1. Resume relaunches a crashed worker without consulting the operation journal — duplicate non-idempotent effects — **High**
`crates/davinci-coding-agent/src/native_extensions/graph/continuation.rs:177`, `mod.rs:811`, `controller.rs:812`, `controller.rs:1071`, `operation_bridge.rs:300`

```rust
// continuation.rs:177 — only the conversation lease and tool ledger are checked
binding.validate_retry_safety().map_err(|error| {
    format!("Interrupted worker '{id}' requires reconciliation: {error}")
})?;
...
recovered.status = TaskStatus::Failed;
task.status = TaskStatus::Failed;        // continuation.rs:190/203
```

```rust
// controller.rs:812 — only a Cancelled record gets the pre-launch recovery gate
.filter(|record| record.status == TaskStatus::Cancelled)
```

```rust
// controller.rs:1071 — a new attempt is spawned directly
for local_attempt in 1..=NODE_ATTEMPTS {
    let Some(attempt) = previous_attempts.checked_add(local_attempt) ...
```

`reconcile_interrupted_attempts` downgrades a crashed `Running` worker to `Failed` after checking `validate_retry_safety`, which inspects the conversation lease and the *tool* ledger only (`worker_sessions.rs:345-416`). It never consults the durable operation journal, even though the attempt record carries `operation_binding`. Because the reconciled record is `Failed` (not `Cancelled`), the pre-launch `retry_recovery_gate` at `controller.rs:1038-1062` is skipped. `begin_attempt` then calls `operation_bridge::launch_worker` with a **new** attempt number (`controller.rs`, `controller_attempts.rs:164-185`), which creates a new journal operation; nothing looks at the previous operation's `EffectPossible` state. The strong gate only runs *after the replacement worker also fails* (`controller.rs:1491`).

This directly violates the module's own contract at `operation_bridge.rs:79-81`: *"A worker failure is not enough to start another process: the original operation must have a durable outcome proving that no effect remains possible."*

**Trigger:** kill the CLI/controller after a Writer's mutating tool completes (result persisted in the ledger) but before `graph_submit`; `/graph-resume`. The writer is relaunched on a workspace its dead incarnation already mutated. A successful `edit`/`write` is "safe" to `validate_retry_safety` because the result is persisted, yet the journal still says the worker launch is `EffectPossible`. Duplicate application of a non-idempotent change (append, incremental edit, script) is possible.

**Fix:** run `inspect_retry_recovery` for the interrupted attempt before dispatching a replacement, exactly as the post-failure path does; or reconcile crashed attempts to `Cancelled` so the existing pre-launch gate is used.

---

### 2. `/graph view` can disclose an arbitrary file tail from a hostile repository — **High**
`crates/davinci-coding-agent/src/native_extensions/graph/store.rs:1028`, `store.rs:781`, `mod.rs:994`

```rust
// store.rs:1028 — follows symlinks; every other workspace reader rejects them
let mut file = fs::File::open(path)?;
let start = file.metadata()?.len().saturating_sub(64 * 1024);
file.seek(SeekFrom::Start(start))?;
```

```rust
// store.rs:781 — a relative stored cwd resolves against the process CWD
let stored_cwd = Path::new(&run.cwd).canonicalize().map_err(|error| {
    format!("Cannot resolve checkpoint workspace for run '{run_id}': {error}")
})?;
```

```rust
// mod.rs:994 — the transcript path is derived from checkpoint data
let path = transcript_path(Path::new(&run.cwd), &run.run_id, &chosen.id);
let tail = store::read_transcript_tail(&path);
```

`workspace_files::read` deliberately refuses symlink leaves and redirected ancestors (`workspace_files.rs:31-45`), and `collect_unreferenced_blobs` refuses redirected run dirs (`store.rs:234-239`). `read_transcript_tail` does neither. The run checkpoint is repository data (`.davinci/graph/runs/<id>/state.json`), and `load_run` accepts a checkpoint whose `cwd` is `"."` because it canonicalizes relative paths against the process CWD. A repository can therefore ship:

- `.davinci/graph/runs/<id>/state.json` with `"cwd": "."`, a large `updatedAt`, and one task whose id matches the symlink;
- `.davinci/graph/runs/<id>/logs/<task>.live.log` as a symlink to `~/.ssh/id_ed25519`, `~/.davinci/auth.json`, `.env`, etc.

Opening the repository and running `/graph view` (no explicit run id) selects the newest run (`mod.rs:951-955`), follows the link, and puts the last 64 KiB of the target into the tool result (model context and UI).

**Fix:** require an absolute, non-empty `run.cwd` and reject relative checkpoints; read the transcript with `symlink_metadata` + `O_NOFOLLOW` (the same admission as `workspace_files::read`); validate `task.id` with `is_safe_run_id` before joining it into a path.

---

### 3. A modified file over 2000 lines is "reviewed" from a patch that contains no changed lines — **High** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs:438`

```rust
if n > 2000 || m > 2000 {
    return format!(
        "diff --git a/{file} b/{file}\n--- a/{file}\n+++ b/{file}\n@@ -1,{n} +1,{m} @@\n@@ large file modified @@\n"
    );
}
```

The returned string is a fake hunk header plus a second `@@` line and **zero** `-`/`+` content lines. It is ~100 bytes, so `chunk_graph_mutation` (`review_coverage.rs:88-95`) emits one normal chunk for it. The reviewer is handed a "diff" with no change, and `record_reviewed` (`controller.rs:3729`, `3817`) marks that chunk reviewed, so `coverage_complete` returns `true`. A reviewer can then `Approve`, and `controller.rs:3865-3870` delivers.

This matters for exactly the files a graph is likely to edit (`controller.rs` itself is ~7k lines). The probe below edits line 1250 of a 2500-line file and confirms the patch contains neither the removed nor the added line.

**Probe output:**
```
test probe_large_modified_file_has_no_diff_lines ... ok
```

**Fix:** emit a bounded real diff (first/last N hunks with explicit truncation markers) and make the chunk un-reviewable until the marker is acknowledged; or treat a content-free patch as unavailable evidence and fail the review gate.

---

### 4. `/graph <goal>` fails outright on any goal with an odd number of quotes — **High** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/render.rs:106`, `render.rs:336`, `render.rs:350`, `mod.rs:1629`

```rust
// render.rs:106 — runs before the "is this really a subcommand?" check
let command = tokenize_args(trimmed)?;
```

```rust
// render.rs:336 — every apostrophe/quote opens a quoted span
'"' | '\'' if in_quote.is_none() => { in_quote = Some(c); }
...
// render.rs:350
if let Some(q) = in_quote {
    return Err(format!("unclosed quote `{q}` in arguments"));
}
```

`parse_advanced_graph_command` tokenizes unconditionally, before `subcommand_shape_matches` can classify the text as an ordinary goal. A single apostrophe in normal English is therefore a hard error:

**Probe output:**
```
test probe_apostrophe_goal_is_rejected_as_unclosed_quote ... ok
```

`/graph fix the parser's bug`, `update the README's API`, `add a "strict" mode` all fail. `mod.rs:1626-1629` returns the error and never falls back to goal parsing. The interactive `is_launch` check (`davinci_interactive/graph_setup.rs:27`) also returns false.

**Fix:** only tokenize when the first whitespace-delimited word is a known subcommand, or make an unterminated quote non-fatal for the goal path (treat it as literal text).

---

## Medium

### 5. Saved graphs cannot recover through `OnFailure` for a required node — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/controller.rs:2701`, `topology.rs:699-742`, `controller.rs:2280-2288`

```rust
// controller.rs:2701 — every required node id must be in succeeded_tasks
for node in &compiled.topology.nodes {
    if node.required && !final_state.succeeded_tasks.contains(&node.id) {
        failed_required = true;
        break;
    }
}
```

A failed node is permanently excluded from `ready_nodes` (`topology.rs:710-714`). An `OnFailure` edge can run a fallback, but the original required node id never becomes `Succeeded`, so the final check always blocks. The same applies to a `Cancelled` node: on resume, only failed/cancelled nodes **without** a failure edge are reset to `Pending` (`controller.rs:2283-2287`), so a stopped node that owns a recovery branch is never retried. `validate_definition` happily accepts these topologies (`topology.rs:306-349` even requires that failure branches reach a reviewer), so the documented revision/recovery loop is unsatisfiable for any `required: true` node.

**Fix:** either reject `OnFailure` edges from required nodes at validation time, or treat "a downstream fallback succeeded" as satisfying the failed node's requirement (and re-open cancelled nodes with failure edges on resume).

---

### 6. The evidence-digest size bound only charges findings, not risks/gaps — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs:239`

```rust
let mut budget = max_chars;
for (_, line) in findings {
    let cost = line.chars().count() + 1;
    if cost > budget { continue; }
    budget -= cost;
    kept.push(line);
}
```

`risks`, `gaps`, and `test_baseline` are assembled afterwards (`briefings.rs:249-283`) without any budget. `validate_evidence` puts no length or count cap on `risks`/`gaps` (`validate.rs:659-668`). A worker (or repository content that steers it) can emit a multi-megabyte risks array; the digest is persisted to `continuation.evidence_digest` (`controller.rs:2890`) and injected into every planner/writer prompt (`controller.rs:2956`, `3161`, `3215`), defeating the purpose of `EVIDENCE_DIGEST_MAX_CHARS = 24_000`.

**Fix:** charge every section against the budget and truncate with an explicit marker.

---

### 7. Saved-graph review gets an unbounded diff and is denied the plan and security verdict — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/bindings.rs:274`, `controller.rs:2565`

```rust
// bindings.rs:274 — raw diff, plan: None, security: None, no chunks
Ok(super::briefings::review_briefing(
    &super::briefings::ReviewInput {
        goal,
        plan: None,
        diff: effective_diff,
        ...
        chunk: None,
        chunk_summaries: None,
        required_chunk_ids: None,
        security: None,
    },
))
```

The built-in delivery path bounds and enriches this exact briefing (`controller.rs:3680` chunks at 60 000 chars, `3780-3783` truncates, and passes `plan` and `security` at `3702-3712`, `3796+`). The saved-definition path passes `cumulative_delta.diff()` unbounded (`controller.rs:2565`) and hardcodes `plan: None`, `security: None`. The reviewer prompt still tells the model to judge plan deviations (`briefings.rs:569`), and a saved graph has no `ReviewCoverage` at all: `saved_review_approves` (`controller.rs:2105`) only checks verdict/severity.

**Trigger:** `/graph run <saved definition>` with an `implement` + `review` stage over a large change. The reviewer prompt grows with the diff and the reviewer cannot see the plan or the security outcome.

---

### 8. Post-exit drain is bounded by frame count, not wall time, and fails a clean run — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/process.rs:260`

```rust
if !outcome.run_deadline_exceeded && !outcome.aborted {
    let mut drained = 0usize;
    while let Ok(frame) = receiver.recv_timeout(DRAIN_GRACE) {
        ...
        drained += 1;
        if drained >= 4096 {
            terminate_process_tree(child_pid);
            return Err(std::io::Error::new(
                ErrorKind::TimedOut,
                "descendant output did not settle; capture incomplete",
            ));
        }
    }
}
```

`DRAIN_GRACE` is per-frame (250 ms), so a descendant that writes one line every <250 ms keeps the supervisor in this loop for up to ~17 minutes after the child has exited. If it reaches 4096 frames, the function returns `Err` even though `child.try_wait()` already reported a clean exit 0. `timed_out` is not excluded either, so a worker that already hit its role timeout still enters the drain. The code's own comment (`process.rs:236-237`) says descendants inheriting the pipe are an expected condition; a background dev server/watcher is enough.

**Trigger:** a worker's `bash` tool starts a watcher that keeps stdout open and prints; the node is reported failed ("descendant output did not settle") despite a valid submitted artifact, or the slot is held for minutes past the deadline.

**Fix:** exclude `timed_out`, and bound the drain by an absolute deadline (e.g. a single 1–2 s grace), not a frame budget.

---

### 9. A failed artifact unlink lets a retry accept the previous attempt's artifact — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/worker.rs:494`, `worker.rs:784`

```rust
// worker.rs:494 — error discarded
let _ = fs::remove_file(&spec.artifact_path);
```

```rust
// worker.rs:785 — accepts whatever is at the path, with no freshness check
if let Some(artifact) = fs::read_to_string(&spec.artifact_path)
    .ok()
    .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
    .and_then(|value| validate_artifact(spec.expect, &value).ok())
{
    return WorkerResult { ok: true, artifact: Some(artifact), ..base };
}
```

`artifact_path` is per run+task, not per attempt (`store.rs:964-968`), and `finish_worker` deliberately leaves artifacts on disk on the deadline path (`worker.rs:755-760`). The only thing preventing attempt N from accepting attempt N−1's file is this unchecked unlink. If it fails (Windows sharing violation from AV/indexer, read-only attribute, leftover handle), a retry whose child exits early without ever calling `graph_submit` is reported `ok: true` with the old artifact. A false success is invisible in the logs.

**Fix:** fail the attempt if the unlink fails, or record a per-attempt nonce/mtime before spawn and require the artifact to be newer.

---

### 10. Every supervisor error is reported as a *spawn* failure and discards the child's state — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/worker.rs:665`

```rust
Err(error) => {
    return WorkerResult {
        stderr: format!("spawn error: {error}"),
        failure_reason: Some(format!("worker could not be spawned: {error}")),
        ..WorkerResult::default()
    }
}
```

`run_child_with_output_dir` returns `Err` for conditions that happen **after** a successful spawn: a line exceeding `MAX_SPOOL_BYTES` (`process_output.rs:188-196`), a spool write failure, a callback error, and the drain cap above. All are labelled "could not be spawned", and `..WorkerResult::default()` throws away `state` (folded usage, final text, `child_pid`) and the entire `TailBuffer` of child stderr. The operator gets the wrong root cause and no child output for the hardest failures, and the spend from the attempt is not attributed.

**Fix:** classify supervisor errors by kind; preserve the captured `state`/stderr/pid in the returned `WorkerResult`.

---

### 11. `max-revision-cycles 0` (and `max-replans 0` / `max-researchers 0`) bypasses the committed-work guard — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/operations.rs:226`

```rust
"max-revision-cycles" => {
    let value = parse_u32_budget(raw_value, resource)?;
    if value != 0 && value < run.counters.revision_cycles {
        return Err("max-revision-cycles cannot be below completed revision cycles".into());
    }
    run.budgets.max_revision_cycles = value;
}
```

The doc comment at `operations.rs:137-139` says zero means "unlimited" for integer ceilings, but the controller treats these fields as an allowance, not a spend cap: `revision_cycles >= budgets.max_revision_cycles` (`controller.rs:3462`), `replans >= budgets.max_replans` (`3298`), and `types.rs:412-415` says they *"stay positive because they bound loops, not spend"*. Because the `value != 0` guard exempts zero, an operator can set a ceiling **below committed work**, and the run is then permanently unresumable: `validate_resume` rejects `delivery.revision_cycles > run.budgets.max_revision_cycles` (`continuation.rs:249-258`).

`max-researchers 0` is the same class with a different effect: `controller.rs:2821` does `.take(max_researchers as usize)`, so zero silently skips all investigation instead of meaning unlimited.

**Trigger:** `/graph budget set max-revision-cycles 0 --revision N --authorize` after one revision cycle; the next `/graph-resume` fails validation.

**Fix:** require a positive value (or apply the committed-work guard to zero too) for loop ceilings and researchers; document zero separately.

---

### 12. Preflight asserts safety checks it never evaluates, and `valid` ignores Unknown/RequiresApproval — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/preflight.rs:137`, `:144`, `:151`, `:219`

```rust
// :137
let max_concurrency = 3;
checks.push(PreflightCheck {
    label: format!("{node_count} worker nodes · max concurrency {max_concurrency}"),
    status: PreflightStatus::Permitted,
...
// :145
checks.push(PreflightCheck { label: "All writer scopes are disjoint".into(), status: Permitted, ... });
// :153
checks.push(PreflightCheck { label: "Reviewer cannot approve own mutation".into(), status: Permitted, ... });
```

Three checks are hardcoded as passing; they are never computed from `def`. The `valid` boolean only tests `!= Rejected` (`:219`), so `RequiresApproval` (untrusted project) and `Unknown` (unclassified shell effect at `:198-206`) still produce `"valid": true`. `/graph dry-run` therefore certifies properties it did not check. It is report-only today (no execution gate reads `valid`), but the report is exactly what an operator uses to decide whether to run.

**Fix:** compute the checks or remove them; make `valid` false for any non-`Permitted` status, or expose a separate `runnable`/`requires_approval` field.

---

### 13. An unreadable file is recorded as a *deletion* in mutation evidence — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs:238`, `:293`

```rust
if let Ok(bytes) = super::workspace_files::read(cwd, Path::new(&rel_path)) {
    let hash = sha256_hex(&bytes);
    current_map.insert(rel_path, FileFingerprint { hash, len: bytes.len() as u64 });
}
...
if !current_map.contains_key(old_path) {
    changed_files.push(ChangedFile::deleted(old_path));
```

`current_map` silently drops every path whose read fails, then treats "not in `current_map`" as deleted. The patch still has the baseline bytes, so the reviewer is shown the **old** content as `-` lines and `deleted file mode 100644`, while the file still exists with new bytes on disk. The symmetric baseline case (`mutation.rs:204`) drops unreadable files, so a file that later becomes readable is reported as graph-*added* with its full content.

**Trigger:** Windows Antivirus/indexer holds the file without share-read; Unix mode `000`/ACL; a path swapped to a symlink (`O_NOFOLLOW` refusal). Reachable from `controller.rs:3387` → `3680` and `3327`.

**Fix:** record unreadable paths as an explicit `unavailable` state and refuse to classify them as added/deleted; surface them in the review evidence.

---

### 14. Replay fingerprints never record the model, so a model swap reuses artifacts — **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/replay.rs:47`

```rust
Self {
    ...
    definition_digest: definition_digest.map(str::to_string),
    simulated: if simulated { Some(true) } else { None },
    model_id: None,                     // always None in production constructors
}
```

`replay_compatible`/`incompatibility_reason` implement a model check (`replay.rs:85-89`, `100-104`) that is dead: every production constructor (`replay.rs:39`, `controller.rs:898, 1434, 1450`, `controller_attempts.rs:129`) sets `model_id: None`; only a test assigns `Some`. Model identity is a material input for reusing a worker artifact: `execute_node` returns any `Succeeded` task's stored artifact unconditionally (`controller.rs:776-797`), and the `resume_artifacts` path (`883-955`) is empty in production (`start_background`/`resume` pass `HashMap::new()`). Resume with `--model` and no per-role override and the old model's artifact is reused as `Succeeded`. `resolve_worker_model_identity` (`mod.rs:123-136`) prefers the role override, and the session model is not part of `config_hash` (only `.davinci/graph.json` is).

**Fix:** populate `model_id` from `spec.model` when creating the fingerprint and compare it on replay.

---

### 15. Reachability is anchored to `nodes[0]`, so a valid multi-root or unsorted DAG is rejected — **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/topology.rs:246`

```rust
let entry_node = definition.nodes.first().map(|n| n.id.as_str());
...
if node.required && !reachable.contains(node.id.as_str()) {
    return Err(GraphTopologyError::UnreachableRequiredNode(node.id.clone()));
}
```

The "entry node" is the first *listed* node, not the source(s) of the DAG (in-degree 0, already computed above). A valid single-root DAG whose `classify` node is not listed first — or any multi-root graph — is rejected as "unreachable". The generated definitions happen to always list `classify` first, which is why tests pass; user-authored YAML is affected.

**Probe output:**
```
test probe_valid_two_root_dag_is_rejected_by_node_order ... ok
```

**Fix:** start reachability from every in-degree-0 node, or from all `required` roots.

---

### 16. Baseline capture reads every tracked file with no size cap — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/workspace_files.rs:52`

```rust
let mut bytes = Vec::new();
file.read_to_end(&mut bytes)?;
```

There is no bound, although `davinci_sys::fs::read_bounded`/`read_capped` exist and are used elsewhere. `capture_baseline` (`mutation.rs:204`) hashes the whole buffer and only skips *blob storage* above 512 KiB — it still reads the entire file into RAM, and it runs at run start and at every writer baseline (`controller.rs:2196, 3188, 3260, 3634`; `forks.rs:94`). A checked-in or stray multi-GB binary causes a huge allocation per baseline; allocation failure aborts the run.

**Fix:** stat first and either hash in a streaming pass or skip oversized files with an explicit "not captured" marker.

---

### 17. Signal deaths are folded into exit code 1 — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/process.rs:303`

```rust
outcome.exit_code = match child.wait() {
    Ok(status) => status.code().unwrap_or(if outcome.aborted || outcome.run_deadline_exceeded {
        130
    } else {
        1
    }),
```

On Unix `status.code()` is `None` whenever the process died from a signal (SIGSEGV, SIGKILL from the OOM killer, SIGBUS). That is mapped to `1`, indistinguishable from `exit(1)`. `finish_worker` then reports `"worker exited with code 1"` (`worker.rs:816`) and the run log records the same (`controller.rs:1365-1368`); no signal information survives in `ChildOutcome`.

**Fix:** keep `ExitStatus`/signal in `ChildOutcome` and report `signal 9` separately.

---

### 18. Provider usage is accumulated with unchecked `u64` addition — **Medium** *(latent)*
`crates/davinci-coding-agent/src/native_extensions/graph/worker.rs:213`, `types.rs:496`

```rust
state.usage.input += number("/usage/input");
state.usage.output += number("/usage/output");
state.usage.cache_read += number("/usage/cacheRead");
```

`number()` returns any `u64` straight from the child's JSON (`worker.rs:207-212`) with no clamp, and `WorkerUsage::add` (`types.rs:496-506`) uses plain `+=`. Two events each carrying `u64::MAX` overflow: panic in debug/test builds, silent wraparound in release, corrupting the run's token/cost totals and any budget decision that reads them.

**Trigger:** a hostile or buggy provider sends huge usage; reachable on every `message_end`.

**Fix:** use `saturating_add` (as `delta` already does) or reject values above a sane ceiling.

---

### 19. The workspace lease checks only `.davinci`/`graph`; the subdirectories actually written are unchecked — **Medium** *(defense-in-depth)*
`crates/davinci-coding-agent/src/native_extensions/graph/lease.rs:32`

```rust
let parent = cwd.join(super::store::CONFIG_DIR).join("graph");
...
for component in parent.strip_prefix(&cwd)...components() {
```

The stated invariant is "reject redirected state directories before creating anything under them", but `runs`, `blobs`, `history`, `checkpoints`, `logs`, and `artifacts` are created/used under `.davinci/graph` without any symlink check (and the legacy `.pi/graph` root is not inspected at all). `forks.rs:235-246` and `store.rs:234-239` implement the missing check for their own subdirectories, showing it is intended. A Unix repository can commit `.davinci/graph/runs` (or `blobs`) as a symlink, and `create_run_dir` (`store.rs:149-150`) plus every `atomic_write` then create/write outside the workspace.

**Fix:** validate every child directory the graph writes to (or open `.davinci/graph` once with `O_NOFOLLOW` and use directory handles).

---

### 20. The evidence schema allows empty `refs`, but the validator rejects it — **Medium** *(contract mismatch)*
`crates/davinci-coding-agent/src/native_extensions/graph/validate.rs:95`, `:598`

```rust
name: "refs",
required: true,
allow_null: false,
kind: FieldKind::StringArray { min_items: 0 },   // schema emits no minItems
description: Some("path:line references that support the claim"),
```

```rust
if refs.is_empty() || refs.iter().any(|v| v.as_str().is_none_or(|s| s.trim().is_empty())) {
    return Err("evidence finding requires at least one non-empty ref".into());
}
```

`artifact_schema(Evidence)` (used for the `graph_submit` tool and the worker prompt) tells the model that `refs: []` is valid; `validate_artifact` then rejects it. A worker following the emitted schema loses a round trip. The parity test (`validate.rs:1151-1187`) only exercises the canonical examples, so the divergence is invisible to CI.

**Fix:** set `min_items: 1` in the contract, or relax the runtime validator.

---

## Low

### 21. The `MissingVerificationNode` guard can never fire — **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/topology.rs:353`

```rust
for edge in &definition.edges {
    if edge.from.starts_with("verify") && !definition.nodes.iter().any(|n| n.id == edge.from) {
        return Err(GraphTopologyError::MissingVerificationNode);
    }
}
```

Step 3 already returns `UnknownNodeInEdge(edge.from)` for every edge whose `from` is not in `nodes` (`topology.rs:152-159`), so the predicate is always false. The variant is never constructed anywhere. The intended invariant — "a verify-success edge must have a verification node" — is not enforced; the check only looks at the id prefix, not at a role. The repo's own test asserts `UnknownNodeInEdge` instead (`topology.rs:817-830`).

**Probe output:**
```
test probe_unknown_verify_edge_is_reported_as_unknown_not_missing_verification ... ok
```

---

### 22. Operation-evidence retention returns "no references" before reading `state.json` — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/store.rs:310`

```rust
let entries = match fs::read_dir(&artifacts) {
    Ok(entries) => entries,
    Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
    Err(_) => return true,
};
```

The doc comment at `store.rs:300-305` promises any persisted operation binding keeps the run directory, but when `artifacts/` is absent the function returns `false` without inspecting `state.json` (the first candidate, line 308). A run dir with a checkpoint containing `operationBinding`/`launchOperationId` is then deletable by `prune_finished_runs` (`store.rs:208-215`). `create_run_dir`'s failure is ignored at `controller.rs:2045`, and older/partially written run dirs can lack the subdir.

**Fix:** scan `state.json` regardless of the artifacts directory.

---

### 23. `blobs::put` trusts any existing file, permanently poisoning a content-addressed key — **Low** *(latent)*
`crates/davinci-coding-agent/src/native_extensions/graph/blobs.rs:26`

```rust
if target.exists() {
    return Ok(());
}
davinci_sys::fs::atomic_write(&target, bytes)
```

`put` validates only the hash *format*, never that `sha256(bytes) == hash`. `migrate_baseline` (`store.rs:427-429`) stores legacy inline bytes under a fingerprint's hash; if a legacy/tampered checkpoint has mismatched bytes, the genuine bytes for that key can never be stored (because `put` never overwrites), and `MutationBaseline::old_bytes` (`mutation.rs:34-38`) rejects the blob forever. Rewind/fork silently loses the preimage.

**Fix:** verify the hash before accepting an existing blob, and replace a corrupt one.

---

### 24. `complete_worker` mixes a stored digest with a new artifact digest on repeated delivery — **Low** *(latent)*
`crates/davinci-coding-agent/src/native_extensions/graph/operation_bridge.rs:442`

```rust
if attempt.state().is_terminal() {
    let stored = journal.load_result_for_attempt(attempt.attempt_id())?;
    let receipt = GraphOperationResult {
        ...
        payload_digest: stored.as_ref().map(|result| result.reference.payload_digest),
        artifact_digest: artifact_digest(result)?,       // caller's NEW result
        ...
    atomic_write(&result_path(cwd, binding), &encode(&receipt)?)?;
```

The branch mixes the journal's stored identity (`state`, `result_id`, `payload_digest`) with the newly supplied result's artifact digest and overwrites the existing receipt. `replay_projection` (`:545`) only compares `binding`/`state`, so it returns the corrupted receipt. No production caller currently completes the same attempt twice (`controller_attempts.rs:264`; attempt numbers are never reused), so this is latent.

**Fix:** return/reuse the stored receipt unchanged, or reject a second completion with a different result.

---

### 25. The retry cap resets on resume — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/recovery.rs:294`, `controller.rs:1489`

```rust
/// `attempt` is one-based and denotes the failed attempt, so only attempt one can
/// produce a second worker invocation under the graph's current cap.
pub fn retry_decision(class: WorkerFailureClass, attempt: usize) -> RetryDecision {
    if attempt > 1 { return RetryDecision::Stop; }
```

The caller passes `local_attempt` (the per-invocation counter), not the task's cumulative `attempt`:
`retry_decision(failure_class, local_attempt as usize)` (`controller.rs:1489`). A node resumed with `task.attempts >= 2` gets `local_attempt == 1` again, so `RetrySameBudget` is allowed and two more workers spawn. The "bounded, one replacement" invariant only holds within one `execute_node` call; repeated resumes have no cumulative automatic cap (only the global worker/cost budgets).

**Fix:** pass the cumulative attempt number (or check `previous_attempts + local_attempt`).

---

### 26. `graph_status`/`status` silently shows a different run when the requested id is unreadable — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:913`

```rust
let explicit = run_id
    .filter(|id| !id.is_empty())
    .and_then(|id| load_run(&self.cwd, id));
let current = explicit.or_else(|| {
    active.clone().or_else(|| list_runs(&self.cwd).first()...)
});
```

An invalid, missing, or corrupt run id is indistinguishable from "no id": the tool returns the active or newest run instead of an error. A caller polling `graph_status` with a stale id can be told the wrong run's phase/cost. `view` has the same auto-selection when no run is active.

**Fix:** if a non-empty run id is supplied and cannot be loaded, return an error naming it.

---

### 27. `abort_all_runs` uses one shared 5-second deadline for every run — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:280`

```rust
let deadline = Instant::now() + SHUTDOWN_WAIT;
for run in &runs {
    while !run.finished.load(Ordering::Relaxed) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(25));
    }
```

The first run can consume the entire `SHUTDOWN_WAIT` (5 s), leaving zero wait for all later runs; they are then left detached ("letting the process exit with them detached", `mod.rs:141-143`). With multiple project runs this is exactly the stranded-worker case the wait exists to prevent.

**Fix:** use a fresh deadline per run, or a global deadline plus a forced kill/join for the rest.

---

### 28. `control_history` grows without a bound — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/control.rs:304`, `controller_control.rs:65`

Every control request is appended to `run.control_history` and persisted in `state.json`; there is no retention/pruning. Repeated `graph-control` calls with unique operation ids (each applied control increments the revision) grow the checkpoint without limit, increasing every `save_run` cost and eventually the parse size on resume.

**Fix:** retain the last N applied/terminal receipts (keep all conflicts) or journal them separately.

---

### 29. `/graph dry-run` accepts a path-escaping name; preflight joins it into a filesystem path — **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/render.rs:122`, `preflight.rs:68-90`

```rust
} else if first == "dry-run" {
    ...
    if name.is_none() {
        name = Some(token);          // no safe_graph_name() check
```

`id_like` permits `.` and `/` (`render.rs:264-269`), so `subcommand_shape_matches` accepts `../../..` as a dry-run target. Sibling subcommands validate with `safe_graph_name` (`render.rs:378`, `400`), but dry-run does not. `preflight.rs:68-90` then falls back to `cwd/.davinci/graph/definitions/<name>.yaml` and reads/parses the file outside the definitions directory. Read-only: no writes, no workers.

**Fix:** apply `safe_graph_name` in the dry-run branch.

---

### 30. The goal parser destroys indentation and strips flag-shaped words anywhere — **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/render.rs:433`

```rust
for line in args.lines() {
    let mut kept: Vec<&str> = Vec::new();
    for token in line.split_whitespace() {
        match token {
            "--simple" => parsed.forced = Some(Complexity::Trivial),
            ...
```

The doc comment says the goal text is kept "verbatim otherwise", but re-splitting on whitespace and re-joining with one space deletes leading indentation, collapses runs, and converts tabs. A pasted code block, YAML, or nested list is corrupted. Flags are stripped wherever they occur with no quoting escape, so `remove the deprecated --simple flag` loses the literal flag *and* forces the trivial path.

**Probe output:**
```
test probe_goal_parser_destroys_indentation_and_strips_flag_words ... ok
```

**Fix:** only strip flags as standalone tokens outside fenced/quoted text, and preserve the original line prefixes.

---

### 31. Export redaction is bypassed by spacing or JSON key syntax — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/export.rs:170`

```rust
let markers = [
    "api_key=", "apikey=", "token=", "password=", "secret=",
    "api-key:", "token:", "password:", "secret:",
];
if markers.iter().any(|marker| lower.contains(marker)) { "[REDACTED]".into() }
```

The module doc (`export.rs:3-5`) promises credentials are kept out, and defaults/allowed values are deliberately dropped for that reason. Matching only these exact substrings misses the most common spellings: `token = abc`, `"password": "hunter2"`, `api_key : sk-…`, `Authorization: Bearer …`, `--token abc`. The export still includes the definition description and parameter descriptions.

**Fix:** use a real secret detector or at least a normalized pattern for `key\s*[:=]\s*value` across quote styles.

---

### 32. Path-parameter validation is keyed on types that can never exist — **Low** *(latent)*
`crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs:1414`, `:411-422`

```rust
match param_type {
    "path" | "scope" => {
        if val.contains("..") || val.starts_with('/') ...
```

`parse_saved_definition` only admits `"path_list" | "relative_paths" | "paths"` as path-ish types (`definitions.rs:411-422`) and rejects `"path"`/`"scope"`. So this arm is dead for every loadable definition, while the types that *do* exist fall through to `_ => {}` with no traversal/absolute-path/drive-letter check. The same mismatch kills the `"int"`/`"bool"` aliases (only `integer`/`boolean` are allowed). Currently limited: bound parameters are discarded (`start_saved_background` only echoes them at `mod.rs:1236`; `run_saved_to_completion` discards at `mod.rs:1252`). It becomes exploitable as soon as parameters are applied.

**Fix:** match the allowed type names, or reject unsupported types at bind time.

---

### 33. Comment stripping is defeated by a single apostrophe — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs:267`

```rust
let in_quotes = before.chars().filter(|&c| c == '"').count() % 2 != 0
    || before.chars().filter(|&c| c == '\'').count() % 2 != 0;
```

"Inside quotes" is inferred from the parity of quote characters before `#`, with `"` and `'` OR-ed together. A single apostrophe anywhere earlier on the line suppresses comment stripping for the whole line; the trailing `# …` becomes part of the value. `description: don't touch auth # security note` yields the full string including the comment. `description` is the run goal (`mod.rs:1198`), so corrupted text is fed to the graph. Round-trip writes always double-quote (`to_yaml_string`), so only hand-authored YAML is affected.

**Fix:** parse the scalar with the same state machine used for values instead of counting quotes.

---

### 34. A verification timeout is not carried into the result; `verified()` relies on the exit code — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/verify.rs:261`, `types.rs:1023`

```rust
Ok(outcome) => {
    if outcome.timed_out {
        output.push_str("\n[graph] verification command timed out");
    }
    (outcome.exit_code, output, duration_ms)
}
```

`ChildOutcome` carries `timed_out`, but `VerifyExec` returns only `(exit_code, text, duration)`, and `VerificationCommandResult` (`types.rs:1008-1021`) has no timeout field; `verified()` only checks `exit_code == 0`. The invariant "a command that exceeded its deadline is never verified" is enforced only by accident (the kill normally yields nonzero). There is a real window in `process.rs:243-256`: `try_wait()` returns `None`, the child then exits 0, `now >= effective` sets `timed_out = true`, and `terminate()`/`wait()` reports the genuine code 0. The command is counted as verified.

**Fix:** propagate `timed_out` into `VerificationCommandResult` and make `verified()` require `!timed_out`.

---

### 35. The config hash fails *open* on an unreadable config file — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/replay.rs:173`

```rust
if config_path.exists() {
    if let Ok(bytes) = std::fs::read(&config_path) {
        return sha256_hex(&bytes);
    }
}
sha256_hex(b"{}")
```

An *existing* config that cannot be read hashes identically to *no config at all*. If the stored fingerprint was taken before the config existed (hash of `{}`), and at replay time a config exists but is unreadable (permissions, lock, `graph.json` replaced by a directory), `policy_matches` is true and the artifact is replayed under an unknown policy. This contradicts the file's own convention two functions below, where an unreadable repo state fails **closed** (`compute_repo_state_hash`, `:251`, returns a random UUID).

**Fix:** return a random/unique hash (fail closed) when the config exists but cannot be read.

---

### 36. An empty required-chunk set counts as complete while the fallback diff is truncated — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/review_coverage.rs:55`, `controller.rs:3779-3783`

```rust
pub fn coverage_complete(coverage: &ReviewCoverage) -> bool {
    ...
    coverage.required_chunk_ids.iter().all(|required| reviewed.contains(required.as_str()))
}
```

`all()` over an empty set is `true`. When the graph owns no mutation (`cumulative_delta` empty), `chunks` is empty and coverage is "complete"; the reviewer is handed `truncate(&default_get_diff(&cwd), DIFF_MAX_CHARS)` — `truncate` (`controller.rs:243-247`) silently cuts with no marker. A dirty workspace larger than 60k chars is partially shown while coverage reports full coverage, allowing `Approve` → `Delivery::Ok` (`controller.rs:3865-3870`). The empty-set behavior is asserted as intended in `review_coverage.rs:134-143`, so the defect is the silent truncation plus the missing chunk in the fallback path.

**Fix:** when falling back to `default_get_diff`, add a truncation marker and a synthetic required chunk (or mark coverage incomplete).

---

### 37. Artifact fields are embedded in downstream prompts without a size cap — **Medium**
`crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs:328`, `:400`, `:478`, `validate.rs:692`

```rust
// briefings.rs:328-385 — every step/test/criteria string is appended verbatim
fn plan_section(plan: &ImplementationPlan) -> String {
    let steps = plan.steps.iter().enumerate().map(|(index, step)| { ... })
```

```rust
// briefings.rs:400-404 — the whole plan goes into the writer prompt
Some(plan) => parts.extend([
    String::new(),
    "## Approved plan (implement THIS, not your own)".to_string(),
    plan_section(plan),
]),
```

`validate_plan` checks only the shape and minimum counts: there is no maximum on `steps.len()`, string lengths, `testsToRun.len()`, `testsToAdd.len()`, or `completionCriteria.len()`. The same is true of `PatchReport.summary`, `ReviewDecision.notes`, and `EvidenceArtifact.risks/gaps`. A worker can submit an arbitrarily large artifact; `graph_submit` writes it with no size check (`worker_hooks.rs:228`, `store.rs:977-983`), and every later planner/writer/reviewer prompt embeds it raw. The runtime-orchestration doc promises a "bounded 2,500 token context packet" (`docs/runtime-orchestration.md:19`); only the evidence *findings* list is budgeted (`briefings.rs:239-247`), and only in the digest.

**Trigger:** a writer (or a repository-steered model) submits a plan with thousands of steps or a megabyte `summary`; every subsequent worker pays for it and can overflow its context.

**Fix:** cap each artifact field and the total serialized artifact at submission time (reject with a field-specific error), and truncate defensively when building briefings.

---

### 38. `/graph diff` never labels unattributed dirty files because the caller always passes an empty list — **Low**
`crates/davinci-coding-agent/src/native_extensions/graph/operations.rs:476`, `mod.rs:1351`

```rust
// operations.rs:476 — the feature is implemented…
let unattributed_code_diff = if let Some(root) = cwd {
    if !unrelated_dirty_files.is_empty() {
        ...
    } else {
        None
    }
```

```rust
// mod.rs:1351 — …but the only production caller always passes &[]
Ok(operations::generate_graph_diff(
    &current,
    prior.as_ref(),
    Some(&self.cwd),
    &[],
))
```

The design doc requires: *"show owned code delta separately and label unattributed edits"* (`docs/superpowers/plans/2026-09-07-14-graph-advanced-controls.md:65`). `diff_command` never computes the dirty-file list (e.g. `git status --porcelain`), so `unattributed_code_diff` is always `None` in `/graph diff`; a user cannot tell graph-owned changes from their own pre-existing dirty files. The path is only exercised by tests (`operations.rs:1916`).

**Fix:** populate `unrelated_dirty_files` from the repository status in `diff_command`, excluding graph-owned paths, or remove the dead field and the claim.---

### 39. `/graph verify` ignores the project's configured `verifyCommands` - **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:1467`

```rust
let commands =
    controller::trusted_detected_verify_commands(self.project_trusted, &self.cwd);
```

```rust
// controller.rs:108-117 — detection only
pub(super) fn trusted_detected_verify_commands(...) -> Vec<VerifyCommandSpec> {
    if project_trusted { detect_verify_commands(cwd) } else { Vec::new() }
}
```

The normal delivery path builds commands with `collect_verify_commands` (`verify.rs:115`), which prefers `config.verify_commands` and then the plan's `testsToRun`. `/graph verify` bypasses both and runs only Cargo/package detection. A project whose `.davinci/graph.json` defines `verifyCommands` (with no Cargo/package test script) gets "no verification commands found to prove correctness"; a project with custom commands gets the default commands instead of the ones the run actually used.

**Fix:** build a `CollectInput` from `load_config(&self.cwd).config.verify_commands`, detected commands, and the run's persisted plan (or at least the config); use `collect_verify_commands` like the controller does.

---

### 40. Saved-graph verification policy and `max_tokens` are accepted but never applied - **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/bindings.rs:183`, `controller.rs:1936`, `controller.rs:2443`

```rust
budgets: def.budgets.clone(),
verification_policy: def.verification_policy.clone(),
```

```rust
// run_graph_internal applies only two of the three saved budgets
if let Some(ms) = saved_budgets.max_duration_ms { budgets.run_deadline_ms = ms; }
if let Some(usd) = saved_budgets.max_cost_usd { budgets.max_cost_usd = usd; }
```

`compiled.verification_policy` is never read by `drive_compiled_saved_graph`; `SavedBudgets.max_tokens` is never applied. The Verify stage (`controller.rs:2443-2503`) always runs commands, always blocks when none exist, and a failed Verify is fatal only through the node's `required` flag at `controller.rs:2701`, not through `verification_policy.required`. A definition can therefore declare `verification_policy: { command_profile: "cargo test", required: true }` and mark its Verify node `required: false`; a failing verification leaves the run eligible for `Phase::Done`. Conversely, `required: false` with no commands still blocks at `2450`.

**Fix:** enforce `verification_policy.required` (and `command_profile`) in the Verify stage, and map `max_tokens` onto a run budget that `budget_exceeded` checks.

---

### 41. Worker-authored `changedFiles` become filesystem paths and raw reviewer-prompt text - **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/controller.rs:3483`, `operations.rs:69`, `briefings.rs:488`

```rust
// controller.rs:3489-3490 — fallback when the graph owns no mutation
} else {
    patch.changed_files.clone()
};
```

```rust
// operations.rs:69-83 — every entry is handed to the manifest builder
if let Some(bundle) = &run.verification_bundle {
    paths.extend(bundle.changed_files.iter().cloned());
}
...
SourceManifestBuilder::new(cwd).target_paths(paths).build()
```

`PatchReport.changedFiles` is model-authored and validated only as an array of strings (`validate.rs:752-757`). When `cumulative_delta.files` is empty (no graph-owned change, or only ignored paths), those strings become the verification bundle's `changed_files`. A later `source_manifest_for_run` (e.g. `/graph verify`, status delivery, or resume) hands them to `SourceManifestBuilder`, which normalizes only backslashes/leading slashes (`source_manifest.rs:365-370`) and then does `root.join(rel)` and `fs::read(full)` (`:149-150`, `:265`), so `../../secret` escapes the workspace on every platform (and a Windows drive/UNC path escapes because only a leading `/` is stripped). The same raw strings are interpolated into the reviewer prompt by `bullet_list(input.changed_files, ...)` (`briefings.rs:488`) immediately; a newline in a "filename" forges a new prompt section.

**Trigger:** a writer submits a valid PatchReport with `changedFiles: ["../../.ssh/id_rsa"]` and makes no graph-owned change; `/graph` proceeds to verification/review, hashing the file and showing the path to the reviewer. A newline in the array element injects reviewer instructions.

**Fix:** require each entry to be a normalized relative workspace path (reject `..`, absolute paths, drive letters, and control characters), and treat the captured mutation as the only filesystem authority.

---

### 42. `git diff` honours repository-local external diff/textconv drivers — **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/git.rs:27`

```rust
.args([
    "-c", "core.fsmonitor=false",
    "-c", "core.hooksPath=/dev/null",
    "-c", "core.pager=cat",
    "-c", "safe.bareRepository=explicit",
])
.args(args);
```

`git::run` disables hooks, fsmonitor, the pager, and global/system config, but not `diff.external`, `diff.<driver>.command`, or `diff.<driver>.textconv`; it also does not pass `--no-ext-diff`/`--no-textconv`. Git reads the repository's local config and `.gitattributes`, so `default_get_diff` (`controller.rs:188`) can execute repository-configured commands.

**Probe:** with a local `diff.external = cmd /c echo GRAPH_EXTERNAL_DIFF_RAN`, running `git diff HEAD` under the graph's exact flags launched `cmd` instead of producing a diff.

**Fix:** add `--no-ext-diff --no-textconv` for bookkeeping diffs and set `-c diff.external=` (plus driver overrides) so a repository cannot supply an executable.

---

### 43. `migrate_baseline` discards inline preimages before the blob write succeeds - **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/store.rs:427`

```rust
for (path, bytes) in std::mem::take(&mut baseline.contents) {
    if let Some(fingerprint) = baseline.files.get(&path) {
        super::blobs::put(blob_dir, &fingerprint.hash, &bytes)?;
    }
}
```

`std::mem::take` empties `baseline.contents` before the first `put`. If a blob write fails (disk full, permissions, a symlinked blob shard), `save_run` returns `Err` but the only copy of the inline bytes is already gone; a retry has neither inline contents nor the blob. Legacy checkpoints are migrated on every save (`store.rs:538`), so a transient failure permanently loses the preimage.

**Fix:** write blobs first and remove each inline entry only after its `put` succeeds (or restore the map on failure).

---

### 44. The repo-state hash and fallback diff include the graph's own state (and dependency trees) - **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/replay.rs:253`, `controller.rs:191`, `mutation.rs:166`

```rust
// mutation.rs excludes harness state and build trees from the mutation baseline
if name == ".git" || name == ".pi" || name == ".davinci"
    || name == "target" || name == "node_modules" { continue; }
```

```rust
// controller.rs:191 — no equivalent exclusion for untracked review evidence
let untracked = git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
```

```rust
// replay.rs:253-264 — the non-git walk excludes .davinci but not node_modules
fn collect_entries(...) { if file_name == ".git" || file_name == "target"
    || file_name == ".pi" || file_name == ".davinci" { continue; } ... }
```

In a git repository that does not ignore `.davinci/`, every checkpoint creates new untracked files under `.davinci/graph/...`. `compute_repo_state_hash` runs `git status --untracked-files=all` with no pathspec exclusion, so the hash changes on every save and replay is refused; `default_get_diff` adds the graph's own logs/artifacts/state to the reviewer's "untracked files" diff. The non-git walk hashes the entire `node_modules` tree on every attempt (unlike `walk_dir_files`, which excludes it).

**Fix:** exclude `.davinci`, `.pi`, `.davinci-transactions`, `target`, and `node_modules` in both the git and directory hash paths, and pass the same exclusions to `git status`/`ls-files`.

---

### 45. `default_get_diff` drops untracked files whose Git path is quoted — **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/controller.rs:191`

```rust
let untracked = git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
for file in untracked.lines().map(str::trim).filter(|file| !file.is_empty()) {
    let path = cwd.join(file);
```

`git ls-files` without `-z` quotes "unusual" paths and escapes non-ASCII bytes. A created `café.rs` is printed as `"caf\303\251.rs"` (verified on this checkout); `cwd.join` then looks for a file literally containing quotes/backslashes, `metadata` fails, and the untracked file is silently omitted from the review diff. The same happens for names containing quotes or newlines. `mutation.rs` uses `-z` for exactly this reason.

**Fix:** use `git ls-files -z --others --exclude-standard` and split on NUL.

---

### 46. `/graph resume` renders checkpoint text without control/bidi sanitization — **Medium** **[proved]**
`crates/davinci-tui/src/davinci/views/graph_resume.rs:79`, `graph_inspector.rs:10`

```rust
// graph_resume.rs:79 — only whitespace is collapsed
let goal = run.goal.split_whitespace().collect::<Vec<_>>().join(" ");
...
rows.push(section_row(width, th, position == selected, &goal, &status));
```

`public_text` strips `char::is_control` but deliberately keeps newlines and does not remove Unicode bidi/format controls (`graph_inspector.rs:26-29`); `terminal_safe` handles both but is only used at `runtime.rs:701`. A run checkpoint is repository data. A goal containing `\x1b[2J` or U+202E reaches the terminal from `/graph resume` (and, through `public_text`, from the other graph panels for bidi controls).

**Probe:** a `GraphResumeRow` goal of `"goal\u{1b}[2J\u{1b}[31mspoof"` appears verbatim in `graph_resume::lines` (assertion checks the output still contains ESC).

**Fix:** run every untrusted field through `sanitize::terminal_safe` (or a public-text variant that also strips bidi/format controls) before it becomes a span; collapse newlines for single-line rows.

---

### 47. The Attention filter stays active after the last attention task clears — **Low** **[proved]**
`crates/davinci-tui/src/davinci/views/graph_run.rs:114`, `graph_nav.rs:14`

```rust
// graph_run.rs:116-119 — the tab is hidden when the count is zero
for tab in GraphFilter::ORDER {
    if tab == GraphFilter::Only(GraphBucket::Attention) && !attention { continue; }
```

```rust
// graph_nav.rs:14-49 — refresh preserves selection but never revalidates the filter
pub fn refresh(model: &mut Model, mut next: GraphRunSheet) { ... }
```

`step_filter` knows how to step off a filter that just emptied (`graph_nav.rs:206-245`), but only when the user presses Tab. `refresh` leaves `graph_canvas.filter` on `Attention`; `filter_tabs` then hides the active tab and `graph_canvas::lines` dims every card because no task is admitted.

**Probe:** after walking to the Attention tab and refreshing with all tasks Done, the filter is still `Attention` and the rendered command center contains no "Attention" chip (no visible active tab).

**Fix:** on refresh, if the current filter is `Attention` and `bucket_count` is zero, move to `All` (or render the emptied tab as disabled); `step_filter` already has the fallback.

---

### 48. A hostile checkpoint's artifacts are read through symlinks; a forged Approve can satisfy review - **Medium** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/store.rs:990`, `controller.rs:2226`

```rust
// store.rs:990 — follows a symlink leaf
let raw = fs::read_to_string(&path).map_err(|_| { ... })?;
```

```rust
// controller.rs:2226 — a review artifact that says approve is accepted
match super::store::read_artifact(&cwd, &run_id, &task.id, task.expect) {
    Ok(artifact) if saved_review_approves(&artifact) => {}
```

`read_artifact` (and `read_task_mutation`/`read_task_fingerprint`) use `fs::read_to_string` without the `symlink_metadata` + `O_NOFOLLOW` admission used by `workspace_files::read`. A repository that ships `.davinci/graph/runs/<id>/state.json` (with `"cwd": "."`; see finding 2) and `artifacts/review-1.json` as a symlink to a forged `{"verdict":"approve","issues":[],"notes":""}` can have `/graph resume` accept a Succeeded review without running one; the artifact is schema-valid, so `validate_artifact` passes.

**Fix:** use the workspace reader for artifact/mutation files and validate the task id/path; never follow a symlink in a run directory.

---

### 49. Git detection only checks `cwd/.git`, so subdirectory workspaces lose git semantics - **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/controller.rs:171`, `replay.rs:182`, `forks.rs:100`

```rust
pub fn default_is_git_repo(cwd: &Path) -> bool {
    cwd.join(".git").exists()
}
```

If the session runs in a subdirectory of a repository, `git` commands still work (they search upward), but `default_is_git_repo` returns false: the classifier is told "git repository: no", `History` research tasks are filtered out (`controller.rs:2820`), `compute_repo_state_hash` uses the directory walk instead of HEAD/status, and `forks::validate_tree_entries` skips its symlink/submodule checks. The graph therefore loses history evidence and git-aware fork/replay protection in a common layout.

**Fix:** use `git rev-parse --is-inside-work-tree`/`--show-toplevel` (bounded) or walk parents for `.git`, and store the resolved toplevel for git operations.

---

### 50. `find_before_writer_checkpoint` falls back to the newest history entry, not one before the target - **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/history.rs:160`

```rust
get_task_checkpoint(cwd, run_id, writer_task_id).or_else(|| {
    let entries = list_history_entries(cwd, run_id);
    for entry in entries.iter().rev() {
        if let Some(cp) = &entry.task_checkpoint {
            return Some(cp.clone());
        }
    }
    None
})
```

When the requested node has no per-task checkpoint (every non-writer, or a writer whose checkpoint was lost), the fallback returns the checkpoint of the **newest** history entry, regardless of whether it precedes the target. In a forked workspace with multiple history entries, rewinding an earlier node can restore the newest fork boundary instead of the one before the target.

**Fix:** select the newest history entry whose `state_revision`/timestamp precedes the target task, or require an explicit checkpoint for non-writer rewinds.

---

### 51. The worker's block hook re-adds role tools the parent removed - **Low** *(defense-in-depth)* **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/worker_hooks.rs:154`, `controller.rs:701`

```rust
// worker_hooks.rs:154 — the child starts from the full role tool set
let mut tools = role_tools(role);
```

```rust
// controller.rs:701-709 — the parent deliberately removes language/process tools
if !has_coordinator_authority || self.deps.language_intelligence.is_none() {
    tools.retain(|name| !language_intelligence::TOOL_NAMES.contains(&name.as_str()));
}
if !has_coordinator_authority || self.deps.processes.is_none() {
    tools.retain(|name| !is_managed_process_tool(name));
}
```

`GraphWorkerContext::from_parts_full` builds `allowed_tools` from `role_tools(role)` plus the extras, not from the parent's `PI_GRAPH_AUTHORIZED_TOOLS`. `block_reason` therefore admits any role-primary tool even when `authorized_worker_tools` removed it. If a coordinator was bound for a different capability, a `tool_search` hit for `lsp_*`/process tools can be admitted and then fail with "native worker capability unavailable" (or reach a parent handler the parent did not intend to expose). The initial `--tools` projection currently limits the blast radius, hence *(defense-in-depth)*.

**Fix:** build the child allowlist by intersecting `role_tools(role)` with the parent's serialized authorization surface; treat `has_coordinator` as capability-specific, not one boolean.

---

### 52. `default_get_diff` turns any Git failure into an empty/partial diff and mislabels it as "not a repository" - **Low** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/controller.rs:188`, `:191`

```rust
let mut diff = git(&["diff", "HEAD"])
    .or_else(|| git(&["diff"]))
    .unwrap_or_default();
let untracked = git(&["ls-files", "--others", "--exclude-standard"]).unwrap_or_default();
```

`git()` already discards the error (`.ok()`), and both diff attempts fall back to an empty string; the untracked listing does the same. The review prompt then says `"(no git diff available - not a git repository; review the changed files by reading them)"` (`briefings.rs:511-513`). A missing/corrupt Git, a permission failure, a bad repository object, or a `git diff` failure is therefore presented to the reviewer as a non-git workspace with no diff, instead of blocking the review for missing evidence. A partially failing diff (e.g. `diff HEAD` fails but `diff` succeeds) silently narrows the evidence to unstaged changes.

**Fix:** distinguish "not a git repository" from "git failed", propagate the latter, and fail closed when the review diff cannot be produced.

---

### 53. `/graph save` writes role names as stage bindings, so every saved generated run is invalid — **High** **[proved]**
`crates/davinci-coding-agent/src/native_extensions/graph/mod.rs:1116`, `bindings.rs:33`

```rust
// mod.rs:1116-1123 — saves the role, not the stage
let bindings = graph_def.nodes.iter().map(|n| definitions::SavedStageBinding {
    node_id: n.id.clone(),
    stage: n.role.to_string(),
    input_artifacts: vec![],
}).collect();
```

```rust
// bindings.rs:33-43 — only stage names are accepted
match s.to_ascii_lowercase().as_str() {
    "classify" => Some(Self::Classify),
    "research" | "investigate" => Some(Self::Research),
    "plan" => Some(Self::Plan),
    "implement" => Some(Self::Implement),
    "verify" => Some(Self::Verify),
    "security" | "security-verify" => Some(Self::Security),
    "review" => Some(Self::Review),
    _ => None,
}
```

`Role::as_str` returns `"classifier"`, `"researcher"`, `"planner"`, `"writer"`, `"reviewer"`, `"test-analyzer"`, `"historian"` — none of which parse as a `SupportedStage`. A generated run saved with `/graph save name` therefore produces YAML whose bindings are all rejected by `compile_saved_definition` (`bindings.rs:88-93`); `/graph run name` blocks with `"saved graph definition invalid: unsupported stage 'classifier' for node 'classify'"`.

**Proof:** the existing `test_command_save_and_run_saved_roundtrip` only asserts `run_res["started"] == true`; adding a completion assertion after `drain_active` made it fail with exactly that message (saved run phase `Blocked`). The generated YAML contained `stage: "classifier"`, `"researcher"`, `"planner"`, `"writer"`, `"test-analyzer"`, `"reviewer"`.

**Fix:** map `Role` to a stage explicitly (Classifier→classify, Researcher/Historian/TestAnalyzer→research or security depending on the node's contract, Planner→plan, Writer→implement, Reviewer→review), or reuse the node id/expect contract when saving. Extend the roundtrip test to assert the saved run reaches `Done`.

---

## Cross-cutting notes

- **The "diff" path is not a real diff.** `format_added_file_diff`/`format_deleted_file_diff`/`format_modified_file_diff` (`mutation.rs:382-479`) build text by hand; they do not quote/escape lines and do not emit `\ No newline at end of file`. A content line beginning with `+++`/`---` can confuse tools, and the >2000-line branch is outright content-free (finding 3). Review evidence is a security boundary for this runtime; it should be generated by a real diff algorithm.
- **Checkpoint data is treated as trusted in several path joins.** `store::artifact_path`, `write_task_attempt`, `write_log`, and `transcript_path` join `task_id` into filenames without `is_safe_run_id` (saved node ids are validated at parse time, but loaded checkpoints are not). The transcript read is the exploitable one today (finding 2); the rest are latent.
- **No test covers the resume-after-crash-with-effect path, the >2000-line review, the quote/tokenizer behavior, `/graph verify`'s configured-command path, the saved verification policy, or the TUI sanitizer**, which is why these survived green 481-test and 58-test suites.

---

## Verification

- Baselines: `cargo test -p davinci-coding-agent --lib native_extensions::graph` → **481 passed; 0 failed; 5 ignored**. `cargo test -p davinci-tui --lib graph` → **58 passed; 0 failed; 1 ignored**.
- Probes (temporary, removed after capture) → **22 passed**:
  - Graph integration probes (8): `probe_apostrophe_goal_is_rejected_as_unclosed_quote`, `probe_goal_parser_destroys_indentation_and_strips_flag_words`, `probe_valid_two_root_dag_is_rejected_by_node_order`, `probe_unknown_verify_edge_is_reported_as_unknown_not_missing_verification`, `probe_large_modified_file_has_no_diff_lines`, `probe_dry_run_accepts_a_path_escaping_name`, `probe_replay_fingerprint_never_records_the_model`, `probe_builtin_definition_is_valid`.
  - TUI integration probes (2): `probe_attention_filter_stays_active_after_attention_clears`, `probe_graph_resume_renders_control_sequences_from_checkpoint_goal`.
  - Crate-internal audit probes (12): `probe_verify_command_input_ignores_config_commands`, `probe_saved_verification_policy_and_max_tokens_are_ignored`, `probe_changed_files_escape_workspace_in_source_manifest`, `probe_git_diff_honours_external_diff`, `probe_migrate_baseline_loses_inline_contents_on_write_failure`, `probe_repo_hash_tracks_the_graphs_own_state`, `probe_untracked_non_ascii_file_is_omitted_from_diff`, `probe_artifact_read_follows_a_junction`, `probe_default_is_git_repo_misses_subdirectories`, `probe_history_fallback_returns_the_newest_checkpoint`, `probe_worker_allowlist_readds_language_tools`, `probe_default_diff_swallows_a_broken_git_directory`.
- Finding 53 proof: the existing `test_command_save_and_run_saved_roundtrip` only asserted `started`. A temporary completion assertion after `drain_active` failed with `left: Blocked, right: Done` and `Some("saved graph definition invalid: unsupported stage 'classifier' for node 'classify'")`; the saved YAML contained `stage: "classifier"`, `"researcher"`, `"planner"`, `"writer"`, `"test-analyzer"`, `"reviewer"`. The assertion was removed and the original test passes.
- Shell probes (Windows, Git installed):
  - `git ls-files --others --exclude-standard` on a file named `café.rs` printed `"caf\303\251.rs"` (quotes + octal escapes), reproducing finding 45.
  - With local `diff.external = cmd /c echo GRAPH_EXTERNAL_DIFF_RAN`, `git diff HEAD` under the graph's exact flags launched `cmd` instead of producing a diff; with an external command that does not exist, Git exited 128 (`external diff died`), reproducing finding 42.

---

## Fix status

All 53 findings were fixed on branch `fix/graph-runtime-bugs`. Regression tests were added for the behaviours that changed; no finding was closed by weakening a check.

| # | Fix |
|---|-----|
| 1 | `controller.rs`: a Failed attempt left by an interrupted controller with an `operation_binding` goes through `retry_recovery_gate` before a replacement is dispatched (same gate as the post-failure path). |
| 2 | `store.rs`: transcript/state/artifact/graph reads use no-symlink readers (`open_regular`, `check_run_path`) and reject redirected ancestors; `load_run_checked` rejects a non-absolute checkpoint `cwd`. |
| 3 | `mutation.rs`: `format_large_modified_file_diff` trims the common prefix/suffix and emits an index-aligned change script, so large files carry every changed line. |
| 4 | `render.rs`: tokenizer failure makes `parse_advanced_graph_command` return `None` (goal text) instead of a quote error. |
| 5 | `controller.rs`: `failure_branch_succeeded` requires every terminal node reachable from the `OnFailure` branch to have succeeded; an intermediate descendant or a failed terminal reviewer does not recover a required node. |
| 6 | `briefings.rs`: `push_bounded_section` charges risks, gaps, and the test baseline against the same evidence budget. |
| 7 | `bindings.rs`: saved review receives bound plan artifacts and the saved security verdict; its diff is split into tracked review chunks instead of being truncated (`bounded_review_diff` removed). |
| 8 | `process.rs`: post-exit drain is bounded by `DRAIN_BUDGET` wall time instead of a frame count. |
| 9 | `worker.rs`: a failed artifact unlink is a worker failure instead of being ignored. |
| 10 | `worker.rs`: supervisor errors report `worker supervisor failed` with exit code, stderr tail, delivered text, and usage. |
| 11 | `operations.rs`: `max-researchers`, `max-revision-cycles`, and `max-replans` reject zero. |
| 12 | `preflight.rs`: writer/reviewer checks are computed from the definition, concurrency is the configured ceiling, and `valid` requires every check to be `Permitted`. |
| 13 | `mutation.rs`: an unreadable baseline/mutation file is an error, not a deletion (`NotFound` still counts as absent). |
| 14 | `replay.rs`, `controller.rs`, `controller_attempts.rs`: `ReplayFingerprint` records the worker model; replay and the completed-task fast path resolve the model the controller would spawn now and refuse a mismatch or a one-sided unresolved identity. |
| 15 | `topology.rs`: reachability starts from every in-degree-zero node, so multi-root and unsorted DAGs are valid. |
| 16 | `workspace_files.rs`: reads are capped at 64 MiB; `mutation.rs` propagates capture errors. |
| 17 | `process.rs`: `fallback_exit_code` maps a Unix signal death to `128 + signal`. |
| 18 | `types.rs`, `worker.rs`, `controller.rs`: usage counters use saturating addition; non-finite/negative cost is ignored. |
| 19 | `lease.rs`: the lease checks every written subdirectory under `.davinci/graph` and the legacy `.pi/graph`. |
| 20 | `validate.rs`: the evidence schema now declares `refs` `minItems: 1`, matching the runtime validator. |
| 21 | `topology.rs`: the unreachable `MissingVerificationNode` loop was removed (step 3 already rejects unknown edge endpoints). |
| 22 | `store.rs`: `run_has_operation_references` no longer returns early when `artifacts/` is absent; `state.json` is always scanned first. |
| 23 | `blobs.rs`: an existing blob is reused only when its bytes hash to the requested key. |
| 24 | `operation_bridge.rs`: repeated terminal delivery reuses the durable receipt and never attaches a fresh artifact digest. |
| 25 | `controller.rs`: the retry cap is computed from the absolute attempt number, so a resume cannot reset it. |
| 26 | `mod.rs`: `graph_status`/`status` return an error for an explicit unknown/unreadable run id instead of showing a different run. |
| 27 | `mod.rs`: shutdown applies a separate `SHUTDOWN_WAIT` deadline per run. |
| 28 | `control.rs`: `control_history` is capped at 256 recent receipts. |
| 29 | `render.rs`: `/graph dry-run <name>` validates the name with `safe_graph_name`. |
| 30 | `render.rs`: `parse_graph_args` strips only standalone flag tokens and keeps whitespace, tabs, and line structure verbatim. |
| 31 | `export.rs`: redaction normalizes quotes/whitespace and covers JSON/YAML/CLI/authorization/bearer forms. |
| 32 | `definitions.rs`: `path`, `scope`, `paths`, `path_list`, and `relative_paths` all take the path validator. |
| 33 | `definitions.rs`: `strip_yaml_comment` tracks double-quoted escapes and doubled single quotes. |
| 34 | `verify.rs`: a timed-out command is recorded as exit 124 even if the kill raced a clean exit. |
| 35 | `replay.rs`: an existing but unreadable config hashes to a fresh value (fail closed) instead of `{}`. |
| 36 | `controller.rs`, `review_coverage.rs`: a no-mutation review's working-tree diff is split into tracked chunks up to `DIFF_MAX_CHARS` before any model request; no truncation marker can stand in for a reviewed chunk. |
| 37 | `worker_hooks.rs`: artifacts are capped at 64 KiB before validation. |
| 38 | `mod.rs`: `/graph diff` passes working-tree files not owned by the run as unattributed (NUL-safe status parsing). |
| 39 | `mod.rs`: `/graph verify` collects configured `verifyCommands` plus detected commands through `verify::collect_verify_commands`. |
| 40 | `controller.rs`, `mod.rs`: a saved graph's `verificationPolicy.required` blocks on failure; `maxTokens` is rejected instead of silently ignored. |
| 41 | `validate.rs`, `operations.rs`: `changedFiles` is limited to 512 paths, checked with platform-independent syntax (backslashes, drive/UNC prefixes, `.`/`..`, empty segments, control characters), and re-checked before becoming a source-manifest path. |
| 42 | `controller.rs`, `replay.rs`: every git call uses `--no-ext-diff --no-textconv`, and output paths are NUL-safe (`ls-files -z`). |
| 43 | `store.rs`: `migrate_baseline` restores the inline contents when a blob write fails. |
| 44 | `replay.rs`, `controller.rs`: repo-state and diff commands exclude `.davinci`, `.pi`, `target`, and `node_modules`. |
| 45 | `controller.rs`: `ls-files -z` parses untracked paths without Git's quote/octal escaping. |
| 46 | `graph_resume.rs`: the checkpoint goal passes through `terminal_safe`. |
| 47 | `graph_nav.rs`: an emptied Attention filter resets to `All`. |
| 48 | `store.rs`: artifact, state, graph, mutation, fingerprint, and transcript reads reject symlinks/reparse points and redirected ancestors. |
| 49 | `git.rs`, `controller.rs`, `replay.rs`, `forks.rs`: repository detection uses `rev-parse --is-inside-work-tree`, so subdirectories keep git semantics. |
| 50 | `history.rs`: `find_before_writer_checkpoint` returns only the writer's own checkpoint instead of the newest unrelated one. |
| 51 | `worker_hooks.rs`: `PI_GRAPH_AUTHORIZED_TOOLS` is authoritative and role defaults are intersected with it. |
| 52 | `controller.rs`: git failure in a repository yields `DIFF_UNAVAILABLE_PREFIX` and blocks the review instead of approving from an empty diff. |
| 53 | `mod.rs`: `saved_stage_for_node` maps roles to runtime stage names for both `/graph save` and `/graph export`; the roundtrip and export tests compile the generated bindings. |

**Test evidence (fixed branch):**

- `cargo test -p davinci-coding-agent --lib native_extensions::graph`: **494 passed; 0 failed; 5 ignored**.
- `cargo test -p davinci-tui --lib graph`: **58 passed; 0 failed; 1 ignored**.
- `cargo test -p davinci-coding-agent --lib`: **1652 passed; 0 failed; 19 ignored**.
- `cargo clippy -p davinci-coding-agent -p davinci-tui --all-targets -- -D warnings`: clean.

New regression tests cover goal/indentation preservation, apostrophe goals, dry-run name validation, multi-root topology, evidence `refs`, `changedFiles` escape/forgery, large-file diffs, saved-run completion, fallback review chunks, and the process-wide registry guard used by shutdown tests.

**PR review follow-up:** six findings from the review of PR #266 were fixed on top of the first pass:

1. The live-browser fixture's writer authorization list now includes every browser tool the graph phase invokes (`browser_click`, `browser_network`); strict parent authorization is unchanged.
2. Working-tree review diffs are chunked into tracked pieces before any model request; `truncate_diff`/`bounded_patch` no longer let a prefix satisfy coverage, and a regression test drives a >60k untracked diff through multiple chunk reviews and checks the last line reached a reviewer.
3. `/graph export` uses the same `saved_stage_for_node` mapping as `/graph save`; the export test compiles the exported bindings.
4. The completed-task fast path and replay admission resolve the model the controller would spawn now (not the old task's recorded model) and refuse a mismatch or a one-sided unresolved identity.
5. `is_safe_workspace_relative_path` applies platform-independent syntax checks (backslashes, drive/UNC prefixes, `.`/`..`, empty segments) before native path parsing, with direct and artifact-level tests.
6. Recovery of a failed required node requires every terminal node of its `OnFailure` branch to have succeeded; an intermediate success or a failed terminal reviewer no longer counts.


