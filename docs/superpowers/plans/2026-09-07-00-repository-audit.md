# Davinci Feature Expansion — Repository Audit and Planning Provenance

**Inspected checkout:** `C:\Users\sergi\Desktop\pi-rust` · **Branch/HEAD:** `main@9c42820` · **Planning baseline date:** September 7, 2026.

This is a static, read-only reconnaissance report supporting the implementation plans. A described code-path risk is not a claim that an exploit, race, hang, or test failure was reproduced. Existing behavior is distinguished from proposed work throughout the bundle.

## Inputs and boundaries

The supplied `davinci_feature_specs(1).zip` contains 14 DOCX feature specifications. Their paragraph/table text was extracted and their reference images inspected; source text is preserved as Markdown. The same interaction mockups recur across the documents. The source explicitly marks the features PROPOSED and excludes Claude-style dynamic workflows.

Repository guidance (`AGENTS.md`, `CLAUDE.md`), root/crate manifests, pinned toolchain, current source modules, recent history, and existing plan filenames were inspected. Three read-only recon workers covered permissions/decisions/UI, task/runtime safety/context, and native graph controls. They did not modify files, install dependencies, run application tests, launch the app, or spawn further agents. Their run was closed after reports were collected.

Initial tracked state was clean at HEAD; the existing untracked entry was `plugins/`. The user's preexisting plugins and older plans are outside this task. The output is a new dated Markdown set only; no source implementation, manifest update, staging, commit, or executable installation is part of this delivery.

## Verified constraints

Rust is pinned to 1.83.0 / edition 2021 with exact dependency versions. Implementation belongs in active `crates/*`; vendor trees are reference-only and archived/stale package layouts are not the current implementation. Protected system, compaction, and branch prompt constants must remain untouched. Inline offline tests and isolated fixtures are the normal repository pattern.

The current permission code has **five modes**, even where older guidance describes fewer: Manual (`Ask`), Accept Edits (`Edits`), Plan Mode (`ReadOnly`), Auto Mode (`Auto`), Always Approve. Auto is conservative, not a synonym for bypass. Explicit denies, freeze/boundary checks, and worker restrictions still matter. Library default compatibility and CLI default are different; the plans do not conflate them.

No native YAML parser dependency was found in the inspected Cargo manifests/lockfile. Existing flat frontmatter parsing is not sufficient for nested DAG YAML. Active source searches found no native LSP manager/client or reusable PTY/browser interaction-test service; these are bounded negative findings about inspected code, not claims about every external plugin or vendor package.

## Feature-by-feature findings

Paths are repository-relative. Line anchors are approximate at the inspected HEAD; resolve symbols again before implementation.

