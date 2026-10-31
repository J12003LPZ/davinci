# Gap closure decisions (Task M1)

Executed 2026-10-29 on branch `J12003LPZ/runtime-gap-closure` from the master plan `docs/superpowers/plans/2026-10-28-claude-runtime-gap-closure-master.md`. Source baseline: `current-source-baseline.md`. Per-finding dispositions for all 305 Claude evidence rows: `claude-finding-crosswalk.csv`.

Roles: Opus planned and reviewed only; GPT 6 Luna (high) did read-only research and the crosswalk triage; GPT 6.1 Sol (high) was the adversarial critic for every wave; one Sonnet 5.5 (medium) agent wrote all code and tests.

## Decision per gap

| ID | Disposition | Where | Evidence (tests) |
|---|---|---|---|
| G-01 | IMPLEMENTED_AND_TESTED | `davinci-mcp/src/{lib,stdio}.rs`, `davinci-agent/src/{mcp,tools,turn}.rs` | `a_lost_tool_reply_is_ambiguous_and_a_refused_call_is_not`, `a_partly_written_line_is_an_unknown_outcome`, `a_line_with_no_byte_written_is_provably_not_delivered`, `g01_*` eviction race tests |
| G-02 | IMPLEMENTED_AND_TESTED (commit 8f3effac, PR #259) | `davinci-agent/src/subagent_receipt.rs` | see `08-FINAL-IMPLEMENTATION-REPORT.md` |
| G-03 | IMPLEMENTED_AND_TESTED | `davinci-agent/src/runtime/worktree.rs` | `a_failing_inspection_keeps_the_lease_instead_of_assuming_absence`, `a_surviving_registration_keeps_the_lease_even_when_the_directory_is_gone`, `releasing_a_lease_with_a_missing_directory_keeps_unrelated_registrations`, `orphan_scan_reports_a_registration_whose_directory_and_branch_are_gone` |
| G-04 / X-09 | IMPLEMENTED_AND_TESTED (default Off path) | `davinci-agent/src/compaction.rs` | `repeated_compaction_carries_forward_user_requirements`, `summary_missing_user_requirement_is_rejected`, `oversized_user_requirements_block_keeps_original_history`. Idle compaction not added (no evidence Claude's fires; report 35). |
| G-05 | IMPLEMENTED_AND_TESTED | `davinci-agent/src/runtime/workflow/{state,executor,source_fingerprint}.rs` | `resume_reruns_artifacts_when_source_fingerprint_changes`, `legacy_artifact_without_fingerprint_is_rerun` |
| G-06 | IMPLEMENTED_AND_TESTED (commit 8f3effac) | `davinci-evals/src/behavior/{oracle,validity}.rs` | see `01-G06-TRUSTED-ORACLE.md` |
| G-07 | IMPLEMENTED_AND_TESTED (commit 8f3effac) | `davinci-agent/src/completion_status_tests.rs` | see `03-G07-COMPLETION-STATUS.md` |
| G-08 | DIFFERENT_SAFE, default unchanged | `davinci-agent/src/lib.rs` `ToolSurface` | Full stays default. Changing it needs a measured, authorized live comparison (plan C2/C4 gate). |
| G-09 | IMPLEMENTED_AND_TESTED | `davinci-server/src/lib.rs`, `davinci-coding-agent/src/experimental.rs` | `authorize_transport` empty/missing/last-byte tests, `run_server_refuses_to_listen_without_a_nonempty_token`, `pi_listen_refuses_to_bind_without_a_nonempty_token` (Linux, Docker). Unix peer-uid check not added: needs a new dependency. |
| G-10 | IMPLEMENTED_AND_TESTED | `davinci-coding-agent/src/{shutdown,main}.rs` | once-only, shared deadline, per-step panic isolation, owned processes first |
| G-11 | IMPLEMENTED_AND_TESTED | `davinci-agent/src/runtime/worker_admission.rs` | queue cap, wait timeout, `a_panicking_cancellation_callback_frees_its_queue_slot`, `an_expired_deadline_wins_over_capacity_freed_meanwhile`. Admitted-concurrency defaults unchanged. |
| G-12 | IMPLEMENTED_AND_TESTED | `davinci-mcp/src/lib.rs`, `davinci-agent/src/mcp.rs` | `g12_*`; id sent as `_meta["davinci/invocationId"]` on direct, batch and codemode paths |
| G-13 | IMPLEMENTED_AND_TESTED | `davinci-agent/src/turn.rs` | compatibility, batch, plan-batch, codemode (`script_child_is_refused_when_policy_moves_after_its_decision`) and journal gate tests |
| G-14 | IMPLEMENTED_AND_TESTED | `davinci-mcp` fixture + `davinci-agent/src/turn.rs` | `g14_a_call_cancelled_after_the_effect_committed_is_unknown_and_never_resent`, agent-level cancel-after-commit |
| G-15 | IMPLEMENTED_AND_TESTED (naming; routing unchanged) | `davinci-agent/src/{turn,tool_ledger}.rs` | `execution_authority_follows_each_tools_own_capability_and_the_journal` |
| G-16 | IMPLEMENTED_AND_TESTED | `davinci-agent/src/tool_ledger.rs` | `g16_*` (tombstones, journal ordering, saturation, legacy ledgers, admission budget) |
| G-17 | IMPLEMENTED_AND_TESTED | `davinci-agent/src/skills.rs` | `discovery_reads_only_bounded_frontmatter`, `invocation_reloads_body_after_file_change` |
| G-18 | DIFFERENT_SAFE | `runtime/workflow/executor.rs` | Dollar caps stay rejected (`max_cost_usd_is_rejected_until_supported`); Codex subscription usage has no reliable dollar figure. |
| X-01 | EXPERIMENTAL, not built | `subagent.rs` | Depth-1 stays. Plan gate O2: only after a measured benefit and approved coding-success study. |
| X-02 | DEFERRED (opt-in) | `jobs/supervisor` | Detached attach/list/logs supervisor is opt-in scope; no evidence it is needed by the headless path. |
| X-03 | DIFFERENT_SAFE | `lib.rs` `tool_search` | Lean/deferred exists; default change needs authorized live measurement (C2). |
| X-04 | PARTIAL | `skills.rs` | Lazy bodies done (G-17); scoped plugin refresh/revocation lifecycle not changed. |
| X-05 | BLOCKED | language_intelligence | Claude IDE peer handshake not dynamically established (reports 14, 38); no protocol to match. |
| X-06 | DEFERRED | `events.rs`, `provider_observation.rs` | Versioned lifecycle event contract is a separate, contract-level change. |
| X-07 | IMPLEMENTED_AND_TESTED for the core loop (Wave 5) | `turn.rs` | `b2_*` tests: reverse completion, denial, invalid input, mid-batch cancel, mutation barrier, output cap. Same-server MCP overlap is not covered. |
| X-08 | EXISTS_UNVERIFIED, tests added (Wave 5) | `davinci-session`, `lib_tests.rs`, `compaction.rs` | `b3_*` tests: torn tail bytes preserved, unresolved call closed and not rerun, orphan result refused, fork starts with an empty ledger, compaction keeps pairing. Crosswalk rows stay EXPERIMENTAL / BLOCKED (not open states). |
| X-10 | DIFFERENT_SAFE | `cache_stability.rs` | Measure-first; no basis to change cache behavior. |
| X-11 | DIFFERENT_SAFE | `permission.rs` | Deny precedence and sandbox kept; dispatch gaps closed under G-13. |
| X-12 | DEFERRED | bench | Baseline (M3) requires n>=5 paired local runs; not run in this branch. |
| X-13 | BLOCKED | team/mailbox | Claude team IPC not established; requires S3 transport identity first. |
| X-14 | BLOCKED | evals | Live paired model study needs explicit budget/authorization. |

## Review record

Every wave was reviewed cold by GPT 6.1 Sol from the diff alone (the Codex shell sandbox refused to start processes on this Windows host, so diffs were inlined). Each round's findings were fixed by the Sonnet coder and re-reviewed by a fresh critic. An automated security review also flagged the first compaction design (requirements parsed from model-written text); that was fixed by moving them into trusted metadata.

| Area | Rounds | Final verdict |
|---|---|---|
| MCP outcome, ledger tombstones (G-01, G-12, G-14, G-16) | 6 full + 1 scoped | PASS (scoped re-check of round-5 findings) |
| IPC auth, worktree, admission (G-09, G-03, G-11) | 5 full + 1 scoped | PASS (scoped) |
| Dispatch gate, shutdown (G-13, G-15, G-10) | 3 full + 1 scoped | PASS (scoped) |
| Compaction, skills, workflow fingerprints (G-04, G-17, G-05) | 1 full + 2 scoped | PASS (scoped) |

The final passes were scoped: they verified the previous round's findings and searched the new diff for P0/P1 defects only. The P2/P3 findings Sol raised that were fixed are in the commit messages; none was left open knowingly except those under "Known limitations".

## Known limitations

- Agent cancellation does not interrupt an MCP call already in flight; the call still ends as an unknown outcome on the transport timeout.
- A Unix peer-uid check on the IPC socket is not implemented (new dependency).
- `record_start_with_policy` (test-only caller) does not go through the tombstone admission budget.
- Workflow fingerprinting costs about 100 ms per artifact commit on Windows (several `git` spawns); a worker with a worktree handoff commits two artifacts. Sharing one fingerprint per worker completion is a possible follow-up.
- Hybrid Context VM compaction does not carry user requirements (unchanged; Hybrid is opt-in).
- Branch summaries are still identified by a text prefix (only compaction summaries moved to metadata).
- Not run: the full `davinci-coding-agent` suite (2400+ tests; only targeted subsets), interactive TTY, IDE peer, live provider runs, and the M3 performance baseline.
