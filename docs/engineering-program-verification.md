# DaVinci engineering program — final verification report

Date: 2026-09-19.
Write tree: `C:\Users\sergi\Desktop\pi-rust\final-worktree`.
Shared checkout `C:\Users\sergi\Desktop\pi-rust` was not modified.

## Delivery

- `origin/main`: `70542fcbf93e223aa40889763d37e1a76913f50d` (merged PR #23, final evidence closeout; code acceptance remains `9fe1e106`).
- Merged PRs: #9 P1, #10 P2, #11 P4, #12 P3, #13 P5, #14 P7, #15 P6, #16 P9, #17 P8, #18 P10–P12, #19 P12 live 17-step, #20 program report, #21 Graph/checkpoint acceptance fix; #22 final acceptance/macOS host fix, and #23 final evidence closeout.
- The final evidence closeout is merged to `origin/main` at this SHA; all twelve feature SHAs and the live acceptance fixes are ancestors of it.

## Final main verification on `70542fcb`

- Security SARIF: https://github.com/J12003LPZ/davinci/actions/runs/35462619625 success.
- CI: https://github.com/J12003LPZ/davinci/actions/runs/35462619603 success across the full matrix, including Linux/macOS/Windows native P12 and test-impact jobs.
- Workflow lint: https://github.com/J12003LPZ/davinci/actions/runs/35460353647 success on the unchanged workflow set.

Prior assembled main (P1–P12 without the live 17-step follow-up): `57975a5` CI https://github.com/J12003LPZ/davinci/actions/runs/35418243472 success.

## P12 live 17-step

The focused command passed twice locally. The merged `main` SHA also passed the real Chromium workflow twice per platform (macOS, Linux, and Windows):

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

Artifact-level P12 evidence was captured on the code acceptance SHA `9fe1e106` in CI run [35460353612](https://github.com/J12003LPZ/davinci/actions/runs/35460353612) and preserved in the `p12-live-login-macOS`, `p12-live-login-Linux`, and `p12-live-login-Windows` artifacts. Every platform ran the feature-off and feature-on fixture twice; all six feature-on browser runs succeeded, recorded startup and working-set memory, and reported 3 normal cache hits plus 4 Graph Writer cache hits. Median feature-on startup/memory was 162.475 ms / 50,642,944 bytes on macOS, 337.726 ms / 62,803,968 bytes on Linux, and 364.064 ms / 39,739,392 bytes on Windows. The Graph Writer completed all 17 steps, `workspace_diff` succeeded, and the Classifier denied both `verification_plan` and `workspace_restore` through `NativeExtensionHost::before_tool`. Final main `70542fcb` reran all three hosted platforms successfully in CI [35462619603](https://github.com/J12003LPZ/davinci/actions/runs/35462619603).

## Limitations (not relabeled success)

- `lsp_diagnostics` / `lsp_document_symbols`: dispatched, returned error (language server unavailable in fixture).
- `workspace_diff`: succeeds on every hosted run after the nested `checkpoint.id` fix.
- Linux/macOS live Chromium 17-step: not executed locally; the hosted native P12 jobs passed the real browser path on all three platforms.
- Full `davinci-agent` / `davinci-coding-agent` package suites retain historical Windows ACL, browser timeout, no-HEAD fixture, and process-lifecycle failures. Those paths were not weakened; the required native CI shards passed.
- Fresh local Windows Cargo validation is unavailable in this environment because rustc reports `Access is denied` and the offline registry lacks `cpal`; hosted CI is the release gate.

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

No production service restart. Agents pick up `origin/main` `70542fcb` on next checkout/rebuild.
