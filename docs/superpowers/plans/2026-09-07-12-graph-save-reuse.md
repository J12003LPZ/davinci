# /graph Save and Reuse Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Save and run validated native engineering graph definitions in .davinci/graphs without introducing a dynamic workflow language.

**Architecture:** Add a strict saved-definition DTO/parser/store and a compiler from safe declarative configuration to the existing native graph controller. Freeze the imported definition and effective policy into each run; keep runtime state and ad-hoc prompts out of saved definitions.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [/graph Save and Reuse — supplied source specification](2026-09-07-davinci-feature-specs/12-graph-save-reuse.md)

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

`graph/topology.rs` defines GraphDefinition/NodeDefinition/EdgeDefinition and validates DAG/readiness/writer ordering. `graph/store.rs` writes run-local graph.json under `.davinci/graph/runs` (legacy `.pi/graph/runs` fallback), which is different from the requested reusable `.davinci/graphs/*.yaml` folder. There is no YAML parser in the inspected Cargo manifests/lock; templates.rs frontmatter parsing is flat and unsuitable. `graph/render.rs` currently parses only mode/dry-run flags; `GraphController::command` treats any nonempty /graph arguments as a goal. Most importantly, controller.rs still drives native hard-coded phases: assigning an imported run.definition alone would not execute the saved topology.

## Scope, alternatives, and chosen approach

Use a restricted YAML1.2 document interpreted only as typed data, not scripts, template expressions, environment interpolation, or arbitrary worker prompts. Proposed parser pin is yaml-rust2 `=0.10.4` with encoding features disabled for UTF-8-only input, subject to MSRV/transitive dependency/advisory checks before implementation; it is not claimed already installed or the latest version. Parse via events so duplicate keys and prohibited aliases/tags can be rejected before map construction. A JSON-compatible YAML-only shortcut would avoid a dependency but would surprise users expecting conventional YAML, so it is not the chosen primary design.

Retain the native mode semantics: all delivered changes require real verification and applicable security gates; Standard/Complex require their current review guarantees, while Simple's existing no-review fast path is preserved unless explicitly strengthened by an approved policy. The source mockup's disjoint scopes do not authorize parallel mutation writers.

**Dependencies and implementation order:** Uses 05 contracts,06 source/evidence,09 resource leases. Schema/parser and compile bindings can be developed before mutating runtime integration. Feature 13 depends on stable validated node identities;14 reuses this parser/export model.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F12-1 | Implement /graph save <name> and /graph run <name> with .davinci/graphs/<name>.yaml storage. |
| F12-2 | Save topology, roles, artifact contracts, budgets, verification policy, and safe parameters only. |
| F12-3 | Reject unsafe/malformed definitions and preserve DAG, one-writer, mode-specific verification/security/review gates. |
| F12-4 | Execute the saved definition through real native bindings without silently reclassifying/replacing it. |
| F12-5 | Do not save hidden reasoning, transcripts, ad-hoc prompts, credentials, or dynamic workflow scripts. |
| F12-6 | Preserve bare /graph and legacy goal/flag behavior with an explicit escape for reserved subcommand words. |

## Data and behavioral contracts

Proposed `SavedGraphDefinitionV1 { schema_version, name, description, graph, bindings, budgets, verification_policy, artifact_contract_versions, parameters }`. `graph` reuses validated native topology semantics. `bindings` maps every native node/template to a finite supported stage and its typed artifact inputs, with no arbitrary command body. Verification uses trusted named command profiles, not executable strings imported from an untrusted file. Parameters are bounded typed values: goal text, relative path sets, enums, and lower resource ceilings; they cannot contain tool grants, shell interpolation, hidden prompt text, credentials, or scope increases.

Limits: UTF-8 file ≤256 KiB; depth≤16; ≤64 nodes, ≤256 edges, ≤32 parameters; names/IDs≤64 ASCII alphanumeric, underscore or hyphen; reject Windows reserved device names, path traversal, case-colliding names, duplicates, unknown keys/versions, aliases/anchors/merge keys/custom tags/multiple documents/nonfinite numbers. Canonical SHA-256 digest covers normalized definition and artifact contract versions. Running a saved graph creates a new run with a frozen copy/digest, fresh trust/permission/contract validation, and a root budget lease. Reuse of earlier evidence is a separate fingerprint-gated operation, never implicit in save/run.

