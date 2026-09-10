# Davinci AI Harness — Feature Expansion Roadmap

**Prepared:** 2026-09-07 · **Baseline:** `main@9c42820` · **Status:** proposed implementation program, not executed.

This bundle translates all 14 supplied feature specifications into repository-specific implementation plans. Start here, read [shared contracts](2026-09-07-00-shared-runtime-contracts.md), then execute a selected feature plan only after its prerequisite interfaces are available. The source specifications remain proposed: the plans record engineering choices, not a claim of user approval or an already working feature.

## Bundle contents

The bundle contains **14 implementation plans**, **4 companion documents**, and **14 Markdown source-spec transcriptions**. The feature plans contain **74 implementation tasks**, each with a failing contract test, implementation example, concrete file responsibilities, additional required integration cases, verification command, and bounded future commit scope. The [requirements matrix](2026-09-07-00-requirements-and-validation.md) tracks **75 requirements**. Source documents are preserved under `2026-09-07-davinci-feature-specs/`.

| Feature | Implementation plan | Tasks | Intended outcome |
|---|---|---:|---|
| 01 | [Permission Approval Modal](2026-09-07-01-permission-approval-modal.md) | 5 | Provide one policy-owned approval interaction across all hosts, with legal scoped grants and no input or consent leakage. |
| 02 | [Structured Clarification / Decision Interview](2026-09-07-02-structured-clarification.md) | 5 | Capture explicit user decisions for material ambiguity without mistaking a recommendation, timeout, or model response for consent. |
| 03 | [Execution Tasks / Live Task Board](2026-09-07-03-execution-tasks-live-board.md) | 5 | Make work ownership, dependencies, progress, and evidence durable and inspectable without conflating execution tasks with the Living Plan. |
| 04 | [Task-level Rewind with Dirty-file Preservation](2026-09-07-04-task-level-rewind.md) | 5 | Restore only a task's reversible changes and selected state while protecting preexisting and subsequent user edits. |
| 05 | [Task-scoped Execution Contracts](2026-09-07-05-task-scoped-contracts.md) | 5 | Enforce accepted task scope across all execution paths, independently of permission mode and worker instructions. |
| 06 | [Evidence-backed Completion](2026-09-07-06-evidence-backed-completion.md) | 5 | Produce honest per-dimension completion status backed by immutable execution evidence for the exact source that was verified. |
| 07 | [Live Task and Agent Control Panel](2026-09-07-07-live-task-agent-controls.md) | 5 | Expose every active worker's work, ownership, waits, usage, and intervention state, with steering and real owned-process cancellation. |
| 08 | [Context and Memory Inspector](2026-09-07-08-context-memory-inspector.md) | 5 | Let the user inspect and correct the exact prepared context and memory provenance without disabling mandatory policy or silently changing in-flight requests. |
| 09 | [Whole-task Budgets and Loop Detection](2026-09-07-09-whole-task-budgets-loop-detection.md) | 5 | Bound the entire task's resources across every worker and retry while detecting repeated work without new evidence and preserving verification capacity. |
| 10 | [Semantic Code Navigation](2026-09-07-10-semantic-code-navigation.md) | 5 | Provide capability-aware symbol navigation and safe rename previews with reliable text-search fallback. |
| 11 | [Browser and Terminal Interaction Testing](2026-09-07-11-browser-terminal-interaction-testing.md) | 6 | Generate reproducible user-surface evidence from real managed browser and PTY sessions, including keyboard and resize behavior. |
| 12 | [/graph Save and Reuse](2026-09-07-12-graph-save-reuse.md) | 6 | Save and run validated native engineering graph definitions in .davinci/graphs without introducing a dynamic workflow language. |
| 13 | [/graph Monitoring and Intervention](2026-09-07-13-graph-monitoring-intervention.md) | 5 | Make graph execution inspectable and safely controllable by run and node without restarting unrelated successful work. |
| 14 | [/graph Diff, Fork, Rewind, Explain, Dry-run, Verify, Budget and Export](2026-09-07-14-graph-advanced-controls.md) | 7 | Provide eight safe engineering controls around the native graph with immutable history, current-source proof, and cumulative resource accounting. |

