# Browser and Terminal Interaction Testing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Generate reproducible user-surface evidence from real managed browser and PTY sessions, including keyboard and resize behavior.

**Architecture:** Add a native interaction-test controller with injectable terminal/browser backends, owned process leases, bounded artifact storage, and explicit assertions. Use current inline renderer tests for fast coverage, then separate real PTY/browser integration fixtures for actual surface behavior.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Browser and Terminal Interaction Testing — supplied source specification](2026-09-07-davinci-feature-specs/11-browser-terminal-interaction-testing.md)

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

`davinci-tui/src/davinci/fixtures.rs`, app.rs/model.rs/views tests, and runtime.rs already provide input/render fixtures and ConPTY paste handling. No managed native browser/PTY test runner was found in active Rust. The xterm dependency under vendor is not an active dependency and must not be edited or borrowed by mutating vendor. Existing graph WorkerRunner/VerifyExec injection patterns and job/process ownership are useful seams. A screenshot or raw ANSI grep alone is not proof that the UI behaved correctly.

## Scope, alternatives, and chosen approach

Use a PTY abstraction rather than piping stdin to a non-terminal child, because Crossterm behavior depends on terminal state. Prefer a reviewed exact pin of portable-pty 0.9.0 and vt100 0.15.2 as candidate backends, subject to a Rust1.83/Windows/Unix compatibility and dependency audit before addition; these are proposed pins, not confirmed build results in this repo. Keep dependencies optional behind an interaction-testing feature and retain offline fake backends. Browser V1 uses a configured, exact-version Playwright runtime through a narrow JSONL bridge, with no automatic download, login, or global profile use. It is a testing adapter, not a replacement UI framework.

**Dependencies and implementation order:** Requires 06 evidence,07 owned process teardown,09 budgets, and 05 process/network contracts. Fake renderer/runner development can begin early; broad UI scenarios integrate 01–03/05/07/08/13. Real backend compatibility is an explicit release gate.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F11-1 | Launch Davinci in a controlled terminal, send keys and resize, and assert rendered state. |
| F11-2 | Cover five-mode Shift+Tab cycling with an unfinished draft preserved and ordinary Tab behavior unchanged. |
| F11-3 | Record screen frames, event logs, process exits, and exact executable/source identity as evidence. |
| F11-4 | For web tasks, assert browser state and capture screenshots/DOM, console errors, failed requests, and useful traces. |
| F11-5 | Run reproducibly with isolated resources, no live services/downloads, bounded artifacts, and no false claim of physical-keyboard verification. |

## Data and behavioral contracts

`InteractionScenario { id, target: Terminal | Browser, executable_or_url, source_manifest, steps, time_budget, allowed_origins, artifact_policy }`; steps are a bounded enum, not arbitrary scripts: Launch, TypeText, Key, Resize, WaitForCondition, AssertScreen/AssertDom, Capture, Shutdown. `InteractionReceipt { scenario_id, backend_identity, source_manifest, assertions, frames, event_log, console_errors, network_failures, trace_refs, exit_outcome }`.

Terminal frames capture a virtual screen grid, dimensions, cursor, and relevant attributes after parsing the PTY byte stream; raw bytes remain a bounded artifact for debugging. The controller waits on observable conditions with deadlines, not arbitrary sleeps. Browser fixtures use loopback test servers with all other traffic denied and isolated ephemeral profiles. Console errors and failed requests are recorded even when assertions pass. Trace export is opt-in/redacted because traces may contain credentials and page data. Every receipt states whether it came from a renderer unit fixture, actual PTY, actual browser, or manual physical interaction.

## User experience and mode behavior

A test-run sheet displays steps, actual assertions, elapsed/remaining budget, source/executable identity, and artifacts. It states whether the run used renderer fixtures, real PTY, real browser, or a manual physical check. A capture-only run is not reported as a passed functional test.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Backend interfaces and dependency admission

**Covers:** F11-1, F11-4, F11-5

