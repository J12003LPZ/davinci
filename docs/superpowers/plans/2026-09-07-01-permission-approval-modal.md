# Permission Approval Modal Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide one policy-owned approval interaction across all hosts, with legal scoped grants and no input or consent leakage.

**Architecture:** Extend the existing permission evaluation and approval bridge; create a host-neutral challenge DTO and a shared TUI modal model. The engine produces legal choices and revalidates the selected challenge immediately before execution; UI code cannot create grants.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Permission Approval Modal — supplied source specification](2026-09-07-davinci-feature-specs/01-permission-approval-modal.md)

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

`crates/davinci-agent/src/permission.rs` already defines five `PermissionMode` variants and centralized tool classes. `permission_risk.rs` performs sensitive-path and command-risk analysis. `turn.rs` is the execution gate, while `permissions.rs`, `davinci_interactive.rs`, `main.rs`, and `rpc.rs` bridge host interactions. This is a consistency and authorization-hardening feature, not a new permission system. In particular, Auto Mode and Always Approve are already distinct in the current source, despite older guidance describing only four modes.

The active callback is `ToolApprover(Arc<dyn Fn(&ToolApprovalRequest) -> ToolApprovalDecision + Send + Sync>)`; request and four-choice reply types already exist in `permission.rs:643–674`. The native host already uses a channel and `Overlay::Ask`. The library policy default is AlwaysApprove for compatibility, while CLI configuration defaults to Manual. Plan Mode currently allows authorized Network research as well as Read operations; none of this plan changes that mode matrix.

## Scope, alternatives, and chosen approach

Choose a typed challenge/reply above the current approver abstraction. Keeping separate string prompts in each host would retain inconsistent choices; implementing classification in a Ratatui widget would create a bypass. Preserve the current renderer and theme, using a bordered, content-sized overlay rather than copying a different application's visual identity. Persistent scopes are generated from a policy-approved canonical target, never from editable model text. High-risk destructive, secret, privileged, or externally visible actions initially offer only one-shot decisions and denials; an explicit deny or hard boundary offers no grant at all.

**Dependencies and implementation order:** Can start independently; its typed interaction bridge is reused by feature 02 and feature 05. Integrate permission changes serially with task-contract enforcement.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F01-1 | Every policy Ask gets one typed challenge that explains action, target, reason, and stopping mode/policy. |
| F01-2 | The policy engine controls legal one-shot/session/project/denial choices; high-risk actions do not automatically gain persistent choices. |
| F01-3 | Up/Down and numbered options select, Enter explicitly confirms, and Esc denies/cancels. |
| F01-4 | The modal traps Shift+Tab, ordinary composer keys, paste, and voice insertions without losing the preexisting draft. |
| F01-5 | All hosts fail closed on stale replies, cancellation, transport loss, invalid scope, or storage failure; persistent grants remain narrowly scoped. |

## Data and behavioral contracts

Proposed DTOs in `approval.rs`: `ApprovalChallenge { schema_version: u16, id: Uuid, call_id: String, action_digest: String, policy_revision: u64, contract_revision: Option<u64>, mode: PermissionMode, action_label: String, display_target: String, reason: String, legal_choices: Vec<ApprovalChoice>, expires_at_ms: u64 }`; `ApprovalChoice { id: String, scope: GrantScope, label: String }`; `GrantScope = Once | Session | Project | Deny | DenyWithInstructions`; `ApprovalReply { challenge_id: Uuid, choice_id: String, instructions: Option<String> }`. Keep grant issuance private to the trusted host/engine bridge.

Bind the digest to canonical tool name, complete arguments, normalized target, actor/run, and relevant source revision. On reply, re-run deny/trust/role/contract checks and compare challenge revision, expiry, and argument digest. Consume a Once grant atomically for that call only. Session grants are session-local and die on reload. Project grants write only the legal narrowly-scoped rule to trusted project settings using an atomic update; failure does not imply approval. Neither recommendation text nor option position carries authority. Queue multiple simultaneous requests in source order, capped at 16 pending challenges; backpressure rather than drop.

