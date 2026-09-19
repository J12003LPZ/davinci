# DaVinci engineering program — final verification report

Date: 2026-09-19.
Write tree: `C:\Users\sergi\.claude-worktrees\pi-rust-9416e5cee6\01a0ad48`.
Shared checkout `C:\Users\sergi\Desktop\pi-rust` was not modified.

## Delivery

- `origin/main`: `5585fd6612aa48a0ce0d48536953ea1697c60836` (`Merge pull request #19 from J12003LPZ/codex/p12-live-eval-01a0ad48`).
- Merged PRs: #9 P1, #10 P2, #11 P4, #12 P3, #13 P5, #14 P7, #15 P6, #16 P9, #17 P8, #18 P10–P12, #19 P12 live 17-step.
- Isolated worktree on `codex/program-report-01a0ad48` from that SHA; twelve feature SHAs are ancestors of `origin/main`.

## Main CI on `5585fd6`

- Security SARIF: https://github.com/J12003LPZ/davinci/actions/runs/35420073914 success.
- CI: https://github.com/J12003LPZ/davinci/actions/runs/35420073891 success on attempt 2. Attempt 1 was cancelled after Windows `Test planning and filesystem observation` stayed in_progress from 04:02:32Z past 28 minutes (prior green Windows job finished that step in 3.3 minutes). `rerun-failed-jobs` started attempt 2 at 04:31:39Z; Windows native completed success.

Prior assembled main (P1–P12 without the live 17-step follow-up): `57975a5` CI https://github.com/J12003LPZ/davinci/actions/runs/35418243472 success.

## P12 live 17-step

Command (twice, both pass):

```text
cargo test --offline --locked -p davinci-coding-agent --bin davinci -- --ignored --nocapture login_button_seventeen_step
```

| Run | Duration | process_start_ms | browser_open_ms | impact cold/warm ms | repo_map_ms | workspace_checkpoint_ms |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 6.47s | 122.1111 | 418.2212 | 68.7418 / 38.2895 | 72.4989 | 49.8195 |
| 2 | 6.57s | 143.748 | 405.9771 | 66.2633 / 35.1222 | 41.8084 | 42.7217 |

Dispatched required tools: `repo_map`, `lsp_document_symbols`, `package_info`, `git_blame_symbol`, `impact_analyze`, `process_start`, `workspace_checkpoint`, `edit`, `lsp_diagnostics`, `test_plan`, `bash`, `build_command`, `browser_open`, `browser_snapshot`, `browser_console`, `workspace_diff`, `verification_plan`.

`workspace_diff` succeeds using nested `checkpoint.id` (run 1 `ws-036fef7745b6687f8da15a39`, run 2 `ws-8e014735e6883802aefdc8f9`).

Graph Writer executed the same 17 tools through `agent.call` with `pre_tool` -> `NativeExtensionHost::before_tool` and Writer allowlist (`workspace_diff` success). Classifier deny of `verification_plan` and `workspace_restore` was returned as tool errors: `tool "..." is not available to the classifier role`.

Raw `--nocapture` transcripts were captured in the session scratch `p12-normal/` and `p12-graph/` directories (not committed).

## Limitations (not relabeled success)

- `lsp_diagnostics` / `lsp_document_symbols`: dispatched, returned error (language server unavailable in fixture).
- `workspace_diff`: succeeds after the nested `checkpoint.id` fix.
- `startup_ms` / `memory_bytes`: null; working-set sampling was not wired.
- `cache_hit_rate`: null; warm impact was faster than cold but hit/miss counters were not exported.
- Linux/macOS live Chromium 17-step: not executed locally. GitHub native ubuntu/macos `test-impact-native` jobs on this SHA succeeded; they do not re-run the ignored Playwright 17-step.
- Full `davinci-agent` / `davinci-coding-agent` package suites previously exposed Windows ACL, browser timeout, no-HEAD fixture, and process-lifecycle failures. Those were not weakened. Native CI shards and focused tests are the Section 34 evidence.

## Architecture (owners on main)

- P1 `crates/davinci-coding-agent/src/native_extensions/test_impact/`
- P2 `crates/davinci-agent/src/process_manager.rs`
- P3 `crates/davinci-coding-agent/src/interaction_testing/` and `native_extensions/browser.rs`
- P4 `crates/davinci-agent/src/runtime/transactions/`
- P5 `crates/davinci-coding-agent/src/native_extensions/package_intelligence/`
- P6 `crates/davinci-coding-agent/src/native_extensions/git_intelligence/`
- P7 `crates/davinci-coding-agent/src/native_extensions/build_intelligence/`
- P8 `crates/davinci-coding-agent/src/hooks/` and `tests/hook_policy.rs`
- P9 `crates/davinci-coding-agent/src/native_extensions/change_impact/`
- P10 `crates/davinci-coding-agent/src/native_extensions/verification_planner/`
- P11 `crates/davinci-coding-agent/src/native_extensions/workspace_snapshot/`
- P12 `crates/davinci-evals/src/engineering.rs` plus live test `browser_integration_tests::login_button_seventeen_step_normal_dispatch_and_graph_deny`

## Restart

No production service restart. Agents pick up `origin/main` `5585fd6` on next checkout/rebuild.
