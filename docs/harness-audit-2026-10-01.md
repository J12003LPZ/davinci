# DaVinci harness audit — 2026-10-01

**Status: DONE_WITH_CONCERNS.** Seventeen reproducible defects were fixed, regression tests were added or strengthened, and the tested paths pass. This is a completed audit of the tested environments, not a guarantee that the harness has no remaining bugs. The six follow-up gaps have been retested; account-changing flows and installed-binary replacement remain unverified as detailed below.

## Scope and delivery

- Worked in `C:\Users\sergi\Desktop\davinci-main`, version 1.0.71, using Rust 1.83.0.
- The audit remained solo and created no worktree. After explicit user authorization, live graph and learning-reviewer workers ran solely as product tests. The initial audit ran in a source directory without Git metadata, and pre-edit source was backed up before changes. For the subsequently requested PR, a temporary bare Git repository maintains branch `fix/harness-audit-20261001` without altering the source checkout's Git state.
- Every live reasoning-model test used **OpenAI Codex `gpt-6-luna`, effort `low`**. The embedding test used the already installed local **EmbeddingGemma**, because it exercises the vector embedding API rather than a reasoning model.
- Excluded microphone/dictation input, the `davinci-voice` test package, and tests matching `voice` as requested.
- Tests used disposable workspaces/configuration. Original credentials and the installed PATH executable were not changed. Sharing was a dry run; no gist was published.
- Fixed executable: `C:\Users\sergi\Desktop\davinci-main\target\debug\davinci.exe`. The installed `C:\Users\sergi\.cargo\bin\davinci.exe` still contains its previous build.
- Size: large, cross-module audit. Validation: full non-voice workspace suite plus targeted regressions, real integrations and live-model evals. Agents: solo, per user instruction. Review: direct review of source diffs against the backup; no independent reviewer agent.

## Fixed defects

Each row has a failing regression captured before its fix and a passing check afterward. Related symptoms caused by one defect are grouped together.

