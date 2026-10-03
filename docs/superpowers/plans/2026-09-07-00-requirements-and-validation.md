# Davinci Feature Expansion — Requirements and Validation Matrix

**Status:** implementation acceptance specification; checkboxes are intentionally unchecked.

The 75 requirement IDs below summarize the complete 14-feature source set at a testable level. Each has at least one implementing task and a proposed contract test. Read the source transcription and all additional scenarios in the linked plan: this matrix does not reduce the original feature to one helper assertion.

## F01 — [Permission Approval Modal](2026-09-07-01-permission-approval-modal.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F01-1 | Every policy Ask gets one typed challenge that explains action, target, reason, and stopping mode/policy. | Task 1: `f01_legal_scope`; Task 3: `f01_modal_traps_mode_key`; Task 5: `f01_noninteractive_fail_closed` |
| F01-2 | The policy engine controls legal one-shot/session/project/denial choices; high-risk actions do not automatically gain persistent choices. | Task 1: `f01_legal_scope`; Task 2: `f01_stale_reply`; Task 4: `f01_grant_binding` |
| F01-3 | Up/Down and numbered options select, Enter explicitly confirms, and Esc denies/cancels. | Task 3: `f01_modal_traps_mode_key`; Task 5: `f01_noninteractive_fail_closed` |
| F01-4 | The modal traps Shift+Tab, ordinary composer keys, paste, and voice insertions without losing the preexisting draft. | Task 3: `f01_modal_traps_mode_key`; Task 5: `f01_noninteractive_fail_closed` |
| F01-5 | All hosts fail closed on stale replies, cancellation, transport loss, invalid scope, or storage failure; persistent grants remain narrowly scoped. | Task 2: `f01_stale_reply`; Task 4: `f01_grant_binding`; Task 5: `f01_noninteractive_fail_closed` |

## F02 — [Structured Clarification / Decision Interview](2026-09-07-02-structured-clarification.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F02-1 | Investigate before asking; require a materiality rationale and repository evidence references. | Task 1: `f02_question_bounds`; Task 3: `f02_waiting_mode` |
| F02-2 | Provide 2–4 concrete choices where possible, an optional recommendation, and an appropriate custom-answer route. | Task 1: `f02_question_bounds`; Task 4: `f02_enter_required` |
| F02-3 | Expose ask_user_question and persist explicit structured answer state in Living Plan. | Task 2: `f02_recommendation_not_answer`; Task 3: `f02_waiting_mode`; Task 4: `f02_enter_required`; Task 5: `f02_answer_invalidates_approval` |
| F02-4 | Only an authenticated user selection creates AnsweredByUser; recommendation is not consent. | Task 2: `f02_recommendation_not_answer`; Task 3: `f02_waiting_mode`; Task 4: `f02_enter_required`; Task 5: `f02_answer_invalidates_approval` |
| F02-5 | Preserve Open/AnsweredByUser/Deferred/Cancelled across resume and invalidate affected stale plan approvals. | Task 2: `f02_recommendation_not_answer`; Task 5: `f02_answer_invalidates_approval` |

## F03 — [Execution Tasks / Live Task Board](2026-09-07-03-execution-tasks-live-board.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F03-1 | Persist id/title/status/owner/dependencies/parent plan step/evidence references/timestamps separately from the Living Plan. | Task 1: `f03_public_status`; Task 3: `f03_no_publish_without_commit`; Task 5: `f03_resume_lineage` |
| F03-2 | Support pending/in_progress/completed/blocked/failed/cancelled in the UI without destroying internal readiness semantics. | Task 1: `f03_public_status`; Task 5: `f03_resume_lineage` |
| F03-3 | Expose task_create/task_update/task_list/task_get and live board snapshots. | Task 4: `f03_dependency_gate`; Task 5: `f03_resume_lineage` |
| F03-4 | Only the owning worker or authorized controller can mutate; claims and handoffs are atomic and revision checked. | Task 2: `f03_claim_race`; Task 3: `f03_no_publish_without_commit`; Task 4: `f03_dependency_gate` |
| F03-5 | Resume dependencies, ownership history, and evidence references safely; completion must not be inferred from plan acceptance. | Task 3: `f03_no_publish_without_commit`; Task 4: `f03_dependency_gate`; Task 5: `f03_resume_lineage` |