## User experience and mode behavior

The action block precedes the explanation and choices. A persistent choice is not the default. Esc visibly reports denied/cancelled. The user can scroll the action details, but no interaction changes the global permission mode until the challenge has closed.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Policy-owned legal choices

**Covers:** F01-1, F01-2

**Test placement:** `crates/davinci-agent/src/approval.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/approval.rs` — challenge and legal-scope types
- **Modify:** `crates/davinci-agent/src/permission.rs` — reuse risk classification and legal grant construction

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f01_legal_scope() {
    assert_eq!(offer_scopes(false, true, true), vec!["once", "session", "project", "deny", "deny_with_instructions"]);
    assert_eq!(offer_scopes(true, true, true), vec!["once", "deny", "deny_with_instructions"]);
    assert_eq!(offer_scopes(false, false, false), vec!["once", "deny", "deny_with_instructions"]);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f01_legal_scope -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn offer_scopes(high_risk: bool, session_legal: bool, project_legal: bool) -> Vec<&'static str> {
    let mut out = vec!["once"];
    if !high_risk && session_legal { out.push("session"); }
    if !high_risk && project_legal { out.push("project"); }
    out.extend(["deny", "deny_with_instructions"]);
    out
}
```

Call this helper only after the engine has returned Ask, never for Deny. Map existing risk classifications to persistent-scope eligibility in engine code. Normalize URL origin, port, path subject, and tool alias before deriving grant scope; do not let a display string become a wildcard rule. Return a bounded explanation with control characters removed for terminal display, retaining exact canonical arguments in the authenticated challenge. Reuse the current settings rule parser rather than invent a second glob grammar.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: explicit deny plus Always Approve; project untrusted; redirect to a different origin; port mismatch; protected file; shell chain containing a privileged segment; malformed action has no grant.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/approval.rs" "crates/davinci-agent/src/permission.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: permission-approval-modal task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Replay-resistant challenge resolution

**Covers:** F01-2, F01-5

**Test placement:** `crates/davinci-agent/src/approval.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/approval.rs` — pending challenge table and grant consumption (introduced by F01 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/turn.rs` — check policy again before dispatch

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f01_stale_reply() {
    assert!(reply_matches("c1", "c1", "h1", "h1", 7, 7, 10, 11));
    assert!(!reply_matches("c1", "c1", "h1", "h2", 7, 7, 10, 11));
    assert!(!reply_matches("c1", "c1", "h1", "h1", 7, 8, 10, 11));
    assert!(!reply_matches("c1", "c1", "h1", "h1", 7, 7, 11, 11));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f01_stale_reply -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn reply_matches(challenge: &str, reply: &str, issued_digest: &str, current_digest: &str,
    issued_revision: u64, current_revision: u64, now_ms: u64, expires_ms: u64) -> bool {
    challenge == reply && issued_digest == current_digest
        && issued_revision == current_revision && now_ms < expires_ms
}
```

Use host-authenticated response routing, an immutable challenge map, and consumed-request IDs. Validate the returned choice against the exact issued legal list. The helper checks identity/freshness only; authorization still checks actor identity, pending/unused state, and all current policy boundaries. A changed tool argument or contract revokes the old challenge and issues a fresh one. Do not hold the native-extension or registry mutex while awaiting UI input. Store denial instructions as bounded user guidance, not an authorization rule.

Propagate a rejection from `RuntimeHandle::emit_decision(PermissionRequested)` instead of the existing ignored `let _ =` result in `permission_denial`. Validate challenge authority after the pre-tool hook has finalized arguments and recheck it immediately before execution. A returned cached ledger result is not a newly approved execution.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: duplicate Enter; reply for another run; forged choice ID; expired challenge; source changed while modal open; cancelled parent; callback panic; queue capacity backpressure. A rejecting decision subscriber prevents callback/execution; changed arguments invalidate the challenge.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/approval.rs" "crates/davinci-agent/src/turn.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: permission-approval-modal task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Modal input ownership and rendering

**Covers:** F01-1, F01-3, F01-4

**Test placement:** `crates/davinci-tui/src/davinci/views/approval_modal.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-tui/src/davinci/views/approval_modal.rs` — bounded overlay renderer
- **Modify:** `crates/davinci-tui/src/davinci/model.rs` — modal state and draft preservation
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — input precedence and reply emission
- **Modify:** `crates/davinci-tui/src/davinci/views/ask.rs` — extend existing Ask rendering rather than adding a competing overlay

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f01_modal_traps_mode_key() {
    assert_eq!(approval_key(true, "shift_tab"), "consume");
    assert_eq!(approval_key(true, "escape"), "deny");
    assert_eq!(approval_key(true, "enter"), "confirm");
    assert_eq!(approval_key(false, "shift_tab"), "delegate");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-tui --offline f01_modal_traps_mode_key -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn approval_key(open: bool, key: &str) -> &'static str {
    if !open { return "delegate"; }
    match key {
        "escape" => "deny", "enter" => "confirm", "up" => "previous",
        "down" => "next", "1" | "2" | "3" | "4" | "5" => "select_number",
        _ => "consume",
    }
}
```

Put modal routing ahead of mode cycling, slash submission, composer editing, global shortcuts, paste, and voice insertion. Keep the draft buffer/caret in place, not copied into the modal. Arrow/number navigation never commits; Enter is the only accept action. Deny-with-instructions opens a modal-local editor and then explicit confirmation. Handle Ctrl+C as a cancellation signal, not a grant; terminal resize redraws without changing selection. Render target/reason/risk and every legal choice with scroll support, terminal-safe text, and monochrome labels. Set focus to Allow once where legal, never a persistent option; document that focus is not acceptance.

Reuse `davinci_interactive::permission_ask`, `permission_choice`, `approval_key`, `run_turn`, `model::Overlay::Ask`, and `views/ask.rs` as the existing seams. A new view helper is subordinate to that overlay, not a second modal or competing input router. Preserve the existing shared SheetChrome and voice-cancellation behavior.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: 40x12 and 120x40 screens; long URLs; Unicode combining marks; bracketed paste; voice transcript arriving while open; terminal resize; existing draft/caret unchanged after deny and allow.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/views/approval_modal.rs" "crates/davinci-tui/src/davinci/model.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs" "crates/davinci-tui/src/davinci/views/ask.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: permission-approval-modal task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Durable legal scopes and session lifecycle

**Covers:** F01-2, F01-5

**Test placement:** `crates/davinci-agent/src/approval.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/approval.rs` — grant lifecycle and scope checks (introduced by F01 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/permissions.rs` — trusted project-rule update
- **Modify:** `crates/davinci-coding-agent/src/settings.rs` — atomic persistence through existing settings APIs

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f01_grant_binding() {
    assert!(grant_applies("s1", "s1", "web_fetch:https://a.example:443", "web_fetch:https://a.example:443", false));
    assert!(!grant_applies("s1", "s2", "x", "x", false));
    assert!(!grant_applies("s1", "s1", "x", "x", true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f01_grant_binding -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn grant_applies(issued_session: &str, current_session: &str,
    canonical_subject: &str, requested_subject: &str, explicit_deny: bool) -> bool {
    !explicit_deny && issued_session == current_session && canonical_subject == requested_subject
}
```

Session grants are an in-memory overlay evaluated after hard denies and before Ask; never serialize them into resumed consent. Project updates use the existing trusted settings location and preserve unrelated fields; verify the policy revision before committing. When a requested persistent rule cannot be represented narrowly in the current rule grammar, do not offer that scope. Display exactly what rule will be saved. Persist before acknowledging success, and invalidate challenge caches when permissions/settings change.

Current native/RPC code downgrades a failed project-rule save to a session grant. This feature deliberately replaces silent fallback with an explicit save-failed decision state and asks the user to select another legal scope. Display the actual generated rule from `session_rule_for`; some existing session rules are wider than an exact call, and must not be described as exact-call grants.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: disk full; concurrent settings change; legacy .pi configuration; session resume does not restore Once/session grants; explicit deny introduced after grant; rule parser refuses a broad wildcard.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/approval.rs" "crates/davinci-coding-agent/src/permissions.rs" "crates/davinci-coding-agent/src/settings.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: permission-approval-modal task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Cross-host parity and cancellation

**Covers:** F01-1, F01-3, F01-4, F01-5

**Test placement:** `crates/davinci-coding-agent/src/rpc.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-coding-agent/src/rpc.rs` — typed extension/UI request adapter
- **Modify:** `crates/davinci-coding-agent/src/main.rs` — print and legacy host behavior; RPC/legacy approval adapters and explicit EOF termination
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — single pending-challenge integration

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f01_noninteractive_fail_closed() {
    assert_eq!(approval_host_result(false, false, false), "approval_required");
    assert_eq!(approval_host_result(true, false, true), "denied");
    assert_eq!(approval_host_result(true, true, false), "resolved");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f01_noninteractive_fail_closed -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn approval_host_result(interactive: bool, valid_reply: bool, disconnected: bool) -> &'static str {
    if disconnected { "denied" }
    else if valid_reply { "resolved" }
    else if interactive { "pending" }
    else { "approval_required" }
}
```

Adapt the existing RPC UI request/response channel without emitting unsolicited non-JSON text on stdout. Negotiate richer capabilities if adding wire fields; older clients receive safe equivalent select/confirm choices and the same engine validation. Print mode returns a structured approval_required result naming the unresolved action and legal configuration path, rather than hanging or auto-allowing. Legacy UI renders the same choices. Test request cancellation, stream completion, and pending challenge cleanup before presenting the next request. Update help and permission docs in this task.

The RPC approval functions and waiter live in `main.rs` (`rpc_approval_call`, `rpc_approval_decision`, `rpc_emit_and_wait_ui`, `parse_rpc_ui_response`), with a thread-local waiter in `js_host.rs`. Reject an unoffered Always response in an untrusted project instead of downgrading it to Session. Distinguish channel timeout from disconnect/EOF so a closed client cannot leave a no-timeout approval loop waiting forever. The EOF risk was identified statically, not reproduced in this documentation session.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: RPC malformed/late response; legacy escape; print JSON output remains parseable; queued requests source ordered; disconnect denies; no process or network fixture called before resolution.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/rpc.rs" "crates/davinci-coding-agent/src/main.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: permission-approval-modal task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Required behavior |
|---|---|
| Reply arrives after policy/contract change | Reject stale challenge; do not silently upgrade scope. |
| Invalid/high-risk persistent choice | Reject at engine regardless of UI contents. |
| Pending request storage/transport fails | Fail closed; explain approval was not recorded. |
| Sensitive arguments/ANSI sequences | Display sanitized bounded context; redact secrets without weakening digest binding. |
| Concurrent prompts or user interrupt | Ordered bounded queue; cancellation resolves each once and releases waiters. |

## Persistence, migration, and rollback

Additive host DTOs and ephemeral state; keep old permission labels/aliases and existing legal rule syntax. On downgrade, ignore only optional display fields, not security-critical unknown scopes. Rollback disables the new renderer while retaining engine validation and existing approval behavior; never roll back by auto-approving pending requests.

## End-to-end acceptance and release gates

Use a canned web-fetch tool request under Manual mode, display its exact target, deny with instructions, and prove no executor invocation occurred. Repeat with a legal session grant and a different port/domain that must ask again. While the modal is open, send every mode key plus paste and verify the permission mode, draft bytes, caret, and submit count remain unchanged. The same policy matrix must pass through native TUI, legacy, RPC, and print fixtures.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.
