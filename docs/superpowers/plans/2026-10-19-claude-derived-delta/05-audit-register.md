# Audit register: Claude-derived delta program

Review document for the branch `J12003LPZ/claude-derived-delta-6f43a004`. One row per plan task, written against the exact source snapshot. Status values: `PRESENT_AND_PROVEN`, `PRESENT_BUT_UNPROVEN`, `MISSING`, `NOT_WORTH_CHANGE`, and `FIXED` (with the red test that proved the defect).

## Baseline identity (M1)

| Item | Value |
|---|---|
| Source revision | `cd4e0e1560e920a1026507358a6e9c1cffc5b96f` (`origin/main`) |
| Toolchain | `rustc 1.88.0 (6b00bc388 2025-06-23)` (pinned by `rust-toolchain.toml`) |
| `Cargo.lock` SHA-256 | `5b65809fe5c1e7cd27c5e5edc3c2908af9a3d75554d8374767ca816b02a26835` |
| Provider routes | `crates/davinci-ai/src/catalogs.json`: 39 providers. The subscription route is `openai-codex` (api `openai-codex-responses`: `gpt-5.4`, `gpt-5.4-mini`, `gpt-5.3-codex-spark`, ...). Nothing in this program adds an API-key requirement or API fallback. |
| `cargo fmt --check` | exit 0 |
| `cargo test -p davinci-agent --offline --locked` | exit 0: 2186 passed, 0 failed, 7 ignored |
| `cargo test -p davinci-evals --offline --locked` | exit 0: 211 passed, 0 failed |
| `cargo run --offline --locked -p davinci-evals -- corpus verify` | exit 0: `verified 200 behavior scenarios` |
| Pre-existing failures | none |
| Raw logs | `target/davinci-delta-baseline/*.log` (untracked), `coverage.json` alongside |

## Plan corrections found while auditing

- Plan paths `src/...` and `tests/...` mean `crates/davinci-agent/src/...` and `.../tests/...`.
- `session_persistence_tests` is a lib unit module, not a Cargo test target.
- R4: `src/runtime/team.rs` has no lease code. Leases live in `runtime/worktree.rs`.
- B2: `corpus verify` covers only the core-200 corpus. The debugging-hard fixture is validated by the `davinci-evals` unit tests.
- O5: "permission change invalidates tool exposure" has a wrong premise. A permission-mode change does not alter tool exposure (see O3/O5 row).

## Register

| Task | Status | Owner files | Tests (existing / added) | What changed | Not run |
|---|---|---|---|---|---|
| R1 killed-tool resume | FIXED, red test `agent_crash_after_started_effect_reports_unknown_without_replay` | `crates/davinci-agent/src/turn.rs` (`reconcile_unresolved_tool_calls`), `src/lib.rs` (`prompt_with_origin`) | existing: `operation_publication.rs:385,844,890`, `operation_crash.rs`. added: `tests/agent_tool_resume_integration.rs` | A call left unanswered by a stopped process now gets an explicit "Outcome unknown" (or "Not executed") tool result, derived from the operation journal effect status (then the tool ledger). Never success, never re-run. Persisted next to the call when it is the newest exchange. | Test kills a real child process running the Agent. The effect tool is a native-extension tool, not built-in `bash` (the built-in shell needs the CLI-wired supervisor host). The journal attempt itself stays `EffectPossible` (it still blocks redispatch); moving it through Interrupt/RecoveryRequired needs an owner fence and is not done. |
| R2 tool-use/result pairing | FIXED (orphan and duplicate, red tests `orphan_result_id_fails_closed`, `duplicate_result_does_not_double_publish`); missing-result case shares the R1 code | `crates/davinci-agent/src/turn.rs` (`validate_loaded_tool_history`, `reconcile_unresolved_tool_calls`) | existing: `ws3_stream_cut.rs:524-646` (cut stream), `operation_publication.rs:844,890,1048`, `compaction.rs:1511`. added: `tests/tool_result_pairing_recovery.rs` (4 tests) | Resume refuses a persisted history holding a result whose call id was never issued (unless the journal accounts for it), before any provider request. A second result for one call id is dropped. Unanswered calls get the R1 unknown / not-executed result, inserted right after their own call. | `missing_result_after_recovery_...` and `compacted_history_retains_matching_ids` were green except for a wrong role-order expectation of mine in the first / green on first run in the second (R1 code already covered it; compaction already kept pairs). Orphan check is limited to agents with a session: several existing fixtures build orphan results in memory on purpose. Whether a real provider rejects an unpaired `function_call` was not tested (live). |
| R3 compaction freshness | NOT_WORTH_CHANGE (structurally unreachable) for the stale-summary race; `resume_after_compaction_keeps_tool_pairing` pinned by `compacted_history_retains_matching_ids`; `failed_or_empty_fold_leaves_active_state_unchanged` added | none | existing: `compaction.rs:1494`, `context_vm_integrity.rs:363`, `lib_tests.rs:4755`. added: `compacted_history_retains_matching_ids`, `failed_or_empty_fold_leaves_active_state_unchanged` in `tests/tool_result_pairing_recovery.rs` | No code change. Evidence for "unreachable": `Agent::compact` / `fold_context` are `&mut self` and the summarizer closure has no Agent access. Every host calls them on the agent's thread: TUI `davinci_interactive.rs:9006` runs `agent.compact` in a scoped worker that holds `&mut agent` while the UI thread only draws and drops keys; RPC `rpc.rs:424`, SDK `sdk.rs:163`, CLI `main.rs:7430` are synchronous. No `Mutex<Agent>` exists in the workspace. | No race test (not expressible without an API change). |
| R4 worktree leases | FIXED (stale release, red test `stale_owner_cannot_release_new_lease`); same-branch race PRESENT_AND_PROVEN | `crates/davinci-agent/src/runtime/worktree.rs` | existing: `worktree.rs` unit tests (dirty preservation at `:595`). added: `tests/worktree_lease_concurrency.rs` (`same_branch_concurrent_acquisition_has_one_winner` green on first run, 15 repeats stable; `stale_owner_cannot_release_new_lease`) | `WorktreeLease` gains `generation` (manager-assigned). `release_lease` compares the caller's lease with the manager's record at that path and returns `StaleLease` (no removal, even with `force`) when they differ. A path the manager does not know keeps the old behavior. Same-branch exclusion comes from git; production passes no explicit branch, so the race is not product-reachable. `team.rs` has no lease code (plan error). | |