## F04 — [Task-level Rewind with Dirty-file Preservation](2026-09-07-04-task-level-rewind.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F04-1 | Checkpoint before substantial mutations and at verified milestones with task-owned effect provenance. | Task 1: `f04_checkpoint_required`; Task 2: `f04_restore_classification` |
| F04-2 | Protect preexisting dirty content and later manual changes; overlapping edits must conflict rather than overwrite. | Task 1: `f04_checkpoint_required`; Task 2: `f04_restore_classification`; Task 3: `f04_stale_preview` |
| F04-3 | Provide code-only, task/conversation-state-only, or combined restore selections. | Task 4: `f04_restore_selection`; Task 5: `f04_external_effect_not_undo` |
| F04-4 | Separate reversible local effects from irreversible publish/deploy or other external effects. | Task 5: `f04_external_effect_not_undo` |
| F04-5 | Make rewind previewable, revision checked, crash recoverable, and evidence invalidating without deleting history. | Task 3: `f04_stale_preview`; Task 4: `f04_restore_selection`; Task 5: `f04_external_effect_not_undo` |

## F05 — [Task-scoped Execution Contracts](2026-09-07-05-task-scoped-contracts.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F05-1 | Bind writable/protected scope, dependency policy, external action policy, and verification requirements to an accepted task. | Task 1: `f05_protected_wins`; Task 3: `f05_unknown_effects_blocked`; Task 5: `f05_requirement_not_skippable` |
| F05-2 | Enforce scope on every mutation including batch, aliases, shell, MCP/native extensions, and children. | Task 2: `f05_modes_do_not_bypass_scope`; Task 3: `f05_unknown_effects_blocked` |
| F05-3 | Require an explicit contract revision for expansion, with approve/in-scope alternative/deny-with-instructions choices. | Task 4: `f05_expansion_requires_user` |
| F05-4 | Always Approve never bypasses task ownership/contracts or hard filesystem/platform boundaries. | Task 1: `f05_protected_wins`; Task 2: `f05_modes_do_not_bypass_scope`; Task 3: `f05_unknown_effects_blocked`; Task 4: `f05_expansion_requires_user`; Task 5: `f05_requirement_not_skippable` |
| F05-5 | Provide auditable, race-safe enforcement and a clearly stated limitation for effects that cannot be contained. | Task 2: `f05_modes_do_not_bypass_scope`; Task 3: `f05_unknown_effects_blocked`; Task 4: `f05_expansion_requires_user`; Task 5: `f05_requirement_not_skippable` |

## F06 — [Evidence-backed Completion](2026-09-07-06-evidence-backed-completion.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F06-1 | Record command/tool provenance, execution outcome, relevant source fingerprint, useful runtime versions, and artifacts. | Task 1: `f06_source_must_match`; Task 2: `f06_no_skipped_success`; Task 5: `f06_report_gap` |
| F06-2 | Mark evidence stale when relevant inputs change and reject prior-source success as current proof. | Task 1: `f06_source_must_match`; Task 3: `f06_required_dimensions`; Task 5: `f06_report_gap` |
| F06-3 | Distinguish implemented/tested/built/installed/live-verified and show remaining gaps. | Task 3: `f06_required_dimensions`; Task 4: `f06_installed_hash_match`; Task 5: `f06_report_gap` |
| F06-4 | Reserve verification/handoff resources and refuse proof-backed completion without required current evidence. | Task 3: `f06_required_dimensions`; Task 4: `f06_installed_hash_match` |
| F06-5 | Use host-authenticated evidence with precise source coverage and no false success from skipped/unavailable checks. | Task 1: `f06_source_must_match`; Task 2: `f06_no_skipped_success`; Task 3: `f06_required_dimensions`; Task 5: `f06_report_gap` |

