# Evidence Availability Across Context Reduction

* **Author:** J12003LPZ
* **Status:** Implemented; pending review
* **Date:** 2026-11-02
* **Base commit:** `543f9a3b`

## Overview

DaVinci has two demonstrated defects in how it reduces model-visible context.

1. The Token Governor's JSON selector can skip a diagnostic that sits past its identity budget. The shipped test cannot detect this because it asserts compression, not retention.
2. Codemode truncates a script result before storing the omitted middle. No recovery mechanism can reach the discarded bytes.

This design fixes those two defects only. It does not introduce a cross-component contract, change pruning, change Context VM, add a namespace, or add an omission-record schema. Those were reviewed and removed from this iteration because the evidence for them is weaker than the evidence for the two fixes.

The rule for this iteration is one sentence: every reduction either keeps the evidence visible or names where it went and how to read it. When no store exists, the result says so and stays `Partial`.

## The Problem

### Governor JSON selection

`selected_json_indices` in `crates/davinci-coding-agent/src/native_extensions/content_router.rs` seeds the first and last items, then adds unique identities until the selected set reaches ten. At that point it exits the loop. Diagnostic records are capped at four.

In the existing 30-item fixture with an error at index 17, the identity budget fills at index 8 and the error is never inspected. The test asserts only that the view is smaller and names `retrieve_output`. Neither assertion depends on the error surviving, so the test cannot fail when the error is dropped.

The result is not data loss. The trailer says `showing 10 of 30` and names the output id. The loss is that the agent is not told an error is among the missing items, so it has no reason to retrieve.

### Codemode pre-truncation storage

`assemble_script_output` keeps the head and tail of an over-budget script result and returns the truncated text. `script_log_events` in `crates/davinci-agent/src/codemode/execution.rs` builds the retained log from `outcome.output_text`, which is already that truncated text. No path stores the pre-truncation string, so the omitted middle is irrecoverable.

`read_script_log` exists but has no caller outside tests. Exposing it without changing what is stored would make a retention test pass while the gap remains.

### What is not claimed

There is no measured task failure from either defect. There is no evidence that pruning or Context VM selection loses task-critical evidence. Those questions remain open and are listed under Future Improvements. This document does not answer them.

## Goals

* **G1. Diagnostic priority.** The Governor's JSON selector selects recorded diagnostic indices before filling identity slots. A diagnostic at index 25 of 30 survives. Verified by a test that fails on a fill-only selector.
* **G2. Diagnostic accounting.** The JSON trailer reports how many diagnostic records are present in the visible view and how many are absent.
* **G3. Pre-truncation capture.** Codemode captures the full collected body before projection and persists it after the host returns and before the outcome is published. A crash between projection and persistence loses the body; this design does not close that window because the host does not own the store. The window is stated, not hidden.
* **G4. Line-based recovery.** The visible marker names the evidence path and the line where the omitted region begins. `read` with `path`, `offset`, and `limit` returns the middle without re-running the script.
* **G5. Honest no-store behavior.** When the agent has no evidence store, a truncated result carries a marker that says retention is unavailable and names the omitted line. A complete result carries no marker. The status match is left unchanged.
* **G6. No new machinery.** No new store, no new tool, no tool-catalog change, no namespace, no deletion of existing evidence.

## Non-Goals

* A cross-component availability contract.
* Changes to pruning or Context VM selection.
* A shared reference namespace or dispatcher.
* A new scoring engine, learned ranker, or model-in-the-loop importance judge.
* Additional runtime agents.
* A new omission-record schema or a telemetry expansion beyond one counter.
* The Governor dedupe marker change.
* A session-scoped evidence store.
* Compression ratio as a success metric.

## Open Questions

None block Phase A. The constants, algorithm, field names, test locations, and fallback behavior are specified in the Technical Plan and Detailed Implementation Plan.

Deferred questions are listed under Future Improvements.

## Critical User Journeys (CUJs)

### CUJ 1: A diagnostic beyond the identity budget

* **Given**: A JSON array of 30 records. Each has a unique `id`, a `status`, and a repetitive payload. Record 25 has `status: "error"`.
* **When**: The Governor compresses the array and the specialized JSON view is chosen.
* **Then**: The view contains `item[25]` or the error record itself, and the trailer says `diagnostics kept: 1, omitted: 0`. A selector that only fills identities selects `{0..14, 29}` and fails the test.