## User experience and mode behavior

`/graph save security-audit` reports the exact project path and definition digest. `/graph run security-audit` previews effective scope/policy/budget and starts only after required authorization. Invalid YAML is never silently replaced with a default graph.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Strict YAML parser and safe identifiers

**Covers:** F12-1, F12-3, F12-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs` — versioned input parser and bounds
- **Modify:** `Cargo.toml` — exact parser pin after dependency gate
- **Modify:** `crates/davinci-coding-agent/Cargo.toml` — YAML parser dependency
- **Modify:** `Cargo.lock` — audited resolved dependencies

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f12_safe_graph_name() {
    assert!(safe_graph_name("security-audit"));
    for bad in ["../x", "CON", "aux", "a/b", "name.", "", "x:y"] { assert!(!safe_graph_name(bad)); }
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f12_safe_graph_name -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn safe_graph_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.is_empty() && name.len() <= 64
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        && !["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9",
             "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"].contains(&lower.as_str())
}
```

Perform dependency admission before touching runtime behavior: exact direct pin, audited lockfile, Rust1.83 compile on supported targets, and no network in tests. Use a streaming event-level validator to enforce document/depth/node limits and reject duplicate mapping keys, anchors, aliases, tags, merges, and complex map keys before conversion to serde DTOs with deny_unknown_fields. Do not use flat frontmatter parsing or assume a generic loader detects duplicates. Return bounded errors with source line/column and field path. Loading invalid saved files fails closed rather than defaulting to a different graph.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: ordinary block/flow YAML roundtrip; duplicate key; nested alias bomb; custom tag; multidocument; NaN/infinity; UTF-8 error; depth limit; 65th node; case collision; symlinked graphs folder.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs" "Cargo.toml" "crates/davinci-coding-agent/Cargo.toml" "Cargo.lock"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-save-reuse task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Definition coherence and mandatory native gates

**Covers:** F12-2, F12-3, F12-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs` — saved DTO and strict validator (introduced by F12 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/topology.rs` — identity/role/artifact/conditional-path validation
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/validate.rs` — artifact contract version mapping

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f12_role_artifact_coherence() {
    assert!(role_contract_valid("writer", "patch-report", true));
    assert!(role_contract_valid("reviewer", "review", false));
    assert!(!role_contract_valid("reviewer", "patch-report", true));
    assert!(!role_contract_valid("researcher", "evidence", true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f12_role_artifact_coherence -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn role_contract_valid(role: &str, artifact: &str, mutates: bool) -> bool {
    match role {
        "writer" => artifact == "patch-report" && mutates,
        "reviewer" => artifact == "review" && !mutates,
        "planner" => artifact == "plan" && !mutates,
        "classifier" => artifact == "classification" && !mutates,
        "researcher" | "test-analyzer" | "historian" => artifact == "evidence" && !mutates,
        _ => false,
    }
}
```

Use actual Role/ArtifactKind enums in production. Validate unique safe node IDs before building maps, every edge endpoint, root/reachability, bounded DAG, branch-condition semantics, and serial mutation ordering. Existing reviewer reachability alone is not proof that all successful delivery paths pass the correct review gate. Compile mandatory host verification/security and mode-specific review into the native stage plan; reject definitions whose conditional exits can bypass them. A reviewer cannot approve its own mutation and review coverage must include every current owned chunk. Validate parameters against a typed allowlist, with canonical relative scopes and ceilings that only narrow authorized limits.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: duplicate IDs; unknown artifact kind/version; reviewer writer privilege; failure branch bypass; unreachable required node; condition conjunction impossible; concurrent writer frontier; Simple retains verify/security and explicit mode semantics.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs" "crates/davinci-coding-agent/src/native_extensions/graph/topology.rs" "crates/davinci-coding-agent/src/native_extensions/graph/validate.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-save-reuse task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Compile saved topology into actual native execution