## F07 — [Live Task and Agent Control Panel](2026-09-07-07-live-task-agent-controls.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F07-1 | Display current activity, owned paths, dependencies/waits, elapsed time, tool/usage counts for every active worker. | Task 1: `f07_stale_control_rejected`; Task 5: `f07_panel_actions` |
| F07-2 | Steer one active worker without restarting the task and show redirect versus queued-follow-up semantics. | Task 2: `f07_steering_delivery`; Task 5: `f07_panel_actions` |
| F07-3 | Stop one worker or its task-owned process tree with real execution acknowledgment. | Task 3: `f07_stop_requires_exit`; Task 5: `f07_panel_actions` |
| F07-4 | Inspect recent tool calls and owned diffs; retry safely without resetting unrelated work. | Task 1: `f07_stale_control_rejected`; Task 4: `f07_retry_lease`; Task 5: `f07_panel_actions` |
| F07-5 | Enforce write ownership and explicit handoff through the runtime rather than prompts. | Task 1: `f07_stale_control_rejected`; Task 2: `f07_steering_delivery`; Task 3: `f07_stop_requires_exit`; Task 4: `f07_retry_lease` |

## F08 — [Context and Memory Inspector](2026-09-07-08-context-memory-inspector.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F08-1 | Show the final prepared request context and why each item is present, including token breakdown. | Task 1: `f08_total_includes_mandatory`; Task 4: `f08_overlay_next_request`; Task 5: `f08_visible_freshness` |
| F08-2 | Support inspect/pin/exclude/refresh with explicit next-boundary application and bounded context. | Task 2: `f08_cannot_exclude_policy`; Task 4: `f08_overlay_next_request`; Task 5: `f08_visible_freshness` |
| F08-3 | Distinguish user decision, repository fact, agent inference, and tool evidence provenance and freshness. | Task 1: `f08_total_includes_mandatory`; Task 3: `f08_inference_not_fact`; Task 5: `f08_visible_freshness` |
| F08-4 | Allow explicit user correction/removal of memory without silent reinjection or inference promotion. | Task 3: `f08_inference_not_fact`; Task 5: `f08_visible_freshness` |
| F08-5 | Mandatory system/permission/trust/safety context cannot be excluded. | Task 1: `f08_total_includes_mandatory`; Task 2: `f08_cannot_exclude_policy`; Task 4: `f08_overlay_next_request`; Task 5: `f08_visible_freshness` |

## F09 — [Whole-task Budgets and Loop Detection](2026-09-07-09-whole-task-budgets-loop-detection.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F09-1 | Share token/time/concurrency/retry/cost accounting across parent, descendants, reviews, verification, and provider retries. | Task 1: `f09_implementation_preserves_reserve`; Task 2: `f09_charge_once`; Task 5: `f09_unknown_cost` |
| F09-2 | Show cost as unknown where pricing/usage is unavailable; do not reset budgets on recursion or retry. | Task 1: `f09_implementation_preserves_reserve`; Task 2: `f09_charge_once`; Task 5: `f09_unknown_cost` |
| F09-3 | Detect repeated unchanged reads/commands, oscillating edits, and failed tests without a new experiment. | Task 3: `f09_repeated_work`; Task 5: `f09_unknown_cost` |
| F09-4 | Offer Continue within remaining budget, return to Plan Mode with evidence, or stop preserving a checkpoint. | Task 4: `f09_continue_cannot_bypass_ceiling`; Task 5: `f09_unknown_cost` |
| F09-5 | Reserve verification and handoff capacity and enforce limits under concurrency, crash/replay, and unknown usage. | Task 1: `f09_implementation_preserves_reserve`; Task 2: `f09_charge_once`; Task 4: `f09_continue_cannot_bypass_ceiling`; Task 5: `f09_unknown_cost` |

## F10 — [Semantic Code Navigation](2026-09-07-10-semantic-code-navigation.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F10-1 | Support definitions, references, outline, diagnostics, rename preview, and call hierarchy where advertised. | Task 1: `f10_capability_fallback`; Task 3: `f10_utf16_boundaries`; Task 5: `f10_rename_preconditions` |
| F10-2 | Keep ordinary file/text search available when semantic support is unavailable or incomplete. | Task 1: `f10_capability_fallback`; Task 4: `f10_server_launch_authority` |
| F10-3 | Manage server startup, capabilities, cancellation, framing, resource bounds, and workspace trust safely. | Task 1: `f10_capability_fallback`; Task 2: `f10_length_counts_bytes`; Task 4: `f10_server_launch_authority` |
| F10-4 | Bind locations/diagnostics/edits to document versions and source fingerprints, including Unicode/CRLF handling. | Task 3: `f10_utf16_boundaries`; Task 5: `f10_rename_preconditions` |
| F10-5 | Rename changes require normal permissions/contracts/checkpointing and verification; LSP output is not proof of correctness. | Task 5: `f10_rename_preconditions` |

