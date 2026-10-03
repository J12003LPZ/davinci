# Semantic Code Navigation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide capability-aware symbol navigation and safe rename previews with reliable text-search fallback.

**Architecture:** Add a bounded native LSP 3.17-compatible client behind a host-owned semantic service. Reuse existing file/tool permission and process-control boundaries; return typed locations and diagnostics, and translate explicitly accepted rename edits into ordinary guarded patch operations.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Semantic Code Navigation — supplied source specification](2026-09-07-davinci-feature-specs/10-semantic-code-navigation.md)

## Global Constraints

- **Rust 1.83.0**, edition 2021; dependencies use exact `=x.y.z` pins. Prefer existing workspace dependencies. No toolchain, product-version, or provider wire-identity change is part of these features.
- Work only in active `crates/*`; `vendor/davinci` is reference-only and `packages/*` is not the implementation reference. Preserve upstream system, compaction, and branch prompt constants.
- Tests live in inline `#[cfg(test)] mod tests`; only `crates/davinci-parity/fixtures` may contain external fixtures. Tests are offline and fixture-controlled, with isolated temporary session/config roots. No provider calls, package downloads, browser downloads, or actual external actions in tests.
- Preserve Windows and Unix behavior, existing CLI aliases, JSON/print/RPC compatibility, JSONL sessions, legacy `.pi` discovery, and the five modes: Manual, Accept Edits, Plan Mode, Auto Mode, Always Approve.
- Deny rules, project trust, role allowlists, task scope, ownership, and platform restrictions are never relaxed by a UI choice. A prompt or permission mode is not an operating-system sandbox.
- Keep Living Plan, execution Tasks, user Decisions, and execution Evidence separate. **Claude-style dynamic workflows are intentionally excluded.** Existing workflow code is not a replacement for the graph controller.
- Preserve the graph's DAG, one mutation-capable writer at a time, bounded fan-out, mandatory real verification, applicable security, mode-specific review guarantees, and exact replay provenance. Do not turn disjoint scopes into permission for concurrent graph writers.
- Shared contracts and failure semantics in [the runtime contract](2026-09-07-00-shared-runtime-contracts.md) are part of this plan. Proposed bounds below are engineering decisions, not values mandated by the source mockups.
- Existing Simple graphs require verification and applicable security but do not currently require a Reviewer; Standard/Complex review guarantees must remain intact. Strengthening Simple review would be a separate explicit product decision.

---

## Execution notes

This is a **proposed implementation plan**, prepared against `main` at `9c42820`; it does not claim the feature is implemented or that the user has approved an execution design. Read the source spec, repository `AGENTS.md`/`CLAUDE.md`, and shared contract before execution. Re-resolve symbol anchors if HEAD changes. Existing paths below were found during reconnaissance; paths marked **Create** are proposals.

Each numbered task owns an independently reviewable deliverable. Its Rust snippets define a small executable contract/oracle, not the complete integrated feature; implement the surrounding adapters and all listed scenarios as part of that task. New symbols in snippets are proposed, not claimed existing APIs. Integrate `mod` declarations and imports before running the red test; a filter matching zero tests is **not** a pass. Merge snippets into one inline test module per source file. Each task's extra cases are required, not optional stretch goals.

Use an isolated execution worktree later, preserve unrelated user edits, and review each diff before its task commit. The documentation-generation session does not run the repository application test suite or change application code. Any separately checked standalone examples are documentation checks, not evidence that the feature works in Davinci.

## Repository fit and existing behavior

Active Rust searches found no native LSP client, manager, or semantic navigation tools; this is a bounded negative over the inspected active crates, not a statement about external plugins. Existing tools.rs read/grep/find/ls functions provide working text fallbacks, and vector memory is not a code-symbol index. The repository already uses serde_json and process/job infrastructure, so a minimal stdio JSON-RPC client can reuse them without adopting an unrelated asynchronous runtime.

## Scope, alternatives, and chosen approach