The original archive's exclusion of **Claude-style dynamic workflows** applies to every feature. The existing native graph controller is extended; no additional workflow language, arbitrary execution DSL, or second orchestration runtime is proposed.

## Architectural direction

The core decision is to extend the existing runtime, not duplicate it. `davinci-agent` owns execution identity, authoritative task state, capability and policy intersections, resource admission, and evidence contracts. `davinci-coding-agent` supplies native persistence, process, graph, semantic, and interaction adapters. `davinci-tui` renders snapshots and submits typed host commands; it never decides that an action is safe or a task complete. Existing session and artifact stores remain migration inputs and compatible projections.

Four concepts remain distinct: **Living Plan** describes intended work and human acceptance; **Task** records executable work and ownership; **Decision** records a user's answer; **Evidence** records what actually happened against a particular source state. Approval does not mean completion, a question answer does not mean permission, and a graph artifact does not prove a command ran.

The repository already supplies useful foundations. The highest-risk work is therefore not visual polish: it is closing ownership races, honoring failed decision hooks and storage operations, correlating actual process termination, binding evidence to current bytes, and making batch/alias/custom-tool paths obey the same admission boundary. See the [audit](2026-09-07-00-repository-audit.md) for inspected evidence and limitations.

## Sequencing without circular dependencies

Feature-level dependencies describe the finished integration, not an instruction to finish two mutually dependent features before either starts. Execute the following task-level waves. A trait-only seam can land with a denying implementation; it must not advertise an enabled capability until its production adapter passes.

| Wave | Work | Exit condition |
|---|---|---|
| 0 — Baseline and contracts | Read current guidance; capture HEAD/dirty files; freeze shared IDs, command envelopes, source manifest, evidence receipt, budget lease, lifecycle, and graph DTO interfaces. | One owner per shared module; legacy fixtures identified; safety defaults and serialization rules agreed. |
| 1 — Durable identity and interaction | F03 Tasks 1–3; F01 Tasks 1–5. F10 pure transport/position contracts and F11 fake backends may proceed in disjoint files. | Acknowledged task changes survive replay; one CAS claim winner; modal cannot invent grants or hang after disconnect. |
| 2 — Execution authority | F07 Tasks 1–3; F09 Tasks 1–3; F06 Tasks 1–3; F05 Tasks 1–3. Shared interfaces from Wave 0 allow separate implementation. | Real stop/drain, atomic budget admission, trusted receipts, and final execution gates cover ordinary and batch calls. |
| 3 — Safe user controls | Complete F03 and F05–F09; F04 rewind; F02 decisions. Finish F07 retry/handoff after owned effects exist. | Rewind preserves unrelated bytes; current evidence gates completion; scope expansion requires a real user; context controls cannot remove mandatory policy. |
| 4 — Native graph and optional backends | F12 strict saved definitions and executable bindings; F13 lifecycle and node controls; F10 rename/launch integration; F11 provisioned PTY/browser adapters. | Saved topology actually controls execution; pause is acknowledged at a safe boundary; parser/backend dependency admission passes Rust 1.83 and offline gates. |
| 5 — Advanced operations and release | F14; complete F11 cross-feature scenarios; full compatibility, crash, malicious-input, and installed-binary evidence matrix. | All 75 requirements mapped to evidence; no untested required path is called complete. |

The critical path is durable task identity → control/budget/evidence/scope gates → task-owned effects and rewind → validated graph execution → graph lifecycle → advanced graph controls. Clarification UI can follow the approval bridge independently. Read-only views and pure parsers can be developed earlier against fixtures, but do not bypass their eventual authority gates.

## Ownership and parallel work