### CUJ 2: A long script result with a failure in the middle

* **Given**: Code mode is on and a script emits 40 KiB with a failure in the middle. The model-visible output budget is 16 KiB.
* **When**: The script finishes and the host projects its output.
* **Then**: The model sees the head, the tail, an omission marker with the byte count and the first omitted line, and the evidence path. The omitted middle was captured before projection and is stored before the outcome is published. `read` with the named path and line offset returns the failure without re-running the script.

### CUJ 3: A worker child with no evidence store

* **Given**: A subagent or worker child runs Code mode and has `evidence = None` (`crates/davinci-coding-agent/src/main.rs:2791`).
* **When**: Its script output is truncated.
* **Then**: The model sees the head, the tail, the omitted byte count, the first omitted line, and `Full output not retained: no evidence store in this agent`. The host's status match is unchanged, so a successful truncated run is `Partial`. No path or id is claimed. A complete run carries no marker.

## Technical Plan (Architecture)

No new component. Two local changes, each behind its own test.

```text
Governor:
tool result -> store full text -> classify -> build JSON view
  -> pass 1: collect all diagnostic indices; keep the first 12 for pre-seeding
  -> pass 2: select pre-seeded diagnostics, seeds, then unique identities (<= 16)
  -> count diagnostics present in the final selected set
  -> trailer: showing N of M; diagnostics kept: K, omitted: O

Codemode:
script run -> collected body (<= 1 MiB) -> capture before truncation
  -> truncate -> head + tail + omitted_bytes + omitted_start_line
  -> host returns outcome with body, count, line
  -> evidence.store("script-output", full body) when a store exists
  -> marker: path + omitted bytes + first omitted line + read path/offset/limit
  -> no store: marker says unavailable; host status unchanged
```

### 1. Governor JSON selection

File: `crates/davinci-coding-agent/src/native_extensions/content_router.rs`.

Add two constants: `MAX_SELECTED_ITEMS = 16` and `MAX_DIAGNOSTIC_ITEMS = 12`.

Replace `selected_json_indices` with:

```text
struct JsonSelection {
  indices: BTreeSet<usize>,
  diagnostics_kept: usize,
  diagnostics_omitted: usize,
}

fn selected_json_indices(items: &[Value]) -> JsonSelection
```

Exact algorithm:

1. If `items.len() <= MAX_SELECTED_ITEMS`, select every index. Count every diagnostic, set `diagnostics_kept` to that count, and set `diagnostics_omitted = 0`. Everything is visible, so no omission exists.
2. Otherwise, scan every item once. Record every diagnostic index in `all_diagnostics`. Record the first `MAX_DIAGNOSTIC_ITEMS` of them in `diagnostic_seeds`.
3. Seed `selected` with every index in `diagnostic_seeds`. Insert index 0 and index `items.len() - 1`. A diagnostic at either boundary is already present and counts once.
4. Initialize the identity set with `json_identity` for every index already in `selected`. This preserves the current selector's behavior: an identity represented by a selected diagnostic or boundary is not inserted again.
5. Walk indices `0..items.len()` in ascending order. For each index not already selected, insert it when `json_identity(item)` returns a value not seen before, until `selected.len() == MAX_SELECTED_ITEMS`.
6. If the walk ends below `MAX_SELECTED_ITEMS`, insert the remaining indices in ascending order until the cap is reached. This handles arrays with fewer than 16 unique identities.
7. Set `diagnostics_kept` to the number of indices in `all_diagnostics` that are present in `selected`. Set `diagnostics_omitted = all_diagnostics.len() - diagnostics_kept`. These counts describe the final view, not the pass-one quota.

`json_identity` keeps its current behavior: the first present key from `IDENTITY_KEYS` (`id`, `name`, `path`, `file`, `line`, `status`, `type`, `error`, `message`), rendered as `key=value`.