| Feature | Confirmed anchors | Existing foundation / material gap |
|---|---|---|
| 01 Approval | `davinci-agent/src/permission.rs:643–674`; `turn.rs:1102`; coding-agent `davinci_interactive.rs:1183,2340`; `main.rs:2795` (all under `crates/`) | Request/reply callback and native Ask overlay exist. Request lacks richer legal-scope/context binding; permission decision emission can ignore rejection; save-failure and malformed RPC reply fallbacks need explicit fail-closed handling. |
| 02 Clarification | agent `living_plan.rs:10,65,218`; `planning.rs:16,172,205`; `scheduler.rs:39`; native `run_turn` | Living Plan has questions and Boolean step decisions, not a typed user-answer store. No `ask_user_question` implementation found. RPC waiter is thread-local; ordinary Read lane alone would be inappropriate. |
| 03 Tasks | agent `runtime/tasks.rs:45,171,274,324,590`; `runtime/tools_task.rs:161,258`; `runtime/team.rs:156`; coding-agent `main.rs:1771` | TaskRegistry and replay exist. Claim/reassign lack owner/revision CAS, updates can partially mutate, completion races revalidation, and fresh RunId filtering can hide restored records. |
| 04 Rewind | graph `mutation.rs:22,162,188`; agent `apply_patch.rs:366,407,429`; `file_mutation_queue.rs:30` | Baseline deltas exclude earlier dirty edits but can include later manual edits and omit large baseline content. Existing patch-crash restoration is not a compare-guarded task undo service. |
| 05 Scope | agent `turn.rs:794,897`; `batch.rs:142–198`; `tools.rs:326`; graph `worker_hooks.rs:70,152` | Prepared tool pipeline, role allowlists, and policy exist. Task-specific immutable scopes need final effect admission, alias/batch/custom coverage, and honest opaque-process containment. |
| 06 Completion | agent `tool_ledger.rs:29`; `evidence.rs:19`; graph `verify.rs:101,137,146`; ecosystem `verification.rs:31–64` | Tool/output stores and real graph verifier exist. They do not provide universally fresh typed task proof. Simulated verifier exit 0 and optional unavailable security must not satisfy mandatory evidence. |
| 07 Controls | agent `runtime/tools_agent.rs:125,167`; `runtime/team.rs:225`; `runtime/cancellation.rs:72,108`; `jobs.rs:209,300`; `queues.rs:39,63` | Mailbox, cancellation tokens, jobs, and steering exist. `agent_stop_tool` changing registry state is not process termination; production delivery/exit acknowledgments and ownership checks need convergence. |
| 08 Context | agent `runtime/context.rs:14,66,82,163`; `lib.rs:534,572,605`; native `vector_memory.rs:608,911,1110`; `token_governor.rs:777,1032` | Broker/governor/scoped memory exist. Selected-item packets are not a complete final-provider manifest or exclusion log. Estimates are heuristic; refresh must not recollect context or expose other scopes. |
| 09 Budgets | agent `stats.rs:21,45`; `turn.rs:410,465,1273`; `runtime/capacity.rs:11`; graph `types.rs:418`; `controller.rs:229,249` | Usage, concurrency limits, graph caps, and retry counters exist. No common atomic root credit reservation across all work; different loop mitigations are not a full progress watchdog. |
| 10 Semantic | agent `tools.rs:539,1424,1794`; native Cargo manifest | Authorized text read/grep/find are fallback tools. A new LSP service must own launch/capabilities/position conversion/versioning and route rename through guarded edits. |
| 11 Interaction | tui `davinci/runtime.rs:133,672`; `app.rs`, `fixtures.rs`, `views/ask.rs` | Input/render fixtures and ConPTY paste handling exist. No active reusable real PTY/browser test service found; vendor xterm dependency is not a native backend. |
| 12 Saved graph | graph `topology.rs:28,54,108`; `controller.rs:324,828,947,1131`; `store.rs:177,188`; `mod.rs:627` | A graph definition/state format exists, but assigning an imported definition is not sufficient: the controller still has generated/hard-coded stages. Import validation and actual native execution bindings are required. |
| 13 Graph controls | graph `mod.rs:76,149,411,567`; `controller.rs:167,480`; tui `views/graph_run.rs:9,86` | Active registry, abort, resume, and read-only monitor exist. Pause and manual node retry need typed lifecycle/attempt receipts; selected-node UI and descendant invalidation are new work. |
| 14 Advanced graph | graph `replay.rs:20,118`; `store.rs:105,196`; `mutation.rs:67`; `verify.rs:137,146`; `controller.rs:414,829` | Diff/replay/simulation foundations exist. Content fingerprints, immutable history, honest preflight, verify-only, preserved attempt spend, and redacted export need stronger contracts. |

In the table, `agent` means `crates/davinci-agent/src/`; `coding-agent`/`native` means `crates/davinci-coding-agent/src/`; `graph` means its `native_extensions/graph/`; `ecosystem` means its `native_extensions/ecosystem/`; `tui` means `crates/davinci-tui/src/`.

## High-priority static risks captured by the plans

**Authority and atomicity.** `permission_denial` can ignore a runtime decision rejection. Current RPC/native approval fallbacks can substitute a session grant after an unoffered/persistence-failed choice. Task ownership can be overwritten without CAS; TeamManager claim is check-then-assign; task_update can reassign before rejecting a status. Completion validates before a decision callback but can commit without rechecking the current state under the final write lock. These are addressed by F01/F03/F05, not merely a new modal.

