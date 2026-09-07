# Harness efficiency and reliability execution checkpoint

Date: 2026-09-05. Parent plan: [implementation plan](2026-09-05-openai-harness-efficiency-reliability.md).

**Status: partial implementation; not release acceptance.** No performance improvement, full plan completion, or live backend validation is claimed. Execution used one agent, RTK, and Headroom. No commits, pushes, dependency installation, live provider calls, or edits to vendor sources were performed.

## Workspace provenance

Starting HEAD: `99c55c07259b9b395bcb8d6ebb21663255df288e` on `main`. The checkout already contained capability-registry, extension, product, and TUI changes plus untracked `plugins/`, the capability registry module, and this plan/spec. Those changes were preserved. A clean HEAD-only checkout would not reproduce the tested tree.

Initial status and relevant diffs were inspected, but initial source hashes were not captured before edits. The hashes below identify this checkpoint's seven edited source files, not an untouched baseline or the complete working tree. T0's full baseline identifier remains incomplete.

## Implemented changes

- `davinci-agent/src/subagent.rs`: restored `tool_class` and `ToolClass` imports after reproducing the unresolved-symbol compilation failure. Preserved the registry-based scoping work and the read-only guard.
- `davinci-evals/src/codex_eval.rs`: excludes failed runs from efficiency pairs while retaining outcome denominators; rejects incomplete/duplicate pairs and zero denominators; preserves the tool-call regression guard. Legacy boolean-only comparisons now produce measurements with promotion disabled. The new verified-report boundary derives success from independent runner observations, requires executed matching commands, successful exit status, complete file inventory, expected changes, and no forbidden/unexpected changes. It requires schema version 1 and exact manifest coverage, with SHA-256 fingerprints binding each observation to its task/oracle definition. Oracle observations are not deserializable as model artifacts. Cached-ratio arithmetic avoids integer overflow.
- `davinci-ai/src/codex_capabilities.rs`: capability resolver uses parsed HTTPS origin and explicit provider/API/auth metadata. Generic OAuth, deceptive hosts, custom ports, proxies, and Azure do not inherit Codex capabilities.
- `davinci-ai/src/provider_retry.rs` and `stream.rs`: HTTP retry waits check cancellation in 25 ms slices; both actual HTTP completion entry points use the controlled helper. Offline injected-wait regression verifies cancellation prevents another request.
- `davinci-agent/src/lib.rs` and `pruning.rs`: text pruning requires stored evidence and allowed `read` access. Projection retains call/outcome metadata and an evidence path. Missing evidence, revoked read permissions, write failure, or unavailable retrieval retains/restores original output. Structured/image content is retained. Estimates use the actual available projection without cloning the transcript. Removed superseded projection helpers. Pruning no longer instructs mutation reexecution.

## Task status and remaining work

| Task | Status | Remaining acceptance work |
| --- | --- | --- |
| T0 | Compiler repair verified | Exact original dirty-tree hashes unavailable; workspace is not wholly green. |
| T1 | Partial | Actual benchmark runner integration; complete usage/total-token and semantic-operation gating through T2; all terminal outcomes at runner boundary. Current oracle/report API is fixture-tested, not an executed benchmark campaign. |
| T2 | Not implemented | Shared task/call/attempt ledger across foreground, compaction, workers, learning, usage provenance, replay dedupe, and reporting. |
| T3 | Partial | Request-profile product boundary and real flag consumers, unknown model support, actual transport seam matrix. Resolver is not yet a production consumer; its changes alone do not alter actual request profiles. |
| T4 | Not implemented | Shared opt-in limits, atomic admission, no-progress control, terminal outcomes, child/resume accounting. |
| T5 | Partial | Shared attempt budget/deadline, structured permanent/transient classification, nested retry and fallback accounting. Cancellation alone is implemented. |
| T6 | Partial | Final request admission/output reserve, bounded compaction, exact shape invalidation, complete evidence lifecycle/integrity and retention. Current evidence map is transient; missing files restore history rather than durable task-level pinning. |
| T7 | Not implemented | Durable checkpoints, uncertain-side-effect recovery, provider-native item fidelity end to end. |
| T8 | Not implemented | Progressive tool discovery and activation in the actual model loop with permission/collision/revocation tests. |
| T9 | Not implemented | Real instruction inclusion and stable wire schemas/cache invalidation measurement. |
| T10 | Not implemented | Measured simple path and component ablations. |
| T11 | Not implemented | Runner executable, deterministic 16-case corpus, independent executed oracles, manifests/reports/campaigns. Live comparisons require separate explicit authorization. |