Diagnostics are classified from structured fields and values, not from ordinary field names. A record counts when a status-like field (`status`, `level`, `severity`, `result`, `outcome`) names a failure; when `error`, `failure`, `panic`, or `exception` is non-null and not `false`; when `exitCode` or `exit_code` is non-zero; when `passed`, `success`, or `ok` is `false`; or when `failed` is `true` or greater than zero. Free text counts only for explicit failure markers such as `panic`, `assertion failed`, `error:`, or `traceback`. `expected` and `actual` are ordinary data and do not consume the diagnostic quota. For `JsonObject`, the same structured check runs on top-level fields, not only on array values.

Change `retrieval_trailer` to:

```text
fn retrieval_trailer(
  kind: ContentKind,
  shown: usize,
  total: usize,
  output_id: &str,
  diagnostics: Option<(usize, usize)>,
) -> String
```

When `diagnostics` is `Some((kept, omitted))`, append `; diagnostics kept: {kept}, omitted: {omitted}` before the existing `full exact output is saved as` clause. When `None`, the trailer is unchanged.

Callers:

| Caller | Argument |
| --- | --- |
| `build_log_view` | `None` |
| `build_json_array_view` | `Some((selection.diagnostics_kept, selection.diagnostics_omitted))` |
| `build_ndjson_view` | `Some((...))` |
| `build_json_object_view` | `Some((sum of kept, sum of omitted))`, one aggregate trailer |
| `build_search_results_view` | `None` |
| `build_bounded_line_view` | `None` |

The object view keeps one aggregate trailer. It does not emit one trailer per array key.

`build_log_view` is unchanged. Its unbounded diagnostic scan is not a demonstrated loss, and changing it requires a config signature chain that this iteration does not need.

### 2. Governor fallback visibility

Files: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs` and `content_router.rs`.

Add `rejected: u64` to `KindRetrievalStats` (the per-kind counter struct), with `#[serde(default)]`. In `after_tool`, capture whether `specialized.is_some()` before the match. In the `_` arm, when a specialized view existed but failed `clears_specialized_threshold`, increment `self.content_routing.by_kind.get_mut(kind).rejected`. The status JSON already serializes `by_kind`, so `contentRouting.byKind.<kind>.rejected` appears without a new key.

A specialized view that contains a diagnostic is preferred over the generic digest when it is smaller than the original content, even when it fails the 10% gate. A diagnostic view that is larger than the input is not a reduction; the generic digest wins and the diagnostic is left to the trailer. This keeps an all-error log compressible while a sparse late diagnostic still reaches the model.

### 3. Codemode pre-truncation body

Files: `crates/davinci-agent/src/codemode/projection.rs`, `crates/davinci-agent/src/codemode/types.rs`, `crates/davinci-coding-agent/src/codemode_host/supervisor.rs`, `crates/davinci-agent/src/codemode/execution.rs`.

Change `truncate_head_and_tail` to return a struct, not a tuple:

```text
pub struct Truncated {
  pub text: String,
  pub omitted_bytes: usize,
  pub omitted_start_line: usize, // 1-based line in the full body
}

pub fn truncate_head_and_tail(text: &str, limit: usize) -> Truncated
```

Compute `omitted_start_line` as the number of newline bytes in `text[..head_end]` plus one. The cut can land inside a line; the named line is the line that contains the first omitted byte, and the marker says "beginning at line L".

Change `ScriptOutput`:

```text
pub struct ScriptOutput {
  pub text: String,
  pub complete: bool,
  pub omitted_bytes: usize,
  pub omitted_start_line: Option<usize>,
  pub retained_body: Option<String>,
}
```

`assemble_script_output` sets `retained_body = Some(collected)` and `omitted_start_line = Some(truncated.omitted_start_line)` only when it truncates. A complete result sets both to `None`.

Change `CodeModeOutcome`:

```text
#[serde(skip)] pub retained_output: Option<String>,
#[serde(skip)] pub omitted_bytes: usize,
#[serde(skip)] pub omitted_start_line: Option<usize>,
```

The fields travel in-process from the host to `execution.rs`. They are skipped from serialization, so they never enter the session, events, or the wire shape.

### 4. Codemode storage and marker

File: `crates/davinci-coding-agent/src/codemode_host/supervisor.rs`.

In the completion outcome, set:

```text
retained_output: output.retained_body.clone(),
omitted_bytes: output.omitted_bytes,
omitted_start_line: output.omitted_start_line,
```

Do not add the path. Leave the `(ok, output.complete)` status match unchanged; a truncated successful run is `Partial`.

File: `crates/davinci-agent/src/codemode/execution.rs`.

After the host returns and before `script_log_events`:

1. If `retained_output.is_some()` and `self.evidence.is_some()`:
   - Call `evidence.store("script-output", &body)`.
   - On success, set `output_artifact = Some(CodeModeArtifact { id: path.to_string_lossy().into_owned(), kind: CodeModeArtifactKind::ToolOutput, bytes: body.len() as u64 })`.
   - Push: `Full output retained at {path}; {N} bytes omitted; the omitted region begins at line {L}; call read with path: {path}, offset: {L}, limit: 2000.`
   - On failure, push: `Full output not retained: evidence store write failed; {N} bytes omitted beginning at line {L}.`
2. If `retained_output.is_some()` and `self.evidence.is_none()`, push: `Full output not retained: no evidence store in this agent; {N} bytes omitted beginning at line {L}; rerun with a narrower script or smaller output.` Leave `output_artifact = None`.
3. If `retained_output` is `None`, push nothing. A complete run has no omission.
4. Do not set `outcome.status`. The host's match already produces `Completed`, `Partial`, or `Failed`.
5. Clear `retained_output` after the branch so the body is not cloned into events or the script log.

`script_log_events` continues to record the visible output and child summaries. The retained body lives in the evidence store only.

Read limits: `read` requires `path` and caps at 2,000 lines and 50 KiB per call. An omitted region larger than that needs multiple reads. A single line longer than 50 KiB is marked `too_long` and returns no bytes; in that case the marker says `the omitted region is one long line; read may not return it`. The design does not add a byte-oriented reader.

The evidence directory is a narrow read-boundary exception, like git metadata. The agent permission policy records the active evidence root before each tool batch, so `ReadOutsideRootPolicy::Deny` does not make the advertised recovery path unusable. Only `read`, only paths under the agent's own evidence root; explicit deny rules still apply. Setting the root does not advance the permission revision, so it does not invalidate pending approvals.

### 5. Evidence store facts

`EvidenceStore` is one flat directory per agent: `~/.davinci/agent/evidence`, constructed at `crates/davinci-coding-agent/src/main.rs:1275-1277`. Files are named `<tag>-<uuid>.txt`; a Script body uses `store_in_session`, which prefixes the name with `s<sha256(session)[..12]>-`. The directory is not session-scoped. A path is protected by the normal permission check on `read`, not by session identity. On Unix, the store creates the directory with mode `0700` and each file with mode `0600`; on other platforms it relies on the agent directory's ACLs.

The read-boundary exception is narrower than the store. It applies only to the `read` tool, only to a canonical path under the evidence root, and only to a filename carrying the active session's token. A symlink that resolves outside the root, or to another session's file, does not qualify. Other read-class tools such as `ls` and `find` do not get the exception, so they cannot enumerate the shared directory under `ReadOutsideRootPolicy::Deny`. Explicit deny rules still override the exception.

This design does not add session isolation for the whole evidence store. Script artifacts are session-tagged and the read exception is session-aware; pruning, batch, and other evidence keep the shared layout. If full session isolation is required, it is a separate change.

## Alternatives Considered

### Alternative A: Do nothing

* **Pros**: No code change, no new tests.
* **Cons**: Both demonstrated defects remain.
* **Reason for Rejection**: The two defects are confirmed by code and fixtures. Fixing them is smaller than the analysis that found them.

### Alternative B: Full four-component availability contract

* **Pros**: One vocabulary across pruning, the Governor, Code mode, and Context VM.
* **Cons**: The review found two incompatible schemas, undefined labels, a false session-isolation claim, and a Context VM count that never reaches the model.
* **Reason for Rejection**: Deferred. The one-sentence rule in this document is enough for the two fixes.

### Alternative C: LLM salience judge at compression time

* **Pros**: Could rank by likely task relevance.
* **Cons**: Adds latency, cost, and nondeterminism to a path that must stay cheap.
* **Reason for Rejection**: Out of scope and unverifiable.