**Test placement:** `crates/davinci-coding-agent/src/interaction_testing/mod.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/mod.rs` — scenario/backend/receipt types
- **Modify:** `Cargo.toml` — exact optional backend dependency pins after audit
- **Modify:** `crates/davinci-coding-agent/Cargo.toml` — interaction-testing feature
- **Modify:** `Cargo.lock` — review resolved MSRV-compatible dependency graph

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f11_backend_capabilities() {
    assert_eq!(interaction_backend(false, false), "unavailable");
    assert_eq!(interaction_backend(true, false), "fixture_only");
    assert_eq!(interaction_backend(true, true), "real_backend");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f11_backend_capabilities -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn interaction_backend(fixture_supported: bool, real_supported: bool) -> &'static str {
    if real_supported { "real_backend" } else if fixture_supported { "fixture_only" } else { "unavailable" }
}
```

Add trait-based fake backends first. Before adding proposed portable-pty/vt100 pins, inspect published manifests/license/advisories and resolve all transitive versions against Rust1.83 on both target families; lock the result exactly. An unavailable cache is an explicit dependency gate requiring authorized preparation, never a reason for tests to fetch online. Do not run cargo add with floating versions or upgrade the toolchain to make a backend compile. Browser runtime configuration records exact package/browser version and path; absent runtime returns unavailable without npx/install. Tests compile the core fake backend even when optional real backends are disabled.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: feature disabled; missing cache/runtime; wrong binary version; offline build both targets; dependency changes outside planned packages rejected; fake receipt cannot claim real PTY.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/interaction_testing/mod.rs" "Cargo.toml" "crates/davinci-coding-agent/Cargo.toml" "Cargo.lock"
rtk git diff --cached --stat
rtk git commit -m "feat: browser-terminal-interaction-testing task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Owned PTY lifecycle and terminal screen state

**Covers:** F11-1, F11-3, F11-5

**Test placement:** `crates/davinci-coding-agent/src/interaction_testing/terminal.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/terminal.rs` — PTY child, reader/writer, resize, drain
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/screen.rs` — terminal parser and bounded frames
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/control.rs` — owned process-tree leases (introduced by F07 Task 1; do not recreate it)

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f11_resize_bounds() {
    assert!(terminal_size_allowed(80, 24));
    assert!(terminal_size_allowed(40, 12));
    assert!(!terminal_size_allowed(0, 24));
    assert!(!terminal_size_allowed(1000, 1000));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f11_resize_bounds -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn terminal_size_allowed(cols: u16, rows: u16) -> bool {
    (1..=300).contains(&cols) && (1..=120).contains(&rows)
}
```

Create and own the pseudoterminal before launching Davinci with an isolated config/session directory and deterministic mock-provider fixtures. Run independent I/O draining so closing/resizing cannot deadlock on output. On Windows honor ConPTY's UTF-8 channel and handle lifecycle; on Unix use the PTY backend and owned process group. Propagate resize to both OS PTY and screen parser. Parse cursor movement, erase, alternate screen, wrapping, and attributes; never treat raw escape bytes as the current screen. Bound scrollback and frame count, then shut down cooperatively and escalate only the owned process tree. Unsupported critical VT behavior makes the test unproven, not silently passed.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: UTF-8 split across reads; resize during render; alternate-screen switch; output flood; child exits early; hung child; no output; ConPTY shutdown drain; unrelated user terminal remains alive.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/interaction_testing/terminal.rs" "crates/davinci-coding-agent/src/interaction_testing/screen.rs" "crates/davinci-agent/src/runtime/control.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: browser-terminal-interaction-testing task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Deterministic scenario runner and assertions

**Covers:** F11-1, F11-2, F11-3, F11-5

**Test placement:** `crates/davinci-coding-agent/src/interaction_testing/runner.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/runner.rs` — condition-based steps/deadlines/assertion receipts
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/interaction_testing/terminal.rs` — key encodings and draft probes (introduced by F11 Task 2; do not recreate it)

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f11_key_encoding() {
    assert_eq!(terminal_key("shift_tab"), Some(&b"\x1b[Z"[..]));
    assert_eq!(terminal_key("tab"), Some(&b"\t"[..]));
    assert_eq!(terminal_key("escape"), Some(&b"\x1b"[..]));
    assert_eq!(terminal_key("unknown"), None);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f11_key_encoding -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn terminal_key(key: &str) -> Option<&'static [u8]> {
    match key {
        "shift_tab" => Some(b"\x1b[Z"), "tab" => Some(b"\t"),
        "escape" => Some(b"\x1b"), "enter" => Some(b"\r"), _ => None
    }
}
```

