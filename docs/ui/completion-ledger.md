# Terminal parity and graph recovery execution ledger

Plan: `docs/superpowers/plans/2026-09-20-claude-code-terminal-parity-implementation.md`.
Binding scope: approved parity specification plus the user's 2026-09-21 continuation attachment (whole terminal AND durable graph/worker recovery).

## Active state

- Branch: `J12003LPZ/claude-parity-graph-completion`.
- Worktree: `C:/Users/sergi/Desktop/davinci-parity-completion`.
- Starting commit: `cb7c7f479a2156f29d6f6e8c0c99bec0d9473952`, fetched rebuild branch, clean before recovery.
- Current task: recover checksum-pinned UI source, validate it, publish a draft checkpoint; then audit graph recovery and reference coverage.
- Completion: NOT established. No native visual parity or complete graph recovery claim.

## State inventory, verified 2026-09-21

- PR #25 is open/draft, base main, rebuild head cb7c7f4. PR #26 is open/draft, base main, implementation head 30fdf8ae819d7bba6fb8e05c6a9e438e6ba8cec1. Rebuild includes the two consolidation merges, but does NOT include the later transported UI implementation.
- Authenticated GitHub API reports push/admin permission. No publication blocker established.
- `.github/scripts/terminal_ui_workspace.py` pins six Git blobs and 25 source/doc paths. Retrieved through authenticated `gh api`; validated each corrected transport part, encoded checksum, decoded length, path allowlist and source SHA256 `2ba63eee249ab1846f397779e17453219e8549a6e46578caee0804e505a8d14b`. `git apply --check` passed, then source applied in this worktree. Original shared checkout untouched.
- Recovery evidence and helper: `C:/Users/sergi/AppData/Local/Temp/davinci-completion-evidence/` (`recover_transfer.py`, `recovered-source.patch`, `quality.log`). Evidence is outside the product package.
- CI job 106414519741 on original head fails rustfmt in `terminal_workspace.rs`; it is not proof of a compile failure. Other jobs were still running when inspected.
- Official Anthropic GitHub release metadata lists 2.1.278 native Windows/Linux assets and SHASUMS256. Binary execution, checksum verification and matched captures remain pending.
- Graph worker source still passes `--no-session`; durable worker conversation recovery is missing in this starting state.

## Decisions

- Ruling: preserve the requested rebuild source as the continuation base, rather than main; both existing UI/graph merges are retained intentionally.
- Ruling: the two historical plans describe the same approved outcome. Use the implementation plan as task ordering and the specification/continuation attachment as acceptance. Historical awaiting-approval prose is superseded by explicit execution authorization.
- Ruling: one tracked ledger here survives publication/compaction; do not create a second skill ledger or delete this record on task completion.
- Ruling: no subagents, including final review, per explicit user instruction. Final review must be identified as author review.
- Pre-flight dependencies: reference captures constrain visual tokens/comparison; shell focus constrains all panels and graph input; host submission/control routing constrains background behavior; durable controller checkpoints and worker sessions jointly constrain recovery. Independent Git/reference reads ran in parallel; writes remain sequential.

## Commands and results

Commands use RTK. Rust pinned to 1.83.0; lockfile preserved.

| Command | Result |
| --- | --- |
| `rtk cargo test --offline --locked -p davinci-tui --test terminal_workspace` before recovery | FAIL: 3 passed, 8 failed; graph draft, typing, submission, paste, background status, Ctrl+D/E/O regressions reproduced |
| `rtk proxy python C:/Users/sergi/AppData/Local/Temp/davinci-completion-evidence/recover_transfer.py` | PASS: all pinned digests and `git apply --check` |
| `rtk git apply C:/Users/sergi/AppData/Local/Temp/davinci-completion-evidence/recovered-source.patch` | PASS |
| `rtk cargo fmt -p davinci-tui -p davinci-coding-agent` | PASS |
| `rtk cargo test --offline --locked -p davinci-tui --test terminal_workspace` after recovery | PASS: 11 passed |
| `rtk cargo test --offline --locked -p davinci-tui` | PASS: 697 passed, 2 pre-existing ignored across 5 suites |
| `rtk cargo fmt --all -- --check` | PASS |
| `rtk cargo clippy --offline --locked -p davinci-tui --all-targets -- -D warnings` | PASS |
| `rtk cargo build --offline --locked -p davinci-coding-agent --bin davinci` | PASS: native Windows debug CLI built, no installation |
| Terminal workflow YAML parse and `rtk git diff --check` | PASS |

Windows reference downloaded outside repository using `gh release download v2.1.278 --repo anthropics/claude-code`. Archive SHA256 `8a223953a84861805ca7d00be12dee25590c06533e02f48dd9496f90db709d2e` matches release SHASUMS256; extracted executable reports `2.1.278 (Claude Code)`. No account/configuration or inference was used for version verification. Native captures still pending.

Terminal workflow now tests committed source on Windows/Linux without source-transfer application, formatter mutation or automatic publishing. Historical transfer helper retained for provenance only.

Focused host validation (`PI_OFFLINE=1`): `cargo test --offline --locked -p davinci-coding-agent --bin davinci terminal_rebuild_host_contracts` and `... --bin davinci terminal_config_alias` each passed one test. An initial `--lib terminal_config_alias` selected zero tests and is NOT counted; corrected to the real binary target. A first PowerShell environment-assignment wrapper failed quoting; reran using an explicit Python child environment.

Checkpoint review: recovered source was reviewed by the implementation author, particularly host model/settings identity, graph view launch behavior, focus ownership and graph-editor tests. This is not an independent final review or full host execution acceptance.

## Open gates and next action

1. Finish recovered UI tests and inspect final source diff; commit/push ordinary source, remove active dependence on CI patch application.
2. Obtain genuine 2.1.278 reference, all current surfaces/nested states, full viewport/theme/renderer matrix, cell/style comparison and interaction traces. Candidate snapshots alone cannot pass.
3. Trace actual graph dispatch/artifact/verification/review/checkpoint controls and implement missing durable sessions, receipts, ownership, resume invariants with offline failure injection.
4. Validate host input/background execution, process supervision/isolation/cleanup, no competing writers, and graph stop/resume against disposable fixtures.
5. Run required exact-revision Linux/Windows Rust and native gates; native Windows Terminal/IME/human comparison remains unverified.
6. Publish branch/commit and draft PR with exact results; keep draft until all gates pass. Do not merge, install or restart user's harness or touch real saved graphs.
