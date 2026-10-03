# Governor Quality Feedback Addendum Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Complete the Governor vNext quality requirements that sit on top of the main Governor plan: structured JSON diversity and near-term retrieval feedback.

**Architecture:** Extend only the existing `content_router` and `TokenGovernor` telemetry. Reuse stored output metadata and the Governor's existing monotonic `tool_calls` sequence; no model call or adaptive self-modification is introduced.

**Tech Stack:** Rust 1.83, serde/serde_json, existing Token Governor/content router.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- Execute after Tasks 1–3 of `2026-09-14-davinci-governor-context-verification.md`.
- Exact originals remain recoverable.
- Metrics are observational only; they do not automatically change compression thresholds.

---

### Task 1: Make JSON specialized views preserve deterministic record diversity

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/content_router.rs`

**Interfaces:**

Add a private deterministic record signature helper:

```rust
fn json_signal_signature(value: &serde_json::Value) -> Vec<String>;
```

Inspect these keys when present:

```text
id
name
path
file
line
status
type
error
message
```

Selection rules:
- preserve first and last records;
- preserve up to four error/warning records;
- prefer records introducing a new signal signature/value for `status`, `type`, `path/file`, or error state;
- fill remaining bounded slots with deterministic positional representatives;
- keep original item indexes in rendered output.

- [ ] **Step 1: Write failing diversity test**

Create a 50-item JSON array with three statuses, two paths, one warning, and one error. Assert specialized output includes representatives from each distinct status/path plus warning/error, not merely fixed indices.

- [ ] **Step 2: Run and confirm failure**

```bash
cargo test -p davinci-coding-agent json_specialized_view_preserves_signal_diversity
```

- [ ] **Step 3: Implement bounded signal-aware selection**

Use `BTreeSet` for selected indexes/signatures so output is deterministic.

- [ ] **Step 4: Run existing JSON/content-router tests**

```bash
cargo test -p davinci-coding-agent json_specialized_view_preserves_signal_diversity
cargo test -p davinci-coding-agent content_router_classifies_and_reduces_supported_shapes
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(governor): preserve structured JSON signal diversity"
```

---

### Task 2: Track retrievals that happen soon after compression

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`

**Interfaces:**

Extend each stored manifest entry with:

```rust
#[serde(default)]
pub compressed_at_tool_call: usize;
```

Extend per-kind stats with:

```rust
pub retrievals_within_3_tool_calls: u64;
```

Define near-term deterministically:

```rust
current_tool_calls.saturating_sub(entry.compressed_at_tool_call) <= 3
```

`retrieve_output` itself is a tool call only if the current Governor implementation already increments `tool_calls` for it in the same execution path; otherwise use the Governor sequence as it exists and document the exact sequence semantics in the field comment.

- [ ] **Step 1: Write failing near-term retrieval test**

Compress a log, retrieve immediately, and assert both total retrievals and `retrievals_within_3_tool_calls` increment for the log kind.

- [ ] **Step 2: Add delayed retrieval test**

Compress another output, execute at least four intervening Governor-observed tool results, then retrieve it and assert total retrieval increments but near-term retrieval does not.

- [ ] **Step 3: Run and confirm missing metric**

```bash
cargo test -p davinci-coding-agent governor_tracks_near_term_retrieval_feedback
```

- [ ] **Step 4: Implement using stored compression sequence metadata**

Do not reparse content or infer timing from wall clock.

- [ ] **Step 5: Run status/telemetry tests and commit**

```bash
cargo test -p davinci-coding-agent governor_tracks_near_term_retrieval_feedback
cargo test -p davinci-coding-agent governor_status
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "feat(governor): measure near-term retrieval feedback"
```