Choose lazy, per-workspace/server LSP sessions keyed by canonical root and configuration digest. Do not launch a server at startup or infer semantic authority from tool_search, which discovers tools rather than symbols. A language server is executable project-adjacent code: trust and execution policy apply even to a navigation query. Disable server features that execute build scripts/proc macros when not authorized, and still refuse launch when the process cannot meet the contract's actual constraints. Use advertised server capabilities; unsupported call hierarchy or rename returns an explicit unavailable result with text fallback where meaningful.

**Dependencies and implementation order:** Uses 05 guarded effects,06 source/evidence manifests,07 process leases, and 09 resources. Pure transport/position tests and read-only service traits can develop independently after contracts are frozen.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F10-1 | Support definitions, references, outline, diagnostics, rename preview, and call hierarchy where advertised. |
| F10-2 | Keep ordinary file/text search available when semantic support is unavailable or incomplete. |
| F10-3 | Manage server startup, capabilities, cancellation, framing, resource bounds, and workspace trust safely. |
| F10-4 | Bind locations/diagnostics/edits to document versions and source fingerprints, including Unicode/CRLF handling. |
| F10-5 | Rename changes require normal permissions/contracts/checkpointing and verification; LSP output is not proof of correctness. |

## Data and behavioral contracts

New native host modules `semantic/{mod,transport,manager,documents,tools,rename}.rs`. Core defines a `SemanticService: Send + Sync` boundary injected into tool context, avoiding a davinci-agent → davinci-coding-agent dependency. Proposed tools: `code_definition`, `code_references`, `code_outline`, `code_diagnostics`, `code_call_hierarchy`, `code_rename_preview`; applying edits uses the existing patch path, not an unrestricted server callback.

`SemanticResult { request_id, server_identity, capability, document_version, source_manifest, locations, diagnostics, partial, fallback_reason }`. `RenamePreview { id, server_identity, document_versions, source_manifest, edits, affected_paths, digest }`. LSP messages use Content-Length byte framing; proposed bounds are 8 MiB per message, 64 pending requests, 2,000 returned locations with explicit truncation, and a configurable request deadline charged to the root task. Negotiate position encoding, defaulting to UTF-16 when unspecified; never treat protocol character offsets as byte indexes. Unsupported server-initiated workspace/applyEdit or executeCommand is denied unless routed through a new explicit normal approval/contract path; V1 denies these unsolicited effects.

## User experience and mode behavior

A symbol view lists definition, references, diagnostics with freshness, and advertised hierarchy support. Rename always opens a preview with affected files and verification requirements. Unavailable semantic support is plainly labeled and does not disable ordinary code search.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Service contract, capabilities, and fallback

**Covers:** F10-1, F10-2, F10-3

**Test placement:** `crates/davinci-agent/src/semantic.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/semantic.rs` — host-neutral semantic trait and result DTOs
- **Create:** `crates/davinci-coding-agent/src/semantic/mod.rs` — native service adapter
- **Modify:** `crates/davinci-agent/src/tools.rs` — semantic tool schemas and fallback

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f10_capability_fallback() {
    assert_eq!(semantic_route(false, false, false), "text_fallback");
    assert_eq!(semantic_route(true, false, false), "unsupported");
    assert_eq!(semantic_route(true, true, true), "stale_requery");
    assert_eq!(semantic_route(true, true, false), "semantic");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f10_capability_fallback -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn semantic_route(server_available: bool, supported: bool, stale: bool) -> &'static str {
    if !server_available { "text_fallback" } else if !supported { "unsupported" }
    else if stale { "stale_requery" } else { "semantic" }
}
```

Register only tools backed by the semantic service or clear fallback. Definition/references/outline fallback uses existing authorized read/grep/find and labels results textual, not exact semantic references. Diagnostics unavailable is not zero errors; rename unavailable is not a safe no-op rename; call hierarchy unsupported stays unsupported. Resolve file URI/path scope before dispatch and return bounded structured locations with snippets retrieved through normal read policy. Advertise operation-specific capabilities rather than one all-or-nothing semantic flag.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: no server installed; unsupported hierarchy; empty legitimate references; stale server; server available but workspace untrusted; text fallback cancellation; unknown language.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/semantic.rs" "crates/davinci-coding-agent/src/semantic/mod.rs" "crates/davinci-agent/src/tools.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: semantic-code-navigation task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Bounded JSON-RPC framing and lifecycle

**Covers:** F10-3

**Test placement:** `crates/davinci-coding-agent/src/semantic/transport.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/semantic/transport.rs` — stdio framing/request table/cancellation
- **Create:** `crates/davinci-coding-agent/src/semantic/manager.rs` — initialize/shutdown/server leases

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f10_length_counts_bytes() {
    let payload = r#"{"x":"é"}"#;
    assert_eq!(lsp_frame(payload), b"Content-Length: 10\r\n\r\n{\"x\":\"\xc3\xa9\"}".to_vec());
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f10_length_counts_bytes -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn lsp_frame(json: &str) -> Vec<u8> {
    let mut out = format!("Content-Length: {}\r\n\r\n", json.len()).into_bytes();
    out.extend_from_slice(json.as_bytes());
    out
}
```