| Contract / shared area | Initial owner | Consumers |
|---|---|---|
| Authoritative task commands, revisions, owner generations, persistence | F03 | F02, F04–F09, graph adapters |
| Approval challenge and host-authenticated response | F01 | F02 questions and F05 scope decisions reuse transport, not grant semantics |
| User Decision records | F02 | Living Plan, task prerequisites, scope interview adapters |
| Source manifest and verification evidence | F06 | F04 checkpoints, F08 provenance, F10 rename, F11 tests, F12–F14 replay |
| Owned process/control registry | F07 | F04 draining, F09 limits, F10 servers, F11 test children, F13 graph controls |
| Root budget ledger and attempt accounting | F09 | Every provider/tool/worker/review/verification attempt |
| Task contract and confined effect executor | F05 | All execution adapters; no UI-specific bypass |
| Owned mutation/checkpoint/rewind service | F04 | F07 diffs/retry and F14 graph rewind |
| Final-provider context manifest and overlays | F08 | Inspector and evidence provenance |
| Strict reusable graph schema and native bindings | F12 | F13 node identity; F14 operations/export |
| Graph lifecycle/attempt receipts | F13 | F14 operations and UI |
| Immutable graph checkpoint history and retention | F14 | Fork, rewind, export, audit |

Only one integrator edits `turn.rs`, `runtime_host.rs`, `main.rs`, `davinci_interactive.rs`, TUI model/app routing, and graph controller at a time. Do not assign these files to multiple simultaneous implementation workers. Independent transport, parser, store, and fixture tasks can run in separate worktrees after contracts are fixed. A task's staging commands are a maximum path scope; use patch staging when a shared file contains unrelated work.

## Compatibility and rollout

Use additive, explicitly versioned metadata with legacy adapters. Existing session JSONL, `.pi` discovery, five permission modes, command aliases, print/JSON/RPC behavior, and graph state-v1 fixtures remain required. Do not silently reinterpret legacy graph zero limits, restore blanket execution consent, or convert simulated verification into real proof.

Optional integrations remain unavailable until their dependency and host capability gates pass. Read-only previews may be exposed first, followed by explicitly enabled mutation paths. A rollback can hide a UI entry or disable a new optional backend, but must preserve journals, historical evidence, consumed budget, and recovery records. It cannot disable mandatory scope/ownership checks for an already accepted scoped task.

New parser/PTY/browser dependencies are an explicit implementation admission task. Exact candidate versions are recorded in the shared contract, but their Rust/MSRV and platform compatibility were not established by a build in this documentation session. No packages were installed while preparing these plans.

## Verification and delivery

The first step in an execution session is a fresh baseline, not assuming the audit still describes HEAD. Every task proves its intended red state, then a green integrated result. Zero tests selected, unavailable browsers, missing scanners, and pre-existing failures are separate outcomes, not passes.

Use the repository's normal wrappers and pinned toolchain:

```text
rtk cargo fmt --check
rtk cargo test -p davinci-agent --offline
rtk cargo test -p davinci-coding-agent --offline
rtk cargo test -p davinci-tui --offline
rtk cargo test --workspace --offline
rtk cargo clippy --workspace --all-targets --offline -- -D warnings
rtk cargo build -p davinci-coding-agent --offline
```

These are **future implementation commands**, not results from plan preparation. Broader compatibility testing includes resumed/corrupt sessions, untrusted workspaces, cancellation at each admission boundary, Windows and Unix path/process behavior, and honest handling of unavailable optional capabilities. Real UI verification includes the unfinished-draft five-mode Shift+Tab cycle and ordinary Tab behavior, not just an enum test.

Executable delivery later must resolve the actual launched binary, follow the repository's backup/replacement guidance without interrupting a user's active session, and compare built/installed hashes. A renderer fixture, a real PTY run, a browser test, and a physical-keyboard check are different evidence classes.

## Program exit criteria

- [ ] All 75 requirement IDs have current-source passing evidence or an explicit required manual gap that prevents the corresponding completion claim.
- [ ] Every mutation route, including aliases, batch inner operations, custom/MCP tools, graph children, semantic edits, and interactive stdin, has an authority/effect decision.
- [ ] Crash recovery preserves acknowledged state, prior user edits, owner generations, irreversible-effect warnings, and cumulative spend.
- [ ] No task completion depends solely on model prose, stale code, simulated runs, zero checks, or ignored persistence errors.
- [ ] Optional backends do not start or install automatically, and unsupported capabilities are visible rather than silently emulated as success.
- [ ] Public documentation, command discovery, old-wire fixtures, and installed-runtime evidence agree with the implemented release.

**Current delivery scope:** Markdown plans and this roadmap only. Application functionality has not been implemented by this planning task.