| # | Defect and observable consequence | Fix and regression evidence |
|---|---|---|
| 1 | Clearing vector memory deleted foreign repository records and allowed legacy memory to reappear on restart. | Atomically persist an empty current-repository store while preserving foreign records. `clear_preserves_foreign_records_and_does_not_restore_legacy_memory`; persisted CLI eval also passes. |
| 2 | A failed memory clear erased the in-memory records even though persistence failed. | Prepare the cleared state separately and publish it only after persistence succeeds. `failed_clear_preserves_in_memory_records`. |
| 3 | Reindexing after an external store deletion reused stale in-memory records. | Reload treats a missing store as empty and propagates other read failures. `reindex_does_not_resurrect_an_externally_deleted_store`. |
| 4 | Repairing three identical memory IDs could generate another duplicate. | Reserve existing and newly generated IDs and deterministically resolve collisions. `reindex_repairs_three_identical_ids_and_is_idempotent`; CLI eval confirms repair, reload and a second no-op reindex. |
| 5 | `/setup` could overwrite an unreadable/non-UTF-8 `.gitignore` with generated content. | Report an actionable read error and preserve the file; successful updates use atomic writes. `apply_preserves_an_unreadable_gitignore` plus byte-for-byte CLI verification. |
| 6 | `/setup` treated an earlier ignore rule as sufficient even when a later negation exposed runtime state. | Evaluate Git ignore semantics, including negations, and append missing rules. `negated_gitignore_patterns_require_a_later_ignore` plus setup/check CLI eval. |
| 7 | Ollama locality detection mishandled IPv6/case and could mistake URL user information for the actual host. | Parse the URL, require HTTP(S), and inspect its host. `local_ollama_url_uses_the_parsed_host` covers IPv6, case, remote suffixes, user information and unsupported schemes. |
| 8 | Starting setup for a different model/server could replace the progress slot of an active pull. | Reserve the shared slot until its owner finishes, including embedding. `another_model_cannot_replace_an_active_pull`. |
| 9 | Runtime turn-context messages suppressed scripted offline tool calls, so fixture runs could miss their intended tools. | Ignore harness context messages when identifying the latest meaningful prompt/result. Both offline tool-call regressions and real child-process fixtures pass. |
| 10 | The graph goal inspector hid active verification progress and the blocked reason. | Render run facts in the goal inspector. `goal_inspector_shows_verification_progress_and_blocked_reason` and ConPTY verification/resume eval pass. |
| 11 | Typing an exact `/model openai-codex/gpt-6-luna` could select a featured fuzzy match such as `gpt-5.6-luna`. | Rank exact full references and exact model IDs first. `exact_model_argument_precedes_featured_fuzzy_matches` and real terminal selection pass. The incorrect selection was reproduced offline. |
| 12 | Responses tool definitions omitted `strict`, allowing provider normalization to make optional arguments required. This affected ranged output retrieval. | Explicitly emit `strict: false` unless strict sampling was requested; preserve the declared parameter schema. `optional_function_arguments_do_not_opt_into_provider_strict_normalization` and live retrieval pass. |
| 13 | A truncated retrieval exposed its continuation cursor only in UI metadata, which the model could not see. | Include callable `retrieve_output` cursor arguments in model-visible content, retaining structured metadata and filter guidance. Pagination and UTF-8 long-line regressions pass; Luna used the cursor and recovered a hidden value on line 452. |
| 14 | Live `/compact` failed on Codex with **“Stream must be set to true.”** | Route final-only Codex completions through the existing streaming transport/decoder. New loopback regression verifies stream mode, low effort, instructions, cache settings, final text and usage. Live compaction persisted a summary and recalled the retained code afterward. |
| 15 | On Unix, `/setup` failed to recognize anchored directory ignores before the directories existed, causing duplicate rules. | Remove the trailing slash from the candidate path while passing the directory flag to the Gitignore matcher. `anchored_state_directories_are_recognized_before_they_exist`; Linux reproduction changed from two failures to two passes, followed by the actual Linux setup suite. |
| 16 | Concurrent tool callbacks interleaved event-log writes, producing invalid JSONL during a live graph. | Serialize each row before appending under a process mutex. `concurrent_tool_events_remain_complete_json_lines` failed before the fix and passed afterward; repeated live graphs produced valid journals and event ledgers. |
| 17 | Windows marketplace clones failed with `Filename too long` in deeply nested staging paths. | Pass `-c core.longpaths=true` only to the plugin Git command. The regression stages a real path longer than 260 characters while preserving a conflicting local Git setting; the public catalog install/update/uninstall eval now passes. |