Implement incremental header/body parsing over partial reads with byte length checks before allocating. Reject missing/duplicate/invalid lengths, oversize frames, invalid JSON, and invalid response IDs. Keep stderr separate from protocol stdout, drain both to avoid blocking, and sanitize logs. Initialize once, record advertised capabilities/encoding, send initialized, synchronize documents, and use shutdown→exit with bounded forced cleanup. Cancel by request ID with $/cancelRequest, but retain a tombstone so late responses cannot satisfy another request. Handle server-initiated requests with a bounded explicit allowlist; unsolicited workspace edits are rejected. No new Tokio runtime is required for this stdio service.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: UTF-8 byte length; split headers/body; multiple frames per read; malformed length; out-of-order replies; cancel then late reply; stderr flood; server exit; request deadline; max pending backpressure.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/semantic/transport.rs" "crates/davinci-coding-agent/src/semantic/manager.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: semantic-code-navigation task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Document versions and correct position conversion

**Covers:** F10-1, F10-4

**Test placement:** `crates/davinci-coding-agent/src/semantic/documents.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/semantic/documents.rs` — open/change/close tracking and position conversion
- **Create:** `crates/davinci-coding-agent/src/semantic/tools.rs` — source-bound query results

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f10_utf16_boundaries() {
    assert_eq!(utf16_to_byte("a😀b", 1), Some(1));
    assert_eq!(utf16_to_byte("a😀b", 2), None);
    assert_eq!(utf16_to_byte("a😀b", 3), Some(5));
    assert_eq!(utf16_to_byte("a😀b", 4), Some(6));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f10_utf16_boundaries -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn utf16_to_byte(line: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in line.char_indices() {
        if units == offset { return Some(byte); }
        units += ch.len_utf16();
        if units > offset { return None; }
    }
    if units == offset { Some(line.len()) } else { None }
}
```

Track a monotonic document version and exact content hash for each open authorized file. Send didOpen/didChange/didClose according to server sync capability, preserving the current on-disk/task edit state. Convert UTF-16 or negotiated UTF-8 offsets with line-ending awareness; edit ranges that split a surrogate/code point or are out of bounds fail safely. Stale diagnostics are displayed with their originating version, not merged as current. Normalize file URIs with the existing URL/path utilities and reject non-file or out-of-root locations unless the normal read boundary explicitly allows them.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: CRLF and bare CR; astral Unicode; combining marks; empty final line; same-size edit; diagnostic arrives after document close; URI percent encoding; Windows drive/UNC; generated file changed midquery.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/semantic/documents.rs" "crates/davinci-coding-agent/src/semantic/tools.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: semantic-code-navigation task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Trusted server launch and task resource integration

**Covers:** F10-2, F10-3

**Test placement:** `crates/davinci-coding-agent/src/semantic/manager.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/semantic/manager.rs` — explicit server config and process policy (introduced by F10 Task 2; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/settings.rs` — trusted server configuration
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/control.rs` — owned server process lease (introduced by F07 Task 1; do not recreate it)

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f10_server_launch_authority() {
    assert!(!server_launch_allowed(false, true, true));
    assert!(!server_launch_allowed(true, false, true));
    assert!(!server_launch_allowed(true, true, false));
    assert!(server_launch_allowed(true, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f10_server_launch_authority -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn server_launch_allowed(project_trusted: bool, execution_allowed: bool, effects_contained: bool) -> bool {
    project_trusted && execution_allowed && effects_contained
}
```

Server config names an explicit executable/argv, supported languages, root detection, environment allowlist, and policy profile; do not execute a repository string through a shell. Record executable/version identity and lazy-start only after authorization. No automatic server installation or package-manager invocation. For Rust, distinguish navigation from build-script/proc-macro execution; disabling server settings is defense in depth, not a substitute for effect containment. Stop idle servers by bounded policy, charge startup/requests to resource ledgers, and isolate projects/profiles so one repository cannot inspect another's documents.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: project file supplies malicious executable; PATH changes after config approval; server spawns build script; inherited secrets; two roots; missing binary; stop during query; budget deadline; repeated launch crash backoff bounded.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/semantic/manager.rs" "crates/davinci-coding-agent/src/settings.rs" "crates/davinci-agent/src/runtime/control.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: semantic-code-navigation task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Rename preview and guarded transactional apply

**Covers:** F10-1, F10-4, F10-5

**Test placement:** `crates/davinci-coding-agent/src/semantic/rename.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/semantic/rename.rs` — versioned WorkspaceEdit preview
- **Modify:** `crates/davinci-agent/src/apply_patch.rs` — normal authorized edit translation
- **Create:** `crates/davinci-tui/src/davinci/views/semantic.rs` — navigation and rename review

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f10_rename_preconditions() {
    assert!(rename_apply_allowed("a", "a", true, true, false));
    assert!(!rename_apply_allowed("a", "b", true, true, false));
    assert!(!rename_apply_allowed("a", "a", false, true, false));
    assert!(!rename_apply_allowed("a", "a", true, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f10_rename_preconditions -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn rename_apply_allowed(preview: &str, current: &str, explicit_user: bool,
    contract_allows: bool, overlapping_edits: bool) -> bool {
    preview == current && explicit_user && contract_allows && !overlapping_edits
}
```

Use prepareRename where supported, then collect WorkspaceEdit into a read-only preview. Validate every affected document version, URI, range, and scope; reject overlapping edits and unsupported resource operations in V1. Apply accepted text edits in descending byte-offset order per file through the existing patch transaction, permission/contract guards, ownership lease, and checkpoint service. A preview ID binds exact edits and source; changed files require a new preview and consent. Requery diagnostics and run contract-required tests afterward, creating feature06 evidence. No diagnostic count of zero alone proves semantic correctness.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: cross-file rename; protected file; resource create/delete request; rename outside root; conflicting edits; partial apply failure; file changes after preview; same spelling different symbol; cancelled confirmation; postrename tests fail.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/semantic/rename.rs" "crates/davinci-agent/src/apply_patch.rs" "crates/davinci-tui/src/davinci/views/semantic.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: semantic-code-navigation task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Server unavailable/unsupported | Explicit capability result and meaningful text fallback. |
| Invalid protocol/oversized frame | Terminate/restart within bounded policy; no unbounded allocation. |
| Stale version or invalid position | Requery or reject edit, never guess byte ranges. |
| Unsolicited server effect | Deny or require a separate normal authorized operation. |
| Rename verification fails | Preserve evidence and task incomplete/stale state; no semantic-correctness claim. |

## Persistence, migration, and rollback

Server configuration is additive and trusted-only. Old sessions simply have no semantic capability. No server binaries are installed by migration. Unknown config fields/versions that affect execution fail closed. Rollback unregisters semantic tools and stops owned servers while preserving normal text tools and edit journals.

## End-to-end acceptance and release gates

Use an in-process fake LSP transport with canned capabilities and out-of-order messages to exercise every operation offline. Include a Unicode/CRLF cross-file rename preview, change one file before applying, and require a stale-preview refusal. With no server, read/grep/find still work and diagnostics are unavailable rather than falsely clean. A malicious server cannot execute commands, widen scope, or modify files through unsolicited callbacks.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.

Protocol baseline: Microsoft LSP 3.17 specification and Microsoft vscode-languageserver-node position types. See shared references for exact URLs. The chosen compatibility baseline is not a claim that 3.17 is the newest protocol version.