### Alternative D: Shared reference namespace

* **Pros**: One idiom for all recovery.
* **Cons**: Does not change selection, breaks persisted `out-` ids, and turns an id into a bearer token.
* **Reason for Rejection**: Orthogonal to the two defects.

### Alternative E: Session-scoped evidence store

* **Pros**: Corrects the isolation claim and limits cross-session reads.
* **Cons**: Adds a store layout change, a migration, and a reader that checks session identity.
* **Reason for Rejection**: Deferred. The document states the current scope honestly instead.

### Alternative F: Dedicated Code mode reader tool

* **Pros**: Explicit argument shape and a session-bound store.
* **Cons**: Adds a tool to the catalog, changes the cached prefix, and duplicates `read` semantics.
* **Reason for Rejection**: The permission-gated `read` of an evidence path is sufficient for line-based recovery.

### Alternative G: Extend `retrieve_output` to script bodies

* **Pros**: Reuses an existing recovery idiom.
* **Cons**: Mixes two authority models. `retrieve_output` reads the Governor's session directory; script bodies are evidence paths.
* **Reason for Rejection**: The authority mismatch is larger than the convenience.

### Alternative H: Fail closed when no evidence store exists

* **Pros**: The contract would never claim retention it cannot provide.
* **Cons**: The protocol output budget is a hard bound. Refusing to truncate either exceeds the prompt or needs a second cap that loses the same bytes.
* **Reason for Rejection**: A degraded marker with a visible reason is honest and executable. It does not claim retention.

### Alternative I: Reuse `ScriptLogStore` for the retained body

* **Pros**: The store is session-bound and already scrubs text.
* **Cons**: Its 1 MiB cap competes with the body size, its reader has no model-facing tool, and its scrubbing changes the bytes.
* **Reason for Rejection**: The evidence store is the existing path used by pruning and already has a model-facing reader.

## Non-Functional Requirements

**Security and privacy.** The retained body is stored byte-exact and protected by the same permission gate as pruning evidence. It is not scrubbed. On Unix the store creates the directory with mode `0700` and each file with mode `0600`. Script artifacts carry a hashed session token in the filename. The store is agent-scoped, not fully session-scoped; the document does not claim otherwise. `read` re-checks path and tool permission on every call. The evidence exception applies only to `read`, only to canonical paths under the store, only to the active session's token, and explicit deny rules still override it.

**Performance and latency.** The Governor selector is a linear scan over the tool result with a bounded output. The Code mode change adds one disk write per over-budget run when a store exists. No model call is added to any reduction path.

**Rollout and rollback.** Phase A ships by default because it fixes demonstrated defects. Code mode remains default off. There is no flag. The rollback is a revert of the two commits. No persisted `out-` id changes meaning. The three new `CodeModeOutcome` fields are skipped by serde, so no session or wire schema changes.

## Additional Resources

* `docs/diagnostics/2026-10-efficiency-diagnosis.md` — the pre-prune equivalence result and its limits.
* `docs/codemode.md` — the Code mode surface and its default-off status.
* The EGM concept brief and review reports are local working artifacts and are not part of this change.

## Future Improvements

* **Pruning investigation.** The forced-trigger test and the opt-in `evidence` profile now exist; a live comparison against the default is still unmeasured.
* **Context VM count.** `omitted_events` is a summarizer-prompt counter.
* **Four-component contract.** Revisit only if a measurement shows the existing trailers are insufficient.
* **Omission-record schema.** One axis, one owner, one type.
* **Dedupe marker.** The Governor cannot see the tool-call id with its current signature.
* **Session-scoped evidence.** Add a session segment and a reader check if isolation is required.
* **Lifecycle harness row.** `context_lifecycle.rs` does not execute tools.
* **Log-view bound.** The all-error log can defeat compression, but no demonstrated loss exists.
* **Byte-oriented reader.** A single line longer than 50 KiB cannot be read back with `read`.

## Detailed Implementation Plan

No production code is included here. Signatures are illustrative. Every file below is modified or created.

### A1. Governor diagnostic priority

**`crates/davinci-coding-agent/src/native_extensions/content_router.rs` (Modify)**