Define named key events and explicit bracketed-paste handling; distinguish a synthetic byte sequence from a physical keyboard scan code. Wait for a known visible condition and stable bounded frame sequence before asserting. Each assertion records expected/actual state, screen digest, timestamp, and source/executable identity. No arbitrary scenario shell commands, JavaScript evaluation, or infinite wait loops. A step timeout fails that assertion, cancels remaining effects safely, and captures a final diagnostic frame. Fixture clocks control unit tests; real-backend deadlines use monotonic time and the root remaining task deadline.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: screen label never appears; repeated key press; incomplete multibyte input; no assertions scenario rejected; timeout preserves logs; terminal output alone cannot fake host assertion metadata.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/interaction_testing/runner.rs" "crates/davinci-coding-agent/src/interaction_testing/terminal.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: browser-terminal-interaction-testing task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Five-mode cycle and modal regression suite

**Covers:** F11-1, F11-2, F11-3

**Test placement:** `crates/davinci-tui/src/davinci/app.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-tui/src/davinci/app.rs` — inline cycle/modal event tests
- **Modify:** `crates/davinci-tui/src/davinci/fixtures.rs` — deterministic permission/task/context graph views
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/scenarios.rs` — real terminal scenario definitions

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f11_five_mode_cycle() {
    let mut mode = 0;
    let mut labels = Vec::new();
    for _ in 0..5 { mode = next_mode_index(mode); labels.push(mode); }
    assert_eq!(labels, vec![1, 2, 3, 4, 0]);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-tui --offline f11_five_mode_cycle -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn next_mode_index(index: usize) -> usize { (index + 1) % 5 }
```

The integration suite must use actual PermissionMode::next and UI events rather than replacing mode logic with this arithmetic oracle. Start Manual, type an unfinished Unicode draft, then Shift+Tab through Accept Edits→Plan Mode→Auto Mode→Always Approve→Manual. Assert draft text and caret unchanged after every transition and assert ordinary Tab's existing completion behavior remains intact. Repeat with approval/decision/scope modals open and require mode cycling blocked. Cover task board selection, steering receipt, context mandatory exclusion, graph pause/retry feedback, Ctrl+C, paste, terminal resize, and transcript scrolling. Renderer fixtures provide broad fast cases; actual PTY cases prove the executable event path.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: five labels exact; partial draft with slash/text/newline; modal intercept; voice not auto-submitted; normal Tab; narrow40x12; Linux/macOS/Windows key adapters; active streaming mode restrictions.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/app.rs" "crates/davinci-tui/src/davinci/fixtures.rs" "crates/davinci-coding-agent/src/interaction_testing/scenarios.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: browser-terminal-interaction-testing task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Managed browser adapter and offline fixture network

**Covers:** F11-4, F11-5