## F11 — [Browser and Terminal Interaction Testing](2026-09-07-11-browser-terminal-interaction-testing.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F11-1 | Launch Davinci in a controlled terminal, send keys and resize, and assert rendered state. | Task 1: `f11_backend_capabilities`; Task 2: `f11_resize_bounds`; Task 3: `f11_key_encoding`; Task 4: `f11_five_mode_cycle` |
| F11-2 | Cover five-mode Shift+Tab cycling with an unfinished draft preserved and ordinary Tab behavior unchanged. | Task 3: `f11_key_encoding`; Task 4: `f11_five_mode_cycle` |
| F11-3 | Record screen frames, event logs, process exits, and exact executable/source identity as evidence. | Task 2: `f11_resize_bounds`; Task 3: `f11_key_encoding`; Task 4: `f11_five_mode_cycle`; Task 6: `f11_evidence_environment` |
| F11-4 | For web tasks, assert browser state and capture screenshots/DOM, console errors, failed requests, and useful traces. | Task 1: `f11_backend_capabilities`; Task 5: `f11_network_fixture_only`; Task 6: `f11_evidence_environment` |
| F11-5 | Run reproducibly with isolated resources, no live services/downloads, bounded artifacts, and no false claim of physical-keyboard verification. | Task 1: `f11_backend_capabilities`; Task 2: `f11_resize_bounds`; Task 3: `f11_key_encoding`; Task 5: `f11_network_fixture_only`; Task 6: `f11_evidence_environment` |

## F12 — [/graph Save and Reuse](2026-09-07-12-graph-save-reuse.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F12-1 | Implement /graph save <name> and /graph run <name> with .davinci/graphs/<name>.yaml storage. | Task 1: `f12_safe_graph_name`; Task 4: `f12_overwrite_requires_explicit`; Task 5: `f12_reserved_word_escape`; Task 6: `f12_replay_requires_all_bindings` |
| F12-2 | Save topology, roles, artifact contracts, budgets, verification policy, and safe parameters only. | Task 2: `f12_role_artifact_coherence`; Task 3: `f12_frozen_definition`; Task 4: `f12_overwrite_requires_explicit`; Task 6: `f12_replay_requires_all_bindings` |
| F12-3 | Reject unsafe/malformed definitions and preserve DAG, one-writer, mode-specific verification/security/review gates. | Task 1: `f12_safe_graph_name`; Task 2: `f12_role_artifact_coherence`; Task 3: `f12_frozen_definition`; Task 6: `f12_replay_requires_all_bindings` |
| F12-4 | Execute the saved definition through real native bindings without silently reclassifying/replacing it. | Task 3: `f12_frozen_definition`; Task 6: `f12_replay_requires_all_bindings` |
| F12-5 | Do not save hidden reasoning, transcripts, ad-hoc prompts, credentials, or dynamic workflow scripts. | Task 1: `f12_safe_graph_name`; Task 2: `f12_role_artifact_coherence`; Task 4: `f12_overwrite_requires_explicit`; Task 6: `f12_replay_requires_all_bindings` |
| F12-6 | Preserve bare /graph and legacy goal/flag behavior with an explicit escape for reserved subcommand words. | Task 5: `f12_reserved_word_escape`; Task 6: `f12_replay_requires_all_bindings` |

## F13 — [/graph Monitoring and Intervention](2026-09-07-13-graph-monitoring-intervention.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F13-1 | Show phases, worker states, time/usage, outputs, dependencies, and blocked reasons in a dedicated graph view. | Task 1: `f13_pause_ack_boundary`; Task 4: `f13_graph_keys`; Task 5: `f13_restart_never_resurrects_worker` |
| F13-2 | Pause/resume at safe boundaries and never start new nodes after an acknowledged pause. | Task 1: `f13_pause_ack_boundary`; Task 2: `f13_no_dispatch_when_paused`; Task 4: `f13_graph_keys`; Task 5: `f13_restart_never_resurrects_worker` |
| F13-3 | Stop/retry a selected node without restarting compatible unrelated successes. | Task 2: `f13_no_dispatch_when_paused`; Task 3: `f13_invalidate_descendants`; Task 4: `f13_graph_keys`; Task 5: `f13_restart_never_resurrects_worker` |
| F13-4 | Inspect public prompt contracts, tool activity, artifacts, verification, and owned diffs. | Task 4: `f13_graph_keys`; Task 5: `f13_restart_never_resurrects_worker` |
| F13-5 | Preserve deterministic ownership, one writer, bounded fan-out, current evidence, and durable lifecycle controls. | Task 1: `f13_pause_ack_boundary`; Task 2: `f13_no_dispatch_when_paused`; Task 3: `f13_invalidate_descendants`; Task 5: `f13_restart_never_resurrects_worker` |

