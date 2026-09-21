# Terminal parity and graph recovery execution ledger

Plan: `docs/superpowers/plans/2026-09-20-claude-code-terminal-parity-implementation.md`.
Binding scope: approved parity specification plus the user's 2026-09-21 continuation attachment (whole terminal AND durable graph/worker recovery).

## Active state

- Branch: `J12003LPZ/claude-parity-graph-completion`.
- Worktree: `C:/Users/sergi/Desktop/davinci-parity-completion`.
- Starting commit: `cb7c7f479a2156f29d6f6e8c0c99bec0d9473952`, fetched rebuild branch, clean before recovery.
- Current task: durable graph ownership, workspace validation, live controls and continuation, followed by complete reference coverage.
- UI recovery published as `6b6fc73`, draft PR [#29](https://github.com/J12003LPZ/davinci/pull/29). Graph verification recovery committed as `5e8dfcc`.
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

1. UI recovery checkpoint published; graph verification recovery validated locally. Continue controller persistence and recovery repairs.
2. Obtain genuine 2.1.278 reference, all current surfaces/nested states, full viewport/theme/renderer matrix, cell/style comparison and interaction traces. Candidate snapshots alone cannot pass.
3. Trace actual graph dispatch/artifact/verification/review/checkpoint controls and implement missing durable sessions, receipts, ownership, resume invariants with offline failure injection.
4. Validate host input/background execution, process supervision/isolation/cleanup, no competing writers, and graph stop/resume against disposable fixtures.
5. Run required exact-revision Linux/Windows Rust and native gates; native Windows Terminal/IME/human comparison remains unverified.
6. Publish branch/commit and draft PR with exact results; keep draft until all gates pass. Do not merge, install or restart user's harness or touch real saved graphs.

## Graph verification recovery checkpoint

Recovered both graph patches as ordinary source after verifying their pinned Git-object SHA256 values. Reproduced Cargo rejecting `cargo fmt --check --offline`; corrected `cargo --offline fmt --check` exits successfully. Focused verification tests: 21 passed. Entire graph library suite: 371 passed, five ignored native fixtures. Central shell policy suite: 15 passed. Formatting and workflow YAML parse passed. Graph CI now verifies committed source and does not apply or publish patches. Author diff review completed for this recovery.

## Native reference capture progress

Disposable Windows ConPTY at 120x40 reached the real 2.1.278 Settings and model picker. Evidence under the external evidence directory `reference/native-captures`. The startup capture is an API-key approval dialog and the attempted help capture is Rewind; neither is valid welcome/help evidence. Renderer classification, complete navigation traces, Windows Terminal font/IME, other viewports/themes and visual parity remain unverified. Only a disposable fixture profile/project was used; no installed harness was changed.

Controller audit found ignored checkpoint writes and fresh-state reconstruction on resume. These remain open; passing existing graph tests does not establish recovery correctness.

## Controller checkpoint hardening

New failure injection reproduced two worker dispatches despite initial storage failure. The regression now covers initial creation, the pre-dispatch checkpoint, and a save immediately after a worker returns. A latched persistence error stops dispatch/verification, keeps the returned run blocked, reports the stopped state as not saved, and prevents subsequent implicit checkpoint retries. The last durable snapshot remains available.

Separate regression reproduced verification dispatch after the progress callback cancelled execution. Verification now checks cancellation after that callback. Two storage regressions reproduced the multi-rename fallback and premature state publication before companion writes. Storage now flushes the new file, uses one replacement, and publishes self-contained state last. Unix directory sync is included; power-loss behavior and exact-revision Linux execution are not yet validated.

Final local validation: graph suite 375 passed, five native fixtures ignored; host library/test clippy passed with warnings denied; formatting and diff whitespace passed. Existing atomic replacement and native Windows run round-trip tests passed in the graph suite. Author review completed. This milestone does not yet fix ignored artifact/sidecar errors, full resume state, worker conversations, ownership or the complete visual parity matrix.

Checkpoint hardening published as `c26d8d27c4bc0bc2135a0af84272f4baa543897f`. Exact-revision graph/CI workflows started; workflow lint and SARIF interoperability passed, graph and general CI were still running at inspection.

## Sidecar persistence follow-up

Extended failure injection reproduced execution advancing when the artifact directory alone became unwritable. Artifact, replay fingerprint, context packet and mutation writes now participate in the same latched checkpoint operation. Successful worker results are durably written by the controller before success is published. Redundant graph-definition writes were removed; save_run persists the definition. Graph suite: 375 passed, five ignored after this change. Published as `c563f1c75c47bdd3801eb535b015fae8939246d9`; host library/test clippy, formatting and whitespace passed. This prevents ignored sidecar errors but is not yet a versioned multi-file checkpoint or side-effect receipt protocol.

Earlier UI CI at `6b6fc73` failed host library integration on both platforms. Windows evidence identifies two semantic backend definition-location regressions (zero locations instead of one), with 1087 passing and 12 ignored. Linux had 1089 passing, 12 ignored and two failures: `capped_results_use_existing_governor_retrieval` and `graph_worker_processes_share_parent_language_session`. Evidence: `ui-windows-ci.log` and `ui-linux-ci.log` in the external evidence directory.

## Host integration follow-up

The original Windows pair passed in isolation. A new workspace-alias regression failed with zero locations; canonicalizing the target while retaining an unresolved root caused the rejection. Both root and target are now resolved before containment checks, and display paths remain relative. Semantic backend suite: eight passed, one subprocess helper ignored; outside-root rejection still passed.

Graph workflow at `c26d8d2` passed Linux graph, shell policy and lint steps. Its Windows graph step failed `graph_deadline_controller_aborts_run_when_worker_exceeds_deadline`: the 50ms lifetime could expire during durable checkpoint I/O before task creation. The test now deterministically injects a worker deadline outcome with a generous setup budget; actual process timeout remains covered separately. Both `graph_deadline` tests passed locally. That historical workflow was ultimately cancelled by the next push and is not a final exact-revision pass.

Windows full host library after these repairs: 1097 passed, 12 ignored. Linux isolated manager suite: six passed in disposable Rust 1.83 Docker execution. Broader Linux execution passed the two original manager failures but exposed five fixture failures: four security CLI fixtures assumed the default target directory, and the learning-sync fixture wrote to the source checkout. The fixtures now resolve the CLI beside the running test build and store learning data in temporary directories. All five changed fixtures passed on Windows. Linux full rerun: 1099 passed, 12 ignored with a read-only source mount and separate Cargo target volume. Host library/test clippy, formatting and whitespace passed; author diff review completed. The original Linux CI failures were not reproduced locally; their resolution is not claimed. Exact-revision remote gates and all native visual gates remain open.

## Durable tool-result follow-up

A new regression reproduced a completed native file write reopening as an uncertain operation: dispatch saved StartedUnknown but never saved the result. Dispatch now checkpoints success or failure before returning. Storage errors latch in the ledger and stop reservations, dispatch, recovery and waiting followers until the session is reopened and reconciled. Output hooks cannot clear ledger errors, rejected replays or call-ID collisions. Poisoned dispatch locks fail closed. Ledger replacement now flushes and replaces in one rename instead of removing the Windows destination first; Unix directory-sync errors are propagated.

Regression coverage includes actual successful writes and failed reads across reopen, injected post-execution checkpoint failures for both successful and failed tools, no automatic retry after storage repair, follower rejection, and hook protection. Windows and Linux each passed 14 ledger tests and 39 turn-related tests. Windows agent library/test clippy, formatting and whitespace passed. Linux evidence is `linux-tool-ledger.log` in the external evidence directory. The first Linux invocation failed because the temporary shell script used Windows line endings; the corrected LF script ran both suites successfully. Author review completed; no independent agent was used.

Host checkpoint `0bea967` passed graph workflow run `35630027309` on both platforms. General CI was still running at inspection. These results are checkpoint evidence, not final-revision acceptance. Worker conversations, complete graph continuation state, effect-report error handling, controller ownership and native visual parity remain open.

## Graph effect handoff follow-up

Tool-result checkpointing was published as `077eca3`. A subprocess-isolated regression then reproduced an actual mutation reporting success when its graph effect-report path was unwritable. Effect handoff errors now retain the applied transaction and local effect, fail the tool, latch further dispatch and cancel the worker. Output hooks cannot revive that result. Legacy mutation capture errors are also propagated instead of dropping missing pre/post images. Effect report writes flush data and metadata and sync the parent directory on Unix.

Windows validation: 40 turn-related tests, four effect tests, 38 transaction tests passed; downstream host library/test clippy passed with warnings denied. Linux validation: 40 turn-related tests, four effect tests, 18 platform-applicable transaction tests passed. Evidence: `linux-effect-handoff.log`. This validates failure reporting and retention, not complete cross-process reconciliation or task-attempt ownership. Full worker-session continuation and graph cursor work are still required.

## Workspace controller ownership

Effect handoff published as `78875ec`. OS-held exclusive ownership now guards graph launch, resume reconciliation and idle graph mutations. Windows uses an exclusive file handle; Unix uses nonblocking flock. The canonical workspace and fixed `.davinci/graph/controller.lock` namespace prevent aliases or legacy-store migration from creating separate owners. The lock file is retained; closing the handle, including process death, releases ownership. Resume transfers the held guard into execution without an unlocked handoff. Background launch acquires ownership before reporting started or replacing the process-local registry.

Regression tests first reproduced competing owners and a background launch falsely returning started. Both now pass, together with process-kill release, alias exclusion, legacy/current store exclusion and dispatch/mutation refusal. `PI_OFFLINE=1 cargo test --offline --locked -p davinci-coding-agent --lib native_extensions::graph::` passed on Windows and Linux: 379 passed, five existing native fixtures ignored each. The initial suite exposed a fixture blocking storage before lease acquisition; it now blocks the runs directory to retain its intended post-ownership checkpoint-failure coverage. Windows host library/test clippy with warnings denied, all-workspace formatting and diff whitespace passed. Linux evidence: `linux-workspace-lease.log`. Review was by the implementation author.

This guards cooperating controllers, not yet orphaned workers after a controller crash. Snapshot workspace validation, actual live-control routing, durable receipts, worker sessions and execution cursor recovery remain open. Native reference parity and exact-final-revision full gates remain open.

## Checkpoint identity validation

Workspace ownership published as `3cb8d87`. A regression reproduced a checkpoint naming a different workspace being accepted by the loader. Loading now rejects mismatched embedded run IDs, unsupported versions and foreign canonical workspaces before reading companion state. Explicit resume uses the checked loader and explains missing, corrupt or incompatible checkpoint failures. Workspace aliases remain accepted. This closes foreign-workspace writes through loaded state; it does not establish complete checkpoint/topology validation.

Windows graph suite passed 380 tests with five existing ignored fixtures; after adding diagnostic coverage, all 22 storage tests passed. Linux graph suite passed 381 tests with five existing ignored fixtures on the resulting source. Evidence: `linux-checkpoint-identity.log`. Host library/test clippy with warnings denied and formatting/whitespace checks passed. Live control routing, durable receipts, worker conversations and stage recovery remain the next work.

## Durable live controls

Checkpoint identity published as `4717c3e`. Controls now reach the executing controller instead of mutating a UI snapshot. Requests and receipts are saved together before cancellation is signalled or success is reported. Duplicate operation IDs survive reload; changing a request under an existing ID conflicts. Node controls require the current attempt. Pause waits for worker result persistence and also gates each verification command. Stop-node cancels the selected attempt without automatic retry, and stop-graph cancels execution. Persistence failure latches the controller stopped without claiming the failed receipt was saved. Superseded pending pauses receive a terminal rejection instead of remaining accepted forever.

New regressions reproduced false live pause acknowledgement, continued dispatch, stale/unknown node acceptance, finished-run pause acknowledgement and a superseded pause remaining accepted. Windows and Linux graph suites each passed 388 tests, with five existing native fixtures ignored. Windows live-control tests passed again after consolidating the dispatch gate. Host library/test clippy with warnings denied passed after correcting two binary-test constructors for the new history field. The Windows RPC regression passed, including refusal to acknowledge an idle resume or cancellation of an interrupted worker without an executing controller. Formatting and diff whitespace passed. Linux graph/RPC final follow-up is running; evidence is stored outside the repository as `linux-live-controls.log`. Review is by the implementation author; no subagents were used.

Linux graph/RPC follow-up completed successfully: 388 graph tests passed (five existing native fixtures ignored), and the RPC regression passed. Durable live controls were published as `824f0c0`.

This is an intermediate control milestone. Live arbitrary-node retry is explicitly rejected until durable cursor rewind is implemented. Reopening an idle run still uses `/graph-resume`; graph-control does not pretend a controller was started. Complete continuation state, task-attempt conversations, orphan worker ownership and the full native UI reference matrix remain required. Next: preserve execution cursor, attempts, original baselines and completed stages across resume.

## Generated graph continuation

Generated runs now retain the execution stage, stable task identities, original and attempt mutation baselines, completed milestone count, planning/revision state and input identities. Resume retains the complete run and cumulative spending; completed worker artifacts are restored without dispatch. Missing verification commands block without consuming revision cycles. Source or verification-input changes invalidate prior review/finalization evidence without replaying the completed writer. Fresh security checks receive distinct task identities. Failed research siblings resume independently. Checkpoint journals are excluded from source baselines.

Failure injection first reproduced lost milestones, repeated writers, verification repair spending revision cycles, skipped failed research, stale review reuse, duplicate security identities and corrupt milestone cursors being accepted. Regressions now cover repeated interruption after every generated stage across two milestones, on-disk reload, retained mutation diffs, verification repair, changed source before review and finalization, security-enabled review, and corrupt cursor refusal without overwriting durable state.

A Linux graph run also exposed lease ownership persisting briefly after the owner dropped its handle. A deterministic duplicate-descriptor regression reproduced the Unix open-file-description lifetime behavior. Explicit unlock on guard release fixes this; process-death release and exclusive ownership remain covered. The first Linux compile of the fix caught an unavailable logging dependency; the diagnostic now uses the standard library.

Windows graph suite passed 394 tests; Linux passed 395 including the Unix descriptor regression. Each had five existing native fixtures ignored. The RPC regression passed on both platforms. Host library/test clippy with warnings denied passed on Windows and Linux; formatting and diff whitespace passed. Linux evidence is `linux-generated-continuation.log` and `linux-generated-continuation-clippy.log` in the external evidence directory. An initial concurrent Linux clippy invocation was terminated when the test container exited; the separate invocation passed. Author review only; no subagents. Saved/custom continuation, persistent private worker sessions, complete attempt history, orphan reconciliation, compatible legacy migration and the full native UI matrix remain required.