**Test placement:** `crates/davinci-coding-agent/src/interaction_testing/browser.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/browser.rs` — trusted bridge process and JSONL protocol
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/browser_bridge.js` — bounded Playwright adapter
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/browser_fixtures.rs` — loopback fixtures and assertions

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f11_network_fixture_only() {
    assert!(browser_origin_allowed("http://127.0.0.1:43123", "http://127.0.0.1:43123"));
    assert!(!browser_origin_allowed("https://example.com", "http://127.0.0.1:43123"));
    assert!(!browser_origin_allowed("http://127.0.0.1:43124", "http://127.0.0.1:43123"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f11_network_fixture_only -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn browser_origin_allowed(request_origin: &str, fixture_origin: &str) -> bool {
    request_origin == fixture_origin
}
```

Compare parsed canonical origins in production; raw URL prefix matching is unsafe. Launch a fresh ephemeral browser profile through the approved exact-version Playwright runtime. Restrict targets/redirects/WebSockets/service-worker traffic to the fixture policy, and enforce network boundaries below page script where required; route interception alone is not a universal OS sandbox. Steps perform locator-based actions/assertions, not arbitrary eval from model content. Capture screenshot or DOM snapshot for relevant states, console errors, failed requests, and optional trace. Explicitly store assertion results because the browser-context tracing API does not itself guarantee test assertion capture. No user cookies, production login, cloud test provider, or package downloads.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: forbidden redirect; external image request; service worker; failed fetch; console error; DOM assertion fails despite screenshot; browser crash; malformed bridge message; missing runtime; same host different port.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/interaction_testing/browser.rs" "crates/davinci-coding-agent/src/interaction_testing/browser_bridge.js" "crates/davinci-coding-agent/src/interaction_testing/browser_fixtures.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: browser-terminal-interaction-testing task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 6: Evidence integration, artifact limits, and release matrix

**Covers:** F11-3, F11-4, F11-5

**Test placement:** `crates/davinci-coding-agent/src/interaction_testing/artifacts.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/interaction_testing/artifacts.rs` — artifact hashing/redaction/retention
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/evidence.rs` — interaction evidence kind (introduced by F06 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — test-run report and gaps

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f11_evidence_environment() {
    assert!(!proves_physical_keyboard("pty"));
    assert!(!proves_physical_keyboard("renderer_fixture"));
    assert!(!proves_physical_keyboard("browser"));
    assert!(proves_physical_keyboard("manual_physical_keyboard"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f11_evidence_environment -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn proves_physical_keyboard(environment: &str) -> bool {
    environment == "manual_physical_keyboard"
}
```

Only a separately authenticated manual check can produce that manual environment tag. Bind all automated receipts to the exact executed binary hash, scenario hash, source manifest, backend/runtime versions, and executed assertions. Proposed cap is 50 MiB per test run and 500 MiB per task; exceeding it keeps required minimal failure metadata and labels omitted artifacts, never drops an assertion. Redaction scans known secrets and sensitive DOM/network fields but cannot promise arbitrary traces are safe to publish; default traces remain local. Run fast inline fake tests in the normal offline suite and explicitly selected real-backend tests where provisioned. Missing real backend is a named coverage gap, not a skipped pass. Update interaction-testing help and capability reporting.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: wrong installed binary; zero assertions; trace cap; missing screenshot; secret in console/network headers; cancelled run; fixture pass vs real PTY pass; manual claim cannot be generated by browser adapter.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/interaction_testing/artifacts.rs" "crates/davinci-agent/src/runtime/evidence.rs" "crates/davinci-coding-agent/src/davinci_surfaces.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: browser-terminal-interaction-testing task 6"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Backend/runtime missing | Capability unavailable, coverage gap, no automatic installation. |
| Child hangs or output floods | Deadline/caps and owned-tree cleanup; retain failure artifacts. |
| Snapshot looks right but assertion fails | Test failed; image is supporting evidence only. |
| Network/credential boundary | Refuse/abort and redact local artifacts before export. |
| Physical-keyboard behavior untested | Explicit remaining gap, even after a real PTY pass. |

## Persistence, migration, and rollback

Optional test backends and scenario schemas are additive. Do not migrate user sessions into test profiles or download dependencies during startup. Old evidence without environment identity remains unproven for live UI. Rollback disables real test launch and preserves existing evidence/artifacts until retention permits removal.

## End-to-end acceptance and release gates

Execute the unfinished-draft five-mode scenario in a real managed PTY on Windows and at least one supported Unix CI target, preserving a frame/event/exit bundle. Execute an offline browser fixture with passing and failing DOM assertions plus console/network failures. Both must create current-source evidence with precise environment labels. Renderer-only tests, skipped browser setup, and physical keyboard checks remain distinguishable in the final completion matrix.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.

Primary references: Microsoft ConPTY creation/lifecycle and Job Objects documentation; Playwright tracing, Trace Viewer, and network documentation; portable-pty 0.9.0 published manifest/source and vt100 0.15.2 manifest. Exact URLs and dependency-gate limitations are in the shared reference section.