## F14 — [/graph Diff, Fork, Rewind, Explain, Dry-run, Verify, Budget and Export](2026-09-07-14-graph-advanced-controls.md)

| Requirement | Required behavior | Implementing tasks / proposed test entry points |
|---|---|---|
| F14-1 | Diff original/prior/current validated graph/state and separately present owned code changes. | Task 2: `f14_read_operations_do_not_dispatch`; Task 7: `f14_export_allowlist` |
| F14-2 | Fork from a selected node with a different bounded strategy while preserving compatible earlier evidence. | Task 1: `f14_replay_content_not_status`; Task 4: `f14_branch_keeps_spend` |
| F14-3 | Rewind to the task-owned checkpoint before a writer without overwriting dirty/manual edits. | Task 1: `f14_replay_content_not_status`; Task 4: `f14_branch_keeps_spend` |
| F14-4 | Explain node purpose, dependencies, permissions, artifacts, and exit criteria deterministically. | Task 2: `f14_read_operations_do_not_dispatch`; Task 7: `f14_export_allowlist` |
| F14-5 | Dry-run validates topology/budgets/scopes/roles/permissions without executing workers. | Task 3: `f14_preflight_zero_effects`; Task 7: `f14_export_allowlist` |
| F14-6 | Verify-only runs current-source verification/review without implementation workers or simulated proof. | Task 5: `f14_verify_never_runs_writer`; Task 7: `f14_export_allowlist` |
| F14-7 | Inspect/adjust only authorized root resource ceilings without resetting cumulative usage. | Task 6: `f14_budget_floor`; Task 7: `f14_export_allowlist` |
| F14-8 | Export the validated declarative definition for review/reuse, excluding private transient content. | Task 7: `f14_export_allowlist` |
| F14-9 | All operations preserve immutable history, revision checks, exact replay identity, and mode-specific native safety gates. | Task 1: `f14_replay_content_not_status`; Task 2: `f14_read_operations_do_not_dispatch`; Task 3: `f14_preflight_zero_effects`; Task 4: `f14_branch_keeps_spend`; Task 5: `f14_verify_never_runs_writer`; Task 6: `f14_budget_floor`; Task 7: `f14_export_allowlist` |

## Cross-feature scenarios

Each scenario needs a fixture-driven integrated test and, where indicated, a provisioned real-environment check. Use injected clocks, barriers, fake provider/tool executors, isolated session roots, and explicit command receipts; avoid timing sleeps as a race oracle.