* Add `MAX_SELECTED_ITEMS = 16` and `MAX_DIAGNOSTIC_ITEMS = 12`.
* Add `struct JsonSelection { indices: BTreeSet<usize>, diagnostics_kept: usize, diagnostics_omitted: usize }`.
* Replace `selected_json_indices` with `fn selected_json_indices(items: &[Value]) -> JsonSelection` using the seven-step algorithm in the Technical Plan.
* Change `retrieval_trailer` to accept `diagnostics: Option<(usize, usize)>` and append `; diagnostics kept: {kept}, omitted: {omitted}` before the `full exact output` clause.
* Update all six callers: `build_log_view`, `build_search_results_view`, and `build_bounded_line_view` pass `None`; the JSON array, NDJSON, and JSON object views pass `Some`. The object view sums counts across keys and emits one trailer.
* Unit tests in the existing `#[cfg(test)]` module:
  * 30 items `{"id": i, "status": if i == 25 { "error" } else { "ok" }, "payload": "repetitive payload"}`: assert `item[25]` and `diagnostics kept: 1, omitted: 0`. A fill-only selector selects `{0..14, 29}` and fails.
  * 40 items `{"id": i, "status": if i < 20 { "error" } else { "ok" }, "payload": "p"}`: assert `diagnostics kept: 15, omitted: 5` and `selected.len() == 16`.
  * 30 items `{"id": i, "status": "error"}`: assert `diagnostics kept: 16, omitted: 14` and `selected.len() == 16`.
  * 6 items with one diagnostic: assert all 6 selected and `diagnostics kept: 1, omitted: 0`.
  * 30 items `{"status":"ok"}` only: assert `selected.len() == 16` and `diagnostics kept: 0, omitted: 0`.
  * 29 items `{"status":"ok","payload":"repeated"}` plus one `{"id":25,"status":"error","message":"boom"}`: assert `item[25]` and `diagnostics kept: 1, omitted: 0`.
  * A diagnostic at index 0 with `id=5` and a non-diagnostic at index 10 with `id=5`: assert index 10 is not selected for that identity because the identity set is initialized from the selected diagnostic.
* Do not change `build_log_view`.

**`crates/davinci-coding-agent/src/native_extensions/token_governor.rs` (Modify)**

* Add `rejected: u64` to `KindRetrievalStats` with `#[serde(default)]`.
* In `after_tool`, capture `specialized.is_some()` before the match. In the `_` arm, when a specialized view existed but failed the 10% gate, increment the per-kind `rejected` counter.
* Tests with two separate governors on `tempfile::tempdir()`-backed `OutputStore`s:
  * Lose case: a 6-item JSON array with short payloads. The compact view adds `item[i]:` prefixes and is not 10% smaller than the generic view. Assert `rejected == 1` and `specializedViews == 0`.
  * Win case: a 30-item JSON array with 200-byte payloads. The 16-item view is well below 90% of the generic view. Assert `rejected == 0` and `specializedViews == 1`.
* Update the existing `governor_status_reports_content_routing` only if its 30-item short-payload fixture now follows the lose case. State the expected `specializedViews` and `rejected` values for that fixture.

### A2. Codemode pre-truncation storage

**`crates/davinci-agent/src/codemode/projection.rs` (Modify)**

* Add `pub struct Truncated { pub text: String, pub omitted_bytes: usize, pub omitted_start_line: usize }`.
* Change `truncate_head_and_tail` to return `Truncated`. Rewrite the destructuring at `projection.rs:207`.
* Add `omitted_start_line: Option<usize>` and `retained_body: Option<String>` to `ScriptOutput`.
* In `assemble_script_output`, set `retained_body = Some(collected)` and `omitted_start_line = Some(truncated.omitted_start_line)` only when truncating. A complete result sets both to `None`.

**`crates/davinci-agent/src/codemode/types.rs` (Modify)**

* Add `#[serde(skip)] pub retained_output: Option<String>`, `#[serde(skip)] pub omitted_bytes: usize`, and `#[serde(skip)] pub omitted_start_line: Option<usize>` to `CodeModeOutcome`.
* Do not add `Default`. Add the fields explicitly at every construction site.