## Verification evidence

All Cargo commands used `--offline --locked`. Fixture runs used `PI_OFFLINE=1`, `DAVINCI_OFFLINE=1`, and `PI_DISABLE_NETWORK=1`. No benchmark model calls were made.

- Baseline compilation reproduced missing `tool_class` / `ToolClass`; import repair removed the errors.
- Targeted subagent tests: 16 passed.
- Final `cargo test -p davinci-agent -p davinci-ai -p davinci-evals -- --test-threads=1`: **498 passed**, six suites. Includes 306 agent, 175 AI, and 17 eval tests.
- Final `cargo check -p davinci-coding-agent`: passed.
- Final `cargo clippy -p davinci-agent -p davinci-ai -p davinci-evals --all-targets -- -D warnings`: passed. An earlier run found the superseded pruning helpers unused; those were removed and the check rerun successfully.
- `cargo fmt --all -- --check`: passed.
- `git diff --check`: passed, including after the documentation update.
- Red/green assertion failures were observed for failed-fast scoring, missing/duplicate metrics, legacy boolean promotion, capability classification, and unsafe pruning. New oracle and injected-wait APIs initially failed compilation because the APIs were absent, then passed; these are not claimed as behavioral red runs.

The broader workspace run predates the last focused additions. It passed through the product and remaining non-TUI crates but failed three tests in pre-existing dirty TUI files:

1. `davinci::views::chrome::tests::the_composer_grows_with_its_content`
2. `davinci::views::chrome::tests::the_header_carries_path_branch_and_model_when_there_is_room`
3. `davinci::views::transcript::tests::a_user_turn_is_an_echo_with_no_bubble_and_no_timestamp`

TUI result: 581 passed, 3 failed, 1 ignored. Log: `C:\Users\sergi\AppData\Local\rtk\tee\1788653774_cargo_test.log`. These TUI files were not edited during this execution. The user was asked whether to preserve/report the failures or reconcile tests with the current design; no answer was received at this checkpoint, so changes were preserved.

An earlier workspace run failed the graph-learning attribution fixture because `PI_LEARNING_DISABLE_BACKGROUND=1` prevented its fixture reviewer from running. Removing that flag while retaining offline flags made the isolated fixture and the subsequent workspace graph tests pass. This was a test-environment issue; no graph/learning source was changed.

## Checkpoint source SHA-256

```text
253cb1ce89b7e665c181fadb4bcc9cfec8fd2bf49a2b3c5fa17659e17366fccc  crates/davinci-agent/src/lib.rs
112db2b62b875da374d1ae091e895311eb07a290658a26e7074d404afa00a153  crates/davinci-agent/src/pruning.rs
1dba0576d2b654d7563c03e01d1e9c2356cfaac6725d2a61530ba86c1052f89b  crates/davinci-agent/src/subagent.rs
191198d5844db61c99f01d532325c8f601c8f4f3746b458db349b0554c1c7411  crates/davinci-ai/src/codex_capabilities.rs
24a5ebbda4c705d2c50d05cb4597b448c08a651e44e3bf6ef845db6b0ecddaa9  crates/davinci-ai/src/provider_retry.rs
760b5999da7702d43d869336cdfc9769c636159d2ca5773ffdc1873494c77482  crates/davinci-ai/src/stream.rs
b3aaeb7ca30bc02c51e92dadcda520feaebdb37f4e91832c04b8fe99c2bcb9fc  crates/davinci-evals/src/codex_eval.rs
```

## Next execution inputs

Continue with T2's shared telemetry design, inspecting provider usage semantics before implementation: protocol `Usage::from_tokens` currently treats input and cache counters as disjoint billing categories. Do not reinterpret those existing fields as inclusive input and double-count cached subsets. Wire the accounting through actual HTTP and WebSocket sends, product compaction, and worker aggregation before allowing complete efficiency promotion. Then finish T1's total-cost gate and T3's actual request-profile consumers before proceeding to dependent tasks. Preserve the TUI changes unless the user changes scope.