| Scenario | Features | Required observable outcome |
|---|---|---|
| Concurrent claim and stale worker | F03/F05/F07 | Two contenders race the same revision; one claim wins. Old owner cannot edit, finish, stop another owner, or release the new lease. |
| Completion versus source mutation | F03/F06/F09 | A source edit or owner change between verification and commit makes completion stale; unknown or missing evidence blocks done while reserve/spend remains accurate. |
| Dirty-file rewind | F04/F05/F06/F14 | Preexisting dirty bytes and later manual edits survive. Exact post-images invert; overlaps conflict. Transcript-only rewind never claims file restoration. |
| Batch/alias/custom effects | F01/F05/F06/F09 | Every inner operation is admitted and recorded once; aliases, stdin, and custom/MCP effects cannot bypass scope, budget, or receipt provenance. |
| Decision versus permission | F01/F02/F05 | A recommended answer, canceled question, model-written user flag, or stale RPC reply cannot approve tools or widen scope. |
| Stop/pause/retry lifecycle | F07/F09/F13 | Pause blocks new admissions, stop observes owned exit, retry invalidates descendants; siblings and user shells remain alive. |
| Crash and corrupted persistence | F03/F04/F09/F13/F14 | Acknowledged commits replay; denied/prepared records do not become completed. Interior corruption fails closed; spend/owner generations do not reset. |
| Protected context and privacy | F02/F08/F11/F14 | Mandatory policy cannot be excluded. Cross-profile memory stays isolated; exported context, traces, or logs do not silently publish secrets. |
| Saved-definition trust and replay | F05/F06/F12/F14 | Invalid YAML/roles/gates or unauthorized inputs fail before spawn. Frozen bindings run as saved; same-status dirty-byte changes invalidate replay. |
| Live UI and unavailable backends | F01/F02/F07/F08/F10/F11/F13 | Draft/caret survive exact five-mode cycle; modals block cycling. Missing LSP/browser/PTY is unavailable evidence, not an empty successful result. |
| Simulation versus verification | F06/F11/F12/F14 | Dry-run/simulated exit zero and capture-only screenshots do not satisfy required real command/assertion proof. |
| Legacy/noninteractive compatibility | All | Old JSONL/state-v1/config aliases load; print/JSON/RPC stay well formed; no interaction waits on absent stdin or grants permission by default. |

## Host and platform coverage

| Surface | Required checks | Evidence boundary |
|---|---|---|
| Core inline tests | State transitions, CAS, admission, receipt provenance, parsing bounds, source hashes, cancellation and recovery. | Pure helpers prove only their predicates; use real adapter calls for integration assertions. |
| Native Ratatui | Ask/decision/scope focus; draft/caret/paste/resize; five-mode cycle; task/context/graph selected identity and receipts. | Rendered frame plus dispatched input/state assertions. |
| Legacy TUI | Same legal choices/cancellation or an explicit reduced capability; no implicit success fallback. | Legacy adapter tests and explicit unsupported results. |
| Print/JSON | No waiting for absent user; no prompts contaminating stdout; denied/unavailable results remain machine-readable. | Exit/status and parsed output fixtures. |
| RPC | Capability-negotiated payloads, matching IDs, malformed/unoffered choices, timeout/EOF, exactly-once response, queued unrelated commands. | Real parser/waiter adapter with controlled transport close. |
| Windows | Case/drive/UNC/reparse behavior; locked files; process ownership; ConPTY lifecycle; actual binary resolution. | Native fixtures and provisioned managed processes; no unrelated user process termination. |
| Unix | Symlink/permissions/rename behavior; owned process groups; PTY resize/EOF and cancellation. | At least one supported Unix target, with platform-specific gaps stated. |
| Optional browser/LSP | No implicit install; actual capability/version identity; network/server effects and bounded messages; browser assertions and artifacts. | Provisioned real backend where required; missing backend stays a coverage gap. |

## Test execution protocol

- [ ] Re-read repository guidance and record current HEAD, dirty paths, toolchain, and baseline failures before implementation.
- [ ] Add each plan's proposed test in its indicated inline module, register new modules, run its filter, and confirm the intended assertion/missing-contract red state. A missing toolchain or zero selected tests is not red proof.
- [ ] Implement the production adapter and all additional scenarios, not just the displayed helper; re-run targeted and related module tests.
- [ ] Record command, working directory, source manifest, selected test count, result/exit, and output/artifact reference. Keep unknown/skipped/simulated outcomes distinct.
- [ ] Run formatting, workspace offline tests, and Clippy with the pinned toolchain; preserve known baseline issues explicitly rather than silently waiving them.
- [ ] Run required managed PTY/browser and installed-binary checks only in isolated approved environments. Physical keyboard behavior is a separate manual evidence item.
- [ ] Confirm every requirement's final evidence is still fresh after the last code/config/dependency change.

## Release evidence record

For each feature retain: source/contract revision, implementation commits, exact test commands and counts, source/build/runtime identities, environment/backend versions, actual outcomes, output hashes, relevant screenshots/DOM/terminal frames, privacy/redaction status, and unresolved gaps. A final report must distinguish **implemented**, **tested**, **built**, **installed**, **automated interaction verified**, and **manually checked** rather than collapsing them into “done.”

Documentation inventory/link/hash validation belongs to this planning delivery; it is not any feature's implementation acceptance evidence.