**Covers:** F12-2, F12-3, F12-4

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/bindings.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/bindings.rs` — finite native stage/input bindings
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — execute compiled saved plan instead of reclassification
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/types.rs` — frozen definition/binding digest

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f12_frozen_definition() {
    assert_eq!(definition_for_run(Some("saved-hash"), "generated-hash"), "saved-hash");
    assert_eq!(definition_for_run(None, "generated-hash"), "generated-hash");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f12_frozen_definition -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn definition_for_run<'a>(saved: Option<&'a str>, generated: &'a str) -> &'a str {
    saved.unwrap_or(generated)
}
```

Add an explicit execution origin GeneratedGoal or SavedDefinition; only GeneratedGoal may classify/build a new topology. Compile supported stage bindings into existing WorkerSpec/briefing/verification/review calls and preserve zero coordinator model calls. Every dispatched node and bounded revision/milestone expansion must have a validated binding in the active compiled plan; the current bypass for unknown task IDs cannot be used by imports. Bind artifact inputs by validated node/contract refs rather than concatenating arbitrary text. A saved execution failure must report unsupported binding instead of silently falling back to the default goal pipeline. Test that changing a saved node changes the actual worker sequence.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: injected WorkerRunner records exact order/roles; import not reclassified; bounded dynamic attempt IDs registered; undefined binding rejected; topology graph and actual frontier agree; verification remains real; no hidden prompt import.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/bindings.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs" "crates/davinci-coding-agent/src/native_extensions/graph/types.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-save-reuse task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Atomic save/load and immutable run snapshots

**Covers:** F12-1, F12-2, F12-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs` — safe project definition store (introduced by F12 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/store.rs` — frozen run-local definition/reference persistence
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` — trusted save/run orchestration

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f12_overwrite_requires_explicit() {
    assert!(save_destination_allowed(false, false, true));
    assert!(!save_destination_allowed(true, false, true));
    assert!(save_destination_allowed(true, true, true));
    assert!(!save_destination_allowed(false, true, false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f12_overwrite_requires_explicit -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn save_destination_allowed(exists: bool, explicit_overwrite: bool, contained: bool) -> bool {
    contained && (!exists || explicit_overwrite)
}
```

Store only validated declarative fields from a completed successful real run for /graph save; explicit export in14 may review an incomplete definition without claiming success. Resolve `.davinci/graphs` securely and prevent symlink/reparse escapes. Create temp files exclusively in the same directory, write/sync, and publish atomically; when replacing, verify the expected prior digest and user confirmation. Validate on every load, not only at save time. Freeze a normalized copy in the new run directory before worker launch. Preserve the source run and never move its artifacts into the reusable definition folder.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: save to existing name; competing saves; disk full; malicious directory link; changed file after prompt; malformed load; source run remains byte-identical; `.pi/graph/runs` resume unaffected.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/definitions.rs" "crates/davinci-coding-agent/src/native_extensions/graph/store.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mod.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-save-reuse task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Command routing and safe parameter binding

**Covers:** F12-1, F12-6

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/render.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/render.rs` — typed subcommand parser
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` — dispatch save/run before free-form goal
- **Modify:** `crates/davinci-coding-agent/src/slash.rs` — one graph family help/autocomplete
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/mod.rs` — internal lifecycle visibility regression

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f12_reserved_word_escape() {
    assert_eq!(graph_command_kind(""), "current");
    assert_eq!(graph_command_kind("save security-audit"), "save");
    assert_eq!(graph_command_kind("run security-audit"), "run");
    assert_eq!(graph_command_kind("-- save the broken file"), "goal");
    assert_eq!(graph_command_kind("repair OAuth"), "goal");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f12_reserved_word_escape -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn graph_command_kind(args: &str) -> &'static str {
    let args = args.trim();
    if args.is_empty() { return "current"; }
    if args.starts_with("-- ") { return "goal"; }
    match args.split_whitespace().next() {
        Some("save") => "save", Some("run") => "run", _ => "goal",
    }
}
```

Extend the parser into a typed GraphCommand enum shared by12–14, preserving multiline goals and existing --simple/--complex/--dry-run behavior. Reserved verbs require their exact documented arguments; malformed save/run is an error, not a surprise new model goal. `/graph -- <goal>` preserves literal goals beginning with a reserved word. Parse safe parameter values without shell expansion or environment substitution. Keep graph-status/resume/view/abort internal lifecycle actions compatible but unadvertised. Bare /graph still opens/resumes the appropriate existing run, with13's explicit paused-state rule.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: bare graph; multiline goal; existing dry-run flag; reserved-name goal escaped; missing name; extra unsafe parameter; paths with spaces as typed values; help lists no internal lifecycle commands.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/render.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mod.rs" "crates/davinci-coding-agent/src/slash.rs" "crates/davinci-coding-agent/src/native_extensions/mod.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-save-reuse task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 6: Trust, replay provenance, and full saved-run acceptance

**Covers:** F12-1, F12-2, F12-3, F12-4, F12-5, F12-6

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/replay.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/replay.rs` — definition/policy/content identity
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — new-run authorization and root-budget lease
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/store.rs` — legacy state compatibility

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f12_replay_requires_all_bindings() {
    assert!(saved_replay_allowed(true, true, true, false));
    assert!(!saved_replay_allowed(false, true, true, false));
    assert!(!saved_replay_allowed(true, true, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f12_replay_requires_all_bindings -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn saved_replay_allowed(definition_matches: bool, source_matches: bool,
    policy_matches: bool, simulated: bool) -> bool {
    definition_matches && source_matches && policy_matches && !simulated
}
```

Saving permission-related configuration does not save user consent. Revalidate project trust, current mode, allowed tools, contract, effective model/config, scopes, resource lease, and named verification profile for each run. Strengthen source identity with06's content manifest rather than HEAD+porcelain status. Legacy/missing replay fingerprints cause reexecution, never approval by default. Keep simulation provenance separate from real evidence. Add a full roundtrip test that saves a successful fixture definition, reloads it, and executes the frozen node bindings with exact nonzero resource receipts and real fake-executor verification semantics. Update graph save/run documentation with a complete accepted YAML fixture generated by the serializer.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: edited saved definition; dirty bytes same status; changed policy/model; stale artifact contract; untrusted import; previous simulated run; old state-v1 fixture; resumed counters not double charged.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/replay.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs" "crates/davinci-coding-agent/src/native_extensions/graph/store.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-save-reuse task 6"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Parser/dependency not admitted | Do not ship conventional YAML loading; explicit capability gate. |
| Unknown keys/IDs/bindings or gate bypass | Reject whole definition before launch, with bounded diagnostics. |
| Untrusted saved config | May inspect safe data; cannot authorize commands or tools. |
| Save/load race | Digest conflict or atomic failure, no partial published YAML. |
| Missing replay provenance | Reexecute under current authorization, not reuse as verified. |

## Persistence, migration, and rollback

Keep existing run-local graph.json and state-v1 readers. Saved definition schema is separate and strict. No automatic conversion of arbitrary old transcript/prompt content into definitions. Preserve 0-as-unlimited legacy graph budget semantics when importing old settings, but a new task root may apply a tighter explicit ceiling. Rollback keeps saved files for review and refuses execution of unsupported versions.

## End-to-end acceptance and release gates

Save a successful fixture graph as security-audit, inspect the emitted YAML, and run it again. Record the worker sequence proving the loaded definition was executed rather than replaced by classification. Reject hostile YAML and graph gate-bypass fixtures without starting a worker. Change dirty file bytes without changing Git status and require replay refusal. Validate bare/goal/flag/internal command compatibility and no saved hidden prompts or reasoning.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.

YAML1.2.2 specification and yaml-rust2 0.10.4 published package documentation were consulted. Parser pin and transitive Rust1.83 compatibility remain an implementation admission test, not a completed build claim.