**`crates/davinci-coding-agent/src/codemode_host/supervisor.rs` (Modify)**

* In the completion outcome, set the three fields from `output`.
* Leave the status match unchanged.
* Do not add a path.
* Rewrite the four `truncate_head_and_tail` unit test call sites at `1495, 1511, 1528, 1541` for the `Truncated` return type.

**`crates/davinci-agent/src/codemode/execution.rs` (Modify)**

* Persist `retained_output` to `EvidenceStore` with tag `script-output` when a store exists.
* Set `output_artifact` with the path as `id`, `CodeModeArtifactKind::ToolOutput`, and the body length.
* Push the retained, write-failed, or no-store marker exactly as specified in Technical Plan section 4.
* Guard the no-store branch with `retained_output.is_some()`. A complete run pushes nothing.
* Do not set `outcome.status`.
* Clear `retained_output` after the branch.

**Construction sites to update (13):**

| File | Lines | Field values |
| --- | --- | --- |
| `crates/davinci-agent/src/codemode/execution.rs` | 923 | `None`, `0`, `None` |
| `crates/davinci-coding-agent/src/codemode_host/supervisor.rs` | 930 | from `output`: body, bytes, line |
| `crates/davinci-coding-agent/src/codemode_host/supervisor.rs` | 1090 | `None`, `0`, `None` |
| `crates/davinci-agent/tests/common/codemode.rs` | 148, 501, 577, 1404, 1524, 2530, 2544 | `None`, `0`, `None` |
| `crates/davinci-agent/tests/common/codemode_mcp.rs` | 205 | `None`, `0`, `None` |
| `crates/davinci-agent/tests/first_request_anatomy.rs` | 76 | `None`, `0`, `None` |
| `crates/davinci-agent/tests/codemode_contract.rs` | 112 | `None`, `0`, `None` |

`crates/davinci-coding-agent/src/main_tests.rs:570` is the return type of `UnrunCodeModeHost::execute`, not a struct literal. It needs no field.

**Production and unit call sites to rewrite for `Truncated`:**

| File | Lines |
| --- | --- |
| `crates/davinci-agent/src/codemode/projection.rs` | 207 |
| `crates/davinci-coding-agent/src/codemode_host/supervisor.rs` | 1495, 1511, 1528, 1541 |

The four `codemode_projection.rs` lines (`91, 96, 106, 115`) call `assemble_script_output`, whose signature does not change. They need no rewrite for `Truncated`; the new assertions are separate tests.

**`crates/davinci-agent/tests/codemode_projection.rs` (Modify)**

* Add a case: 400 lines of 100 bytes, needle at line 250, `assemble_script_output(&items, 16 * 1024, 1024 * 1024)`. Assert `retained_body == Some(collected)`, `text` does not contain the needle, `text` contains the omission marker, and `omitted_start_line == Some(110)` for the current marker wording. Recompute the expected line if the marker wording changes.
* Add a complete-result case: `retained_body` and `omitted_start_line` are `None`.

**`crates/davinci-agent/tests/common/codemode.rs` (Modify)**

This file is `#[path]`-included into the lib's `#[cfg(test)]` module, already has `configured_agent()`, already calls `run_codemode`, and already sets `agent.evidence`.

* Store case: `let store = EvidenceStore::new(dir.path()); agent.evidence = Some(store.clone());` Run a script that produces an over-budget body with a needle in the middle. Assert `details["codemode"]["outputArtifact"]` is present, its `id` starts with `store.dir()`, `hostNotes` contains the path and the line, and `std::fs::read_to_string(artifact_id)` contains the needle.
* No-store case: set `agent.evidence = None` and use a stub host that returns `CodeModeOutcome { status: Partial, output_complete: false, retained_output: Some(body), omitted_bytes: N, omitted_start_line: Some(L), .. }`. Assert the note says `no evidence store in this agent`, the artifact is `None`, and the status is `Partial`.
* Complete no-store case: stubbed `output_complete: true`, `retained_output: None`. Assert no marker and the status is `Completed`.
* Do not put these in `codemode_contract.rs`; its `FixedHost` is `unreachable!()`, `ReadOnly` and `Controlled` require a trusted journal, and `run_codemode` is `pub(crate)`.