**Durability and restart.** Runtime observer logging can ignore append errors. Main startup can ignore read/rehydration errors while creating a new run identity. Graph checkpoint persistence ignores errors and overwrites mutable state. Acknowledged authority must move through committed durable records, with visible recovery errors and historical state migrations, in F03/F13/F14.

**Effects and proof.** Inner batch calls do not use the ordinary finalizer. Raw tool outcomes can be rewritten later; the proposed receipt boundary must preserve actual outcome plus hook veto. Graph mutation baselines are not exact ownership journals, and patch-crash recovery is not guarded task rewind. Missing/large content and unknown external effects cannot be treated as empty reversible changes. F04–F06 handle these distinctions.

**Lifecycle and accounting.** Registry Stopping/Cancelled is not observed process exit. Existing graph non-Windows cleanup differs from the agent JobBook group behavior. Graph replay can add historical usage again while retaining cumulative counters, and task-derived totals can omit failed/superseded usage; this was a static code-path concern, not a measured billing result. F07/F09/F13/F14 require nonzero usage and actual-exit fixtures.

**Graph validity and provenance.** Existing topology checks do not establish every imported node's role coherence or every conditional success path's applicable gates. Some runtime node IDs bypass readiness checks when absent from the definition. Current replay fingerprint uses HEAD/status (or shallow non-Git metadata), which can miss byte changes. These risks justify actual compiled bindings and content manifests, not just a YAML save button.

**Interaction and environment.** The RPC waiter needs explicit EOF handling; this potential hang was not reproduced. A renderer screenshot is not proof of keyboard routing, and a PTY key sequence is not a physical keyboard check. Browser traces can omit assertion semantics and contain sensitive data. F11 records environment-specific proof and redaction boundaries rather than asserting “UI tested” generically.

## Existing regression tests to extend

The following tests were identified in source, **not run** in this planning session:

| Area | Existing test examples |
|---|---|
| Permission matrix | `auto_mode_escalates_unknown_destructive_and_external_actions`; `plan_freeze_precedes_session_grants_and_side_effect_class_hints`; `explicit_shell_denies_survive_substitution_and_always_approve` |
| Modal input | `the_permission_panel_offers_always_only_in_a_trusted_project`; `under_the_permission_panel_esc_denies_and_enter_chooses_the_row`; `active_overlays_own_text_and_backspace_before_the_composer` |
| Plan separation | `plan_is_versioned_and_requires_explicit_human_approval`; `invalid_dependencies_and_model_self_approval_are_rejected_atomically`; `busy_runtime_cannot_change_plan_or_permissions` |
| Graph persistence/trust | `graph_definition_roundtrips_through_disk_and_sibling_file`; `security_untrusted_project_config_cannot_authorize_execution`; `bare_graph_continues_a_stopped_run_with_the_same_identity_and_counters` |
| Mutation/replay/gates | `graph_mutation_excludes_preexisting_uncommitted_user_edits`; `graph_replay_resume_refuses_incompatible_fingerprint_and_reexecutes`; `graph_review_coverage_approval_impossible_when_one_chunk_is_omitted`; `graph_security_gate_blocks_approval_on_high_risk_mutation` |

New test names beginning `f01_` through `f14_` in the plans are proposed contract tests, not existing repository tests. A helper assertion is an executable design example, not a substitute for its required production cases.

## Planning verification scope

The generated documents are checked for expected feature/spec inventory, requirement-to-task coverage, unique task test names, UTF-8/control characters, closed code fences, local Markdown links, and safe dated output paths. Existing/proposed implementation file classifications were checked against 114 concrete paths on the user's machine. File ownership markers identify modules introduced by prerequisite tasks rather than asking later workers to recreate them.

The original source archive, source transcriptions, generated document manifest, and ZIP use content hashes/integrity checks during packaging. The final Windows copy is compared with the packaged Markdown content. These are **documentation delivery checks**; they do not establish application behavior. Repository build/test/Clippy commands, real browser/PTY scenarios, physical keyboard checks, dependency compatibility, and installed-runtime verification remain future implementation gates.

This planning task makes no claim that application tests pass or any of the fourteen features have been implemented.