The strict-schema change follows the documented Responses behavior: omitting `strict` can normalize the schema, whereas explicit `false` preserves optional arguments. See [OpenAI function-calling documentation](https://developers.openai.com/api/docs/guides/function-calling).

Additional validation repairs: the graph child fixture now establishes real worker/session authority and asserts a child was launched; the corrupt-journal test checks that neither prompt is persisted before recovery; Unix-only sandbox helpers are conditionally compiled to eliminate Windows warnings; the ConPTY driver releases terminal handles before deleting its temporary directory. The follow-up also serialized environment-sensitive hook and MCP tests after instrumented runs exposed races between configuration readers and environment writers. These test-isolation repairs are separate from the seventeen defects above.

## Initial audit validation results

The initial audit workspace run included `experimental-ipc` and `interaction-testing`. Counts below describe separate runs; overlapping runs must not be added together as unique test coverage.

| Check | Observed result | Evidence file |
|---|---|---|
| Full Rust workspace, excluding voice | **5,221 passed, 0 failed**, 40 default-ignored, 19 voice-filtered; 144 test/doc-test result groups | `final-workspace.log` |
| Explicit integration/opt-in Rust runs | **22 passed**, including 21 of the 40 default-ignored tests | `extra-results.json`, `optional-local-results.json`, individual logs |
| Whole-workspace Clippy, non-voice, all targets, both optional features | Pass with `-D warnings` | `final-clippy.log` |
| Formatting | `cargo fmt --all -- --check` passed | Tool execution result |
| Fixed debug executable build | Passed | `build-final.log` |
| Node baseline suite | **37 passed**, 0 skipped | `baseline-node.log` |
| Python suite | 149 run: **145 passed**, 4 platform-specific skips | `baseline-python.log` |
| Real Chromium Node tests | **5 unique scenarios passed**; TLS-enabled rerun also passed | `real-browser-node.log`, `real-browser-tls.log` |
| Slash-command terminal eval | **46 checks passed** | `slash-final.log`, `slash-final-screens.json` |
| Native CLI command eval | **30 cases produced expected exit codes**, including intentional invalid-input cases | `cli-results.json` |
| Native intelligence eval | Discovery, semantic/memory/retrieval tools and graph dry run passed; no provider calls or MCP startup | `native-final.log` |
| Memory/setup state eval | Passed with real local embeddings | `state-eval.log` |
| Offline optimization eval | **8/8 correctness gates passed** | `optimization-gate.json` |
| Terminal interaction eval | Passed at widths 40, 80, 109, 120 and 240; delayed paste, explicit submit, graph setup/cancel, verification progress and blocked-run resume | `terminal-final.log` |
| Changed/added Python eval scripts | All eight compiled with `py_compile`, including the two follow-up evals | Tool execution result |

The final-only Codex regression was additionally rerun after bounding its loopback server's accept wait; it passed, as did its targeted Clippy check. The follow-up instrumented workspace rerun below covers the bug fixes. A later behavior-preserving iterator cleanup was followed by its seven targeted tests, refreshed instrumentation, affected-crate Clippy, formatting and a debug rebuild; the full suite was not repeated after that cleanup.

### Live model and integration evidence

| Feature | Result actually exercised |
|---|---|
| Agent read/edit/shell loop | Luna read and edited a disposable fixture from `audit_value=17` to `audit_value=23`; four tool executions completed without errors. |
| Token governor | A 35,094-byte, 900-line output was compressed. Luna retrieved the first page, followed a visible continuation cursor, then searched the full saved output and recovered the hidden line-452 value. Three retrievals succeeded. |
| `/init` | Luna created/updated the disposable project's `AGENTS.md`, preserving its existing sentinel instructions and leaving `CLAUDE.md` unchanged. |
| `/compact` and recall | Luna saved a compaction entry containing `amber-9364`, then returned that code after compaction. Final eval exited successfully, including cleanup. |
| Vector memory | Existing Ollama/EmbeddingGemma produced three 768-dimensional vectors; semantic search recovered the deployment region. Duplicate repair was idempotent; clear preserved foreign records and did not revive legacy records. |
| Graph | Controller, budgets/retry safety, persistence, recovery and transaction fixtures passed. Offline child processes exercised missing-submit rejection, edit provenance and successful submit. Real browser transport tests exercised graph context isolation and authenticated writer dispatch. The follow-up live graph additionally ran classifier, planner, writer and reviewer workers successfully with GPT-6 Luna/low. |
| Browser | Real Chromium exercised supervised lifecycle, restart, concurrent startup, login failure/fix, redirects/subresources, HTTPS/WSS, owned/foreign WebSockets, retained screenshots and bounded JSONL. Native and RPC browser paths passed. |
| Language intelligence | Real TypeScript server exercised definitions, hover, references, symbols, implementations, types, diagnostics and edit synchronization. Follow-up Rust, Pyright and BasedPyright tests and real CLI definition navigation also passed. |
| Processes/repository/test impact | Real Node process reuse, stdin, bounded large output and descendant cleanup passed. Repository indexing/cache/edit reindex and test selection with a planted failing test passed. Graph layout/render/navigation ran with 25/100/500/1,000 nodes. |

The eight offline optimization results measure specific fixtures, not general production savings. They cover output routing, context/schema reduction, deferred graph schemas, retry attempts, memory freshness, incremental scanning and semantic navigation. Exact before/after values are retained in `optimization-gate.json`.

### Command coverage

All 37 advertised built-in command entry points were exercised across the terminal eval, graph eval and live `/init` eval. This does not mean every subcommand combination received a live external-service test.

- Configuration/UI: `/help`, `/hotkeys`, `/settings`, `/config`, `/model`, `/thinking`, `/effort`, `/context`, `/reload`, `/status`, `/cost`, `/doctor`, `/quit`.
- Sessions: `/new`, `/name`, `/tree`, `/rewind`, `/export`, `/import`, `/fork`, `/clone`, `/resume`, `/copy`, `/share`, `/compact`, `/init`. Export/import was checked against the actual JSONL file. Empty-state errors were exercised for rewind/copy and absent sessions. Sharing used the built-in dry run.
- Authority/integrations: `/permissions`, `/plan`, `/act`, `/login`, `/logout`, `/mcp`, `/agents`, `/plugin`, `/tasks`, `/workflow`, `/graph`. Login opened its provider picker; logout targeted a nonexistent provider in disposable config. Agent/task views were empty-state checks. `/workflow list` used its explicit feature flag and verified the unavailable-in-this-session response; it did not run a live workflow.
- Native commands: `/setup` and `/setup check`, `/security-scan --report`, repository/cache/package/build/Git/impact/verification/workspace/LSP status, memory status/search/reindex/clear/page, governor status/reset, graph dry-run/status/view/abort, learning status/pending/invalid approval/rejection, skill list/invalid view, and hook status.

## Important changed files

| Area | Files |
|---|---|
| Memory/setup/governor | `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`, `setup.rs`, `token_governor.rs` |
| Codex tool requests and compaction | `crates/davinci-ai/src/responses_tools.rs`, `stream.rs`; new `crates/davinci-ai/tests/codex_simple_completion.rs` |
| Terminal model/graph UI | `crates/davinci-tui/src/autocomplete.rs`, `crates/davinci-tui/src/davinci/views/graph_inspector.rs` |
| Offline fixtures/build checks | `crates/davinci-coding-agent/src/main.rs`, `main_tests.rs`, `sandbox_config.rs`, `native_extensions/graph/worker.rs` |
| Follow-up fixes/tests | `crates/davinci-coding-agent/src/hooks.rs`, `mcp.rs`, `plugins/marketplace.rs`; new `crates/davinci-agent/tests/sandbox_bubblewrap.rs` |
| Follow-up reusable evals | `scripts/eval-live-graph.py`, `scripts/eval-plugin-marketplace.py` |
| Existing evals extended | `scripts/eval-native-intelligence.py`, `scripts/eval-terminal-input.py` |
| New reusable evals | `scripts/eval-harness-state.py`, `scripts/eval-live-governor.py`, `scripts/eval-slash-commands.py`, `scripts/eval-live-session.py` |

No production dependency was added. Twenty-one existing Rust files, two existing Python scripts and one JavaScript fixture were modified; two Rust integration tests and six Python eval scripts were added, plus this report (33 files total). The behavior-preserving iterator cleanup in `crates/davinci-agent/src/decision/advice.rs` resolves a Clippy warning exposed by a narrower build; its seven existing tests were rerun.

## Follow-up verification of the six reported gaps

1. **The original 19 ignored entries are accounted for.** Seventeen substantive checks were explicitly run and passed: ten diagnostic/baseline/performance checks, four real Rust/Python LSP checks, the live learning reviewer, official SARIF validation, and the manual screen dump. The other two are subprocess fixtures exercised through their parent tests. Their default `#[ignore]` attributes remain intentional. The new Linux bubblewrap test is another opt-in check, and it was explicitly run successfully.
2. **External validation dependencies were provisioned in an isolated test directory.** Rust-analyzer 2026-09-21, BasedPyright 1.40.1, Pyright 1.1.414 and Python jsonschema 4.23.0 were used. All four LSP opt-ins and the official SARIF-schema check passed. The CLI also resolved actual Rust definitions through rust-analyzer and Python definitions through both Python servers. No production dependency was added.
3. **Live product workers are now verified on a concrete fixture.** The reusable graph eval completed in **79.8 seconds** with classifier, planner, writer and reviewer workers, repaired `sum.js`, preserved the acceptance tests, and returned `SUM_OK`. All **17 recorded worker provider responses** identify GPT-6 Luna and low effort. All **194 journal records and 19 event records** parsed successfully. A separate live learning-reviewer smoke test passed with no diagnostics and produced a structured memory candidate. This is a measured sample, not a general production-quality guarantee; automatic promotion was not exercised. No audit work was delegated.
4. **Two real external integration paths now pass.** The production HTTP MCP client initialized the Microsoft Learn endpoint, discovered tools and successfully called `microsoft_docs_search` against public documentation. A real official marketplace download then passed eleven CLI checks covering browse, install, info, disable, enable, update, uninstall and catalog removal in isolated local state. No plugin commands or hooks were executed. Fresh OAuth, logout of real credentials and real gist publication remain untested: the separate authorization question for account/resource changes was not answered, so real credentials and third-party state were preserved.
5. **Platform and measurement evidence now exists.** Real Linux bubblewrap runs passed filesystem, network and environment assertions in both writable and restricted modes. The ephemeral Docker test container needed `SYS_ADMIN` and an unconfined outer seccomp policy so it could create the inner namespaces; default Docker permissions rejected namespace creation. No host policy was changed. These tests validate the production launch plan, not every resource-limit or process-lifecycle guarantee. Actual Linux setup, hook and marketplace regression groups passed (17, 50, 2 tests respectively). The Linux Python suite ran 149 tests: 144 passed, four Windows-only tests skipped, and one unsupported-owner negative test skipped because native supervision was available. macOS CI at the original PR head passed all ten native Seatbelt tests, including the test proving detached-process ownership is unsupported; Auto therefore remains unavailable on macOS. The final Windows instrumented unit/integration suite passed **5,224 tests**, with zero failures, 40 default ignores and 19 voice-filtered tests. Aggregate line coverage is **82.49% (288,009/349,142)**. This is cumulative cargo-llvm-cov instrumentation for the non-voice workspace and includes inline unit tests and integration-test code; it is not a production-only or cross-platform coverage percentage. The coding-agent crate is **76.46%**, so the aggregate does not establish an 80% target for every crate. Release fixtures also passed: large-tool-call decoding took 3.6182 ms with 6,520,044 allocated bytes; context reuse p50 was 2,485 microseconds at 100 turns and 9,140 microseconds at 300 turns (16 samples each). These fixture measurements are not production latency guarantees.
6. **PR delivery is established; installed-binary replacement is still outside this verification.** [PR #79](https://github.com/J12003LPZ/davinci/pull/79), branch `fix/harness-audit-20261001`, contains the audit and follow-up fixes. Its original base is `50cf29d18d7f488be3f5a29ca42bbfea7d9d8579`, checked against all 4,467 original source files. The source directory still has no Git metadata; a separate bare repository delivers the commits without a worktree. The tested debug binary and installed CLI have different SHA-256 hashes, recorded below. The installed CLI was not overwritten.

| Executable | SHA-256 |
|---|---|
| `C:\Users\sergi\Desktop\davinci-main\target\debug\davinci.exe` | `ae43798680dc45d82b7e3757f4b90c6eb5e145f3323ed41624e0b8707cafb299` |
| `C:\Users\sergi\.cargo\bin\davinci.exe` | `6a43606b8eb96f45f902c3533533326276ef4b281a742d130ebff2ef345de17a` |

Validation repairs were driven by observed failures: the first instrumented run exposed a hook environment race; a later run exposed the same problem in MCP configuration tests. Both readers and writers now share their module's test lock, and the final parallel run passed. The synthetic URL-userinfo regression was also expressed without a credential-shaped literal after GitGuardian falsely flagged its original fixture; it never contained a real credential.

Final local checks also passed: affected agent/coding-agent Clippy for all targets with both optional features and `-D warnings`, `cargo fmt --all -- --check`, and a fresh debug CLI build. The Linux-only bubblewrap integration target passed Clippy separately. The full workspace test count precedes only the behavior-preserving advice iterator cleanup; its seven tests passed again afterward and the coverage report was refreshed.

Self-review: **8/10, with concerns**. The tested gates above passed. Fresh account flows, real publication, installed-binary replacement, broad live multi-agent quality and production-only/per-crate coverage remain explicit limits. This report does not claim the harness is free of all bugs.

## CI timing repairs before merge

The Windows coding-agent job at the second PR head exposed two fixture timing failures: Node had not finished starting within the graph regression's 30-second allowance, and the stale-result test waited only three seconds for a hover request. The duplicate PR-event job passed at that same head, confirming the failures were intermittent. The graph fixture now allows 120 seconds for readiness. The stale-result fixture also allows bounded 120-second startup and holds its response until the parent has edited the source and written an explicit release marker, removing its previous 250-millisecond race. Production timeouts and dedicated timeout assertions are unchanged.

The original JavaScript fixture failed an explicit withheld-response check; the repaired fixture passed ten repetitions. The affected graph/language groups passed 58 tests with six intentional ignores. Both previously failing tests passed 20 further repetitions each (40 test executions). Formatting and JavaScript syntax checks passed. Hosted CI remains the acceptance gate for the full matrix; its final status is recorded on PR #79.

A subsequent Windows agent job exposed eight related failures under parallel load. Guarded workspace snapshots used a hardcoded 100-millisecond budget even in tests, unlike the existing ten-second test budget for ordinary snapshots. `verification/workspace.rs` now shares that budget; production remains at 100 milliseconds. A regression with a deliberately slow permission check failed before the fix and passed afterward. Process-manager and foreground fixtures now allow 120 seconds for startup, with four-minute fixture lifetimes so they cannot expire during setup. Dedicated timeout, cancellation and prompt cleanup assertions retain their original limits.

The 53 affected agent tests passed, followed by all **1,348 agent library tests** with zero failures or ignores. Agent Clippy passed for all targets with `-D warnings`, and formatting passed. These CI repairs came after the workspace coverage measurement above; that percentage has not been remeasured for the added regression.

The branch's audit commits are consolidated before merge to remove the historical synthetic URL false positive from the submitted commit history. The previous two-commit tip is preserved in the local bare repository. The fixture still verifies that URL userinfo cannot disguise a remote host as localhost; no real secret was involved and no security check was disabled.

## Evidence location

Artifacts are under:

`C:\Users\sergi\AppData\Local\Temp\davinci-harness-audit-20261001`

- `original/`: pre-edit source backup.
- `diffs/`, `changes.json`: comparison of modified files against the backup.
- `audit-fixes.patch`, `audit-manifest.json`: combined review patch and before/after SHA-256 hashes for all 33 changed or added files, including this report.
- `merge/`: failed hosted-job log, fixture handshake red/green evidence, affected tests, stress repetitions and final delivery/CI records.
- `verified-results.json`: original audit results; the follow-up supersedes its remaining-ignore list.
- `followup/verified-followup.json`, `followup/coverage.json`, `followup/coverage-final.log`: all 19 opt-in dispositions, measured coverage, final test counts and executable hashes.
- `followup/local-results.json`, `followup/live-reviewer.log`, `followup/native-lsp.log`, `followup/native-pyright.log`, `followup/sarif.log`: formerly missing opt-in and real language/schema gates.
- `followup/graph-reusable/`, `followup/marketplace-reusable/`, `followup/remote-mcp-result.json`: real integration evidence.
- `followup/linux-bubblewrap.log`, `followup/linux-followup-checks.log`, `followup/linux-python-provisioned.log`, `followup/ci-macos.log`: platform evidence.
- `followup/hooks-concurrency-red.log`, `followup/setup-linux-red.log`, `followup/marketplace-red.log`: follow-up failing regressions; adjacent green logs contain the successful reruns.
- `pr-base-verification.json`: current upstream base and complete source-baseline comparison for PR delivery.
- `red-*.log`: failing regressions before the fixes; `red-live-compaction-screen.txt` captures the original provider rejection.
- `live-luna-summary.json`, `live-governor-paging-summary.json`, `live-session-summary.json`, `live-init-screen.txt`: live evidence without credentials.
- `slash-final-screens.json`, `terminal-final.log`, and the logs named above: command/UI and suite evidence.

These temporary artifacts can be removed by normal Windows cleanup. The source fixes, regression tests, reusable evals and this report are retained in the project folder.