### Phase B. Omission metadata

Phase A already names the quantity and the recovery reference. Phase B makes the class and error flag explicit and surfaces the Context VM fold count:

* The pruning placeholder says `class: tool_output`; it already names the byte and line counts, the call id, the outcome, and the evidence path.
* The Governor trailer says `class: diagnostic` and reports diagnostics kept and omitted; it already names the output id.
* The Codemode marker says `class: script_output; error: <bool>`; it already names the omitted bytes, the first omitted line, and the evidence path. The error flag is true for a failed, cancelled, or recovery-required run, or when a child failed.
* The Context VM fold notice reports how many events the fold could not represent and names them as retrievable by source reference. The count also stays in the summarizer prompt.

No new message type, store, or telemetry schema is added.

### Phase C. Pruning investigation

* The forced-trigger test builds an agent whose estimate is over the start line, prunes an old large result with `keep_recent = 0`, and asserts the placeholder names the call id and outcome while the stored body still contains the needle. The mechanism can fire; the earlier 17-run corpus recorded zero prune events, so that corpus could not measure it.
* `DAVINCI_PRUNE_PROFILE=evidence` selects `PruneOrder::ErrorsLast`. The eligible older results are identified first, so the newest `keep_recent` are always protected; errors-last ordering applies only within that subset. The default remains `PruneOrder::OldestFirst`.
* The profile preserves the route's start and target fractions: 65%/35% on cache-sensitive routes, 50%/35% otherwise. It changes ordering only, so a comparison isolates ordering from the trigger.
* The profile is an experimental arm, not a default. A live comparison against the default is Phase D.

### Phase D. Measurement

* Pre-registration: `docs/perf/evidence-availability/preregistration.md` fixes the arms, corpus, repetitions, metrics, pass rule, stop rules, and rollback before any live run.
* Primary metric: the deterministic needle harness at `crates/davinci-coding-agent/tests/evidence_availability.rs` checks that the Governor and Code mode retain the needle and that the first recovery returns it, with a token estimate per recovery. It runs on every PR with no model call.
* Trigger coverage: the 17-task corpus recorded zero pruning events. A pruning comparison is admissible only after adding at least three long-context tasks per arm that record `prunedResults > 0`; otherwise the pruning arm is void.
* Diagnostic admission budget: record the diagnostic view, generic digest, and original bytes per task. A view above 90% of the original is flagged and cannot support a compression-default promotion.
* Secondary metrics: verified task success, total and uncached input, compactions and summarizer tokens, first-attempt recovery success, tokens per resolved recovery, and prefix breaks by cause.
* The 17-task corpus is directional, not powered for a 3 percentage-point success difference. Live runs require separate approval.

### Verification Plan

* **Build:** `cargo build --workspace --offline --locked`
* **Governor selector:** `cargo test -p davinci-coding-agent content_router`
* **Governor counters:** `cargo test -p davinci-coding-agent token_governor`
* **Codemode projection:** `cargo test -p davinci-agent --test codemode_projection`
* **Codemode host unit tests (`Truncated`):** `cargo test -p davinci-coding-agent --lib codemode_host`
* **Codemode retained-body cases:** `cargo test -p davinci-agent --lib codemode`
* **Struct construction:** `cargo test -p davinci-agent --lib` (compiles the `#[path]`-included common modules) or `cargo test -p davinci-agent --no-run`
* **Formatting:** `cargo fmt --all --check`
* **Linting:** `cargo clippy --workspace --all-targets --offline`
* **Full gate:** `cargo test --workspace --offline --locked`

The A1 and A2 tests must fail before the corresponding fix and pass after it. Do not use `--test codemode_supervisor` or `--test codemode_supervisor_budget` as gates; every test in those files is `#[ignore]` and env-gated.

### Rollout and rollback

* Both changes ship by default. Code mode remains default off.
* No flags, no schema migration, no tool-catalog change.
* Rollback is a revert of the two commits. Old sessions are unaffected because the three new fields are skipped by serde and no existing id changes meaning.
* The evidence store's 7-day TTL applies to the retained body. A read after eviction returns the normal missing-file message; the marker still names the path and line, so the failure is visible rather than silent.
