# DaVinci audit triage: WOR-175–294

> **Historical implementation notes and independent verification.** The original assignments and implementation notes below describe intermediate commits. PR #163 also merged the Opus-assigned fixes. The independent verification section records acceptance evidence and remaining gaps for the current main branch.

Scope: 120 Linear issues added after the initial WOR-20–174 audit, organized into the eight issue areas agreed for DaVinci.

Proposed model routing: **Opus 5.5: 44** for complex architecture, concurrency, recovery, and security; **Sonnet 5.5: 76** for comparatively localized fixes and bounded regressions. This is a cost/complexity recommendation, not a capability boundary.

## Priority and execution criteria

1. Triage security, data loss, credential exposure, and silent execution/corruption ahead of UI and feature completeness. In particular investigate [WOR-215](https://linear.app/work-lpz/issue/WOR-215) (Git options injection), [WOR-229](https://linear.app/work-lpz/issue/WOR-229) (malformed executable tool arguments), [WOR-235](https://linear.app/work-lpz/issue/WOR-235) (proxy credentials), [WOR-175](https://linear.app/work-lpz/issue/WOR-175) (concurrent session corruption), [WOR-269](https://linear.app/work-lpz/issue/WOR-269) (unrelated file deletion), and [WOR-278](https://linear.app/work-lpz/issue/WOR-278) (secret exposure).
2. Check whether each finding reproduces on current `main`; many Linear descriptions are source-review hypotheses on earlier commits. Mark already-fixed or duplicate findings explicitly.
3. Group changes by shared invariant but keep implementation PRs reviewable. Tests must cover the reported failure and a non-regression case.
4. Run relevant Rust workspace tests and checks for each implementation PR; record actual results, rather than claiming the checklist below is complete.

## Issue areas

### Sessions and streaming (WOR-175–WOR-191)

**Focus:** Preserve durable session integrity and branch ancestry; bound provider-stream memory and verify safe crash recovery.

**Opus 5.5 (7):** [WOR-175](https://linear.app/work-lpz/issue/WOR-175), [WOR-176](https://linear.app/work-lpz/issue/WOR-176), [WOR-177](https://linear.app/work-lpz/issue/WOR-177), [WOR-180](https://linear.app/work-lpz/issue/WOR-180), [WOR-182](https://linear.app/work-lpz/issue/WOR-182), [WOR-183](https://linear.app/work-lpz/issue/WOR-183), [WOR-184](https://linear.app/work-lpz/issue/WOR-184)

**Sonnet 5.5 (10):** [WOR-178](https://linear.app/work-lpz/issue/WOR-178), [WOR-179](https://linear.app/work-lpz/issue/WOR-179), [WOR-181](https://linear.app/work-lpz/issue/WOR-181), [WOR-185](https://linear.app/work-lpz/issue/WOR-185), [WOR-186](https://linear.app/work-lpz/issue/WOR-186), [WOR-187](https://linear.app/work-lpz/issue/WOR-187), [WOR-188](https://linear.app/work-lpz/issue/WOR-188), [WOR-189](https://linear.app/work-lpz/issue/WOR-189), [WOR-190](https://linear.app/work-lpz/issue/WOR-190), [WOR-191](https://linear.app/work-lpz/issue/WOR-191)

### Code Mode (WOR-192–WOR-200)

**Focus:** Preserve complete child evidence, deterministic identity, bounded callbacks, and honest result completeness.

**Opus 5.5 (7):** [WOR-192](https://linear.app/work-lpz/issue/WOR-192), [WOR-193](https://linear.app/work-lpz/issue/WOR-193), [WOR-194](https://linear.app/work-lpz/issue/WOR-194), [WOR-195](https://linear.app/work-lpz/issue/WOR-195), [WOR-197](https://linear.app/work-lpz/issue/WOR-197), [WOR-198](https://linear.app/work-lpz/issue/WOR-198), [WOR-199](https://linear.app/work-lpz/issue/WOR-199)

**Sonnet 5.5 (2):** [WOR-196](https://linear.app/work-lpz/issue/WOR-196), [WOR-200](https://linear.app/work-lpz/issue/WOR-200)

### Learning and packages (WOR-201–WOR-214)

**Focus:** Make learning mutations recoverable and authorized; represent multiple package versions correctly.

**Opus 5.5 (4):** [WOR-201](https://linear.app/work-lpz/issue/WOR-201), [WOR-204](https://linear.app/work-lpz/issue/WOR-204), [WOR-206](https://linear.app/work-lpz/issue/WOR-206), [WOR-214](https://linear.app/work-lpz/issue/WOR-214)

**Sonnet 5.5 (10):** [WOR-202](https://linear.app/work-lpz/issue/WOR-202), [WOR-203](https://linear.app/work-lpz/issue/WOR-203), [WOR-205](https://linear.app/work-lpz/issue/WOR-205), [WOR-207](https://linear.app/work-lpz/issue/WOR-207), [WOR-208](https://linear.app/work-lpz/issue/WOR-208), [WOR-209](https://linear.app/work-lpz/issue/WOR-209), [WOR-210](https://linear.app/work-lpz/issue/WOR-210), [WOR-211](https://linear.app/work-lpz/issue/WOR-211), [WOR-212](https://linear.app/work-lpz/issue/WOR-212), [WOR-213](https://linear.app/work-lpz/issue/WOR-213)

### Git, LSP and tool safety (WOR-215–WOR-229)

**Focus:** Close Git argument-injection and malformed-tool-call execution boundaries; preserve cancellation and symbol identity.

**Opus 5.5 (4):** [WOR-215](https://linear.app/work-lpz/issue/WOR-215), [WOR-217](https://linear.app/work-lpz/issue/WOR-217), [WOR-225](https://linear.app/work-lpz/issue/WOR-225), [WOR-229](https://linear.app/work-lpz/issue/WOR-229)

**Sonnet 5.5 (11):** [WOR-216](https://linear.app/work-lpz/issue/WOR-216), [WOR-218](https://linear.app/work-lpz/issue/WOR-218), [WOR-219](https://linear.app/work-lpz/issue/WOR-219), [WOR-220](https://linear.app/work-lpz/issue/WOR-220), [WOR-221](https://linear.app/work-lpz/issue/WOR-221), [WOR-222](https://linear.app/work-lpz/issue/WOR-222), [WOR-223](https://linear.app/work-lpz/issue/WOR-223), [WOR-224](https://linear.app/work-lpz/issue/WOR-224), [WOR-226](https://linear.app/work-lpz/issue/WOR-226), [WOR-227](https://linear.app/work-lpz/issue/WOR-227), [WOR-228](https://linear.app/work-lpz/issue/WOR-228)

### AI providers and authentication (WOR-230–WOR-243)

**Focus:** Correct protocol-specific authentication and serialization; avoid credential leakage and dropping images.

**Opus 5.5 (6):** [WOR-235](https://linear.app/work-lpz/issue/WOR-235), [WOR-237](https://linear.app/work-lpz/issue/WOR-237), [WOR-239](https://linear.app/work-lpz/issue/WOR-239), [WOR-240](https://linear.app/work-lpz/issue/WOR-240), [WOR-242](https://linear.app/work-lpz/issue/WOR-242), [WOR-243](https://linear.app/work-lpz/issue/WOR-243)

**Sonnet 5.5 (8):** [WOR-230](https://linear.app/work-lpz/issue/WOR-230), [WOR-231](https://linear.app/work-lpz/issue/WOR-231), [WOR-232](https://linear.app/work-lpz/issue/WOR-232), [WOR-233](https://linear.app/work-lpz/issue/WOR-233), [WOR-234](https://linear.app/work-lpz/issue/WOR-234), [WOR-236](https://linear.app/work-lpz/issue/WOR-236), [WOR-238](https://linear.app/work-lpz/issue/WOR-238), [WOR-241](https://linear.app/work-lpz/issue/WOR-241)

### Build and repository analysis (WOR-244–WOR-258)

**Focus:** Maintain repository/workspace attribution, resource bounds and valid build graph relationships.

**Opus 5.5 (3):** [WOR-246](https://linear.app/work-lpz/issue/WOR-246), [WOR-256](https://linear.app/work-lpz/issue/WOR-256), [WOR-258](https://linear.app/work-lpz/issue/WOR-258)

**Sonnet 5.5 (12):** [WOR-244](https://linear.app/work-lpz/issue/WOR-244), [WOR-245](https://linear.app/work-lpz/issue/WOR-245), [WOR-247](https://linear.app/work-lpz/issue/WOR-247), [WOR-248](https://linear.app/work-lpz/issue/WOR-248), [WOR-249](https://linear.app/work-lpz/issue/WOR-249), [WOR-250](https://linear.app/work-lpz/issue/WOR-250), [WOR-251](https://linear.app/work-lpz/issue/WOR-251), [WOR-252](https://linear.app/work-lpz/issue/WOR-252), [WOR-253](https://linear.app/work-lpz/issue/WOR-253), [WOR-254](https://linear.app/work-lpz/issue/WOR-254), [WOR-255](https://linear.app/work-lpz/issue/WOR-255), [WOR-257](https://linear.app/work-lpz/issue/WOR-257)

### Change Impact and workspace recovery (WOR-259–WOR-270)

**Focus:** Invalidate stale source-derived impact reports; enforce skill permissions and transactional restore safety.

**Opus 5.5 (5):** [WOR-259](https://linear.app/work-lpz/issue/WOR-259), [WOR-265](https://linear.app/work-lpz/issue/WOR-265), [WOR-266](https://linear.app/work-lpz/issue/WOR-266), [WOR-268](https://linear.app/work-lpz/issue/WOR-268), [WOR-269](https://linear.app/work-lpz/issue/WOR-269)

**Sonnet 5.5 (7):** [WOR-260](https://linear.app/work-lpz/issue/WOR-260), [WOR-261](https://linear.app/work-lpz/issue/WOR-261), [WOR-262](https://linear.app/work-lpz/issue/WOR-262), [WOR-263](https://linear.app/work-lpz/issue/WOR-263), [WOR-264](https://linear.app/work-lpz/issue/WOR-264), [WOR-267](https://linear.app/work-lpz/issue/WOR-267), [WOR-270](https://linear.app/work-lpz/issue/WOR-270)

### TUI, plugins and repository graph (WOR-271–WOR-294)

**Focus:** Confine plugin paths, avoid concurrent lost updates, correctly resolve repositories and mark file content untrusted.

**Opus 5.5 (8):** [WOR-283](https://linear.app/work-lpz/issue/WOR-283), [WOR-284](https://linear.app/work-lpz/issue/WOR-284), [WOR-285](https://linear.app/work-lpz/issue/WOR-285), [WOR-289](https://linear.app/work-lpz/issue/WOR-289), [WOR-290](https://linear.app/work-lpz/issue/WOR-290), [WOR-291](https://linear.app/work-lpz/issue/WOR-291), [WOR-292](https://linear.app/work-lpz/issue/WOR-292), [WOR-294](https://linear.app/work-lpz/issue/WOR-294)

**Sonnet 5.5 (16):** [WOR-271](https://linear.app/work-lpz/issue/WOR-271), [WOR-272](https://linear.app/work-lpz/issue/WOR-272), [WOR-273](https://linear.app/work-lpz/issue/WOR-273), [WOR-274](https://linear.app/work-lpz/issue/WOR-274), [WOR-275](https://linear.app/work-lpz/issue/WOR-275), [WOR-276](https://linear.app/work-lpz/issue/WOR-276), [WOR-277](https://linear.app/work-lpz/issue/WOR-277), [WOR-278](https://linear.app/work-lpz/issue/WOR-278), [WOR-279](https://linear.app/work-lpz/issue/WOR-279), [WOR-280](https://linear.app/work-lpz/issue/WOR-280), [WOR-281](https://linear.app/work-lpz/issue/WOR-281), [WOR-282](https://linear.app/work-lpz/issue/WOR-282), [WOR-286](https://linear.app/work-lpz/issue/WOR-286), [WOR-287](https://linear.app/work-lpz/issue/WOR-287), [WOR-288](https://linear.app/work-lpz/issue/WOR-288), [WOR-293](https://linear.app/work-lpz/issue/WOR-293)

## Completion gates for subsequent implementation PRs

- [ ] Each referenced issue is revalidated against the implementation branch and deduplicated as appropriate.
- [ ] Fixes include deterministic regressions for the actual failure path and preserved authorization/recovery invariants.
- [ ] Security, cross-platform and data-loss paths receive adversarial/negative tests.
- [ ] Relevant targeted tests, formatting and checks are reported with exact results.
- [ ] The implementing PR descriptions identify which WOR issues were actually resolved.

## Implementation status: Sonnet 5.5 set (76 issues)

At this intermediate implementation stage, the Sonnet-assigned issues had source fixes with regression tests and the 44 Opus-assigned issues were still untouched. PR #163 subsequently merged both sets. The original implementation used each Linear description and the current code; it did not first reproduce every finding on an earlier commit.

| Area | Fixed | Regression coverage |
| --- | --- | --- |
| Sessions and streaming | WOR-178, 179, 181, 185–191 | `davinci-session` (`wor178_181_session_recovery`, `wor179_tree_depth`); `davinci-ai` handshake, OAuth callback and SSE tests |
| Code Mode | WOR-196, 200 | Unit test for the budget helper; native tests in `codemode_supervisor_budget` are `#[ignore]` (need the admitted Node fixture) and were compiled but not run |
| Learning and packages | WOR-202, 203, 205, 207–213 (and WOR-267, listed under Change Impact in the triage) | `learning::store`, `learning::skill_manager`, `package_intelligence_audit` |
| Git, LSP and tool safety | WOR-216, 218–224, 226–228 | `git_intelligence_audit`, LSP client/session tests, `davinci-ai` parser tests |
| AI providers and authentication | WOR-230–234, 236, 238, 241 | `davinci-ai` serializer, auth and model-config tests |
| Build and repository analysis | WOR-244, 245, 247–255, 257 | `build_intelligence_audit`; WOR-257 has a unit test for the cancel budget only |
| Change Impact and workspace recovery | WOR-260–264, 267, 270 | `change_impact_audit`; skill manager and TUI word-wrap tests |
| TUI, plugins and CLI | WOR-271–282, 286–288, 293 | `davinci-tui` unit tests, `test_impact::analysis`, `file_processor` |

Historical gaps: WOR-286 (plugin uninstall cleanup warning) and WOR-280/281 (launch and clipboard failure reporting) had no failure-path test. The Unix-specific repository symlink test now has a platform guard; its Windows compilation limitation no longer applies.

## Independent verification of the 128 pending issues

Scope: the user's 128-issue closure list, including the 126 fixes merged in PRs #161 and #163, plus WOR-33 and WOR-37 in open PR #165. Verification starts from main commit `67d2c303af70d379e265aea5a77748fe62ce5a15` in an isolated worktree. No installed binary is replaced. Tests use disposable local fixtures; the already completed subscription probe used `gpt-6-luna` at medium effort.

Disposition: **125 merged fixes verified**, **one remaining merged-fix gap repaired in this follow-up (WOR-208)**, and **two verified fixes pending merge in PR #165 (WOR-33/WOR-37)**. No PR is merged by this verification task.

### Checks and observed results

| Check | Result |
| --- | --- |
| `cargo test --locked --workspace --quiet` | Passed: 6,138 reported passes across 218 test summaries; 55 ignored. This aggregate includes fixture subprocess summaries. |
| `cargo test --locked -p davinci-coding-agent --lib -- --nocapture` | 1,493 passed, 18 ignored, including the added learning, browser, map and cleanup checks. |
| `cargo test --locked -p davinci-coding-agent --test package_intelligence_audit --test package_intelligence -- --nocapture` | 19 passed, including both new WOR-208 failures reproduced before the repair. |
| `cargo test --locked -p davinci-tui --test launcher_failures -- --nocapture` | Four harness cases passed; three exercise isolated production browser/clipboard children. Invalid disposable executables prevent desktop side effects. |
| `cargo test --locked -p davinci-coding-agent --lib plugins::command::tests::uninstall_reports -- --nocapture` | Two passed, including a Windows directory containing a file held without delete sharing. |
| `cargo test --locked -p davinci-coding-agent --test codemode_supervisor --test codemode_supervisor_budget -- --ignored --nocapture --test-threads=1` | Ten passed using the admitted Node 24.21.0 and validated disposable bundle; no model calls. |
| `cargo fmt --all -- --check`, `git diff --check`, `cargo clippy --locked --workspace --all-targets -- -D warnings` | All passed on the final code. An initial Clippy failure for test-module placement was corrected and the full check rerun. |

The first broad run stopped at 12 MCP fixture lookup failures because the unit helper assumes a repository-local `target/`, while this audit reuses a separate Cargo target directory. Supplying the actual built `CARGO_BIN_EXE_mcp-fixture` path resolved these failures; the complete rerun passed. The ordinary suite's eight native Code Mode cases and two budget cases remain ignored unless explicitly provisioned. Both were run explicitly here, and CI now runs the budget target with its existing admitted fixture.

The original truncation regression requested an unsupported eight-byte limit and therefore returned `INVALID_INPUT` before script execution. It now uses the supported 4,096-byte limit and an 8,192-byte result, requiring completed script execution, incomplete output and `Partial` status. The machine's default Node 24.19.0 was correctly refused; the admitted installed Node 24.21.0 was used without replacing either runtime.

WOR-208's merged stamp covered only top-level `types`/`typings` and `index.d.ts`. New same-length declaration edits reproduced stale cached symbols for conditional `exports.types` and for a separate `@types` package. The repair hashes the declaration selected by the existing resolver, after canonical repository confinement, so both results refresh with an unchanged manifest and lockfile. The issue stays open until this follow-up merges.

WOR-258's generated 10,000-module / 9,999-edge chain matched an independent reference for every connection count. In one local debug run, the current repository map took **162 ms**, while the former per-file degree scan alone took **4,404 ms**. These are fixture observations, not a portable latency guarantee.

### CI and ignored-test review

All six workflows on main revision `67d2c303` completed successfully: [CI](https://github.com/J12003LPZ/davinci/actions/runs/38005510307), [language intelligence](https://github.com/J12003LPZ/davinci/actions/runs/38005510316), [workflow lint](https://github.com/J12003LPZ/davinci/actions/runs/38005510370), [dependency audit](https://github.com/J12003LPZ/davinci/actions/runs/38005510331), [SARIF](https://github.com/J12003LPZ/davinci/actions/runs/38005510324), and [runtime recovery](https://github.com/J12003LPZ/davinci/actions/runs/38005510356). The historical PR #163 rollup has 103 passes, three skips and a GitGuardian failure on its PR head. That external check remains an unresolved historical result; green main workflows do not override it.

The live language-server matrix and optional mutation fuzz job were skipped on main and provide no acceptance evidence. The affected LSP cancellation, delivery and registration regressions ran in the deterministic local/server-fixture suite. Full named Linux CI logs were reviewed for the existing acceptance tests, including plugin file-symlink confinement that cannot run on this Windows account. Directory confinement ran locally with Windows junctions. The native Test Impact CI matrix passed on Ubuntu, Windows and macOS. Other ignored suites require real servers, browser installations, explicit performance probes or separate live-provider authorization; their omission does not replace the named acceptance checks below.

WOR-33/WOR-37 retain the evidence and limits in [the Codex backend probe on PR #165](https://github.com/J12003LPZ/davinci/blob/c5accfb616e3befbc6bc8cbe5b080d74617b71df/docs/cache/codex-backend-probe.md) and PR #165. Its checks now report 100 passes and two skips. The successful subscription Auto → Plan → Auto run used `gpt-6-luna`, medium effort, stable keys/prefix/40 tool schemas, persisted native replay, and raw reads **4,608 → 4,608 → 3,584**. WOR-37 has a real loopback HTTP decoder → agent → cache-status test for unknown versus explicit zero. This follow-up made **zero additional model calls** and introduced no API-key fallback.

### Issue-by-issue acceptance evidence

Each row names a primary regression; related positive and negative cases run in the same target or the workspace suite. A verified merged fix is eligible for Done. A pending merge remains In Progress.

| Issue | Primary regression | Disposition |
| --- | --- | --- |
| [WOR-33](https://linear.app/work-lpz/issue/WOR-33) | [PR #165](https://github.com/J12003LPZ/davinci/pull/165) | Verified in PR #165; pending merge |
| [WOR-37](https://linear.app/work-lpz/issue/WOR-37) | [PR #165](https://github.com/J12003LPZ/davinci/pull/165) | Verified in PR #165; pending merge |
| [WOR-165](https://linear.app/work-lpz/issue/WOR-165) | [audit_finished_child_registrations_remain_bounded](../crates/davinci-agent/src/runtime/cancellation.rs) | Verified merged fix |
| [WOR-166](https://linear.app/work-lpz/issue/WOR-166) | [audit_resources_only_server_connects_without_tools_capability](../crates/davinci-mcp/tests/audit_capabilities.rs) | Verified merged fix |
| [WOR-167](https://linear.app/work-lpz/issue/WOR-167) | [audit_json_batch_preserves_correlated_response](../crates/davinci-mcp/tests/audit_capabilities.rs) | Verified merged fix |
| [WOR-170](https://linear.app/work-lpz/issue/WOR-170) | [supervised_mcp_rejects_every_malformed_envelope_but_keeps_null_results](../crates/davinci-agent/src/mcp/sandboxed.rs) | Verified merged fix |
| [WOR-173](https://linear.app/work-lpz/issue/WOR-173) | [cached_branch_matches_the_path_for_every_leaf_of_a_chain](../crates/davinci-session-sqlite/tests/audit_cached_branch.rs) | Verified merged fix |
| [WOR-174](https://linear.app/work-lpz/issue/WOR-174) | [audit_jsonl_repo_reopens_after_clearing_a_name](../crates/davinci-session/tests/audit_fact_clear.rs) | Verified merged fix |
| [WOR-175](https://linear.app/work-lpz/issue/WOR-175) | [stale_stored_handle_cannot_corrupt_shared_sequence](../crates/davinci-session/tests/session_integrity.rs) | Verified merged fix |
| [WOR-176](https://linear.app/work-lpz/issue/WOR-176) | [fork_omits_abandoned_branch_messages](../crates/davinci-session/tests/session_integrity.rs) | Verified merged fix |
| [WOR-177](https://linear.app/work-lpz/issue/WOR-177) | [fork_retains_compaction_first_kept_reference](../crates/davinci-session/tests/session_integrity.rs) | Verified merged fix |
| [WOR-178](https://linear.app/work-lpz/issue/WOR-178) | [unsupported_version_header_with_nested_role_is_not_migrated](../crates/davinci-session/tests/wor178_181_session_recovery.rs) | Verified merged fix |
| [WOR-179](https://linear.app/work-lpz/issue/WOR-179) | [deep_linear_history_builds_without_overflowing_the_stack](../crates/davinci-session/tests/wor179_tree_depth.rs) | Verified merged fix |
| [WOR-180](https://linear.app/work-lpz/issue/WOR-180) | [replay_rejects_nonconsecutive_lane_and_name_sequences](../crates/davinci-session/tests/session_integrity.rs) | Verified merged fix |
| [WOR-181](https://linear.app/work-lpz/issue/WOR-181) | [stored_session_recovers_a_partial_utf8_tail](../crates/davinci-session/tests/wor178_181_session_recovery.rs) | Verified merged fix |
| [WOR-182](https://linear.app/work-lpz/issue/WOR-182) | [legacy_reader_preserves_lane_sequence_before_append](../crates/davinci-session/tests/session_integrity.rs) | Verified merged fix |
| [WOR-183](https://linear.app/work-lpz/issue/WOR-183) | [replay_exposes_multiple_open_operations_for_recovery](../crates/davinci-session/tests/session_integrity.rs) | Verified merged fix |
| [WOR-184](https://linear.app/work-lpz/issue/WOR-184) | [stream_event_history_does_not_retain_cumulative_snapshots](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-185](https://linear.app/work-lpz/issue/WOR-185) | [a_malformed_sse_frame_fails_the_turn_even_with_a_terminal_event](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-186](https://linear.app/work-lpz/issue/WOR-186) | [malformed_ok_body_is_an_error_not_assistant_text](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-187](https://linear.app/work-lpz/issue/WOR-187) | [handshake_rejects_missing_accept_header](../crates/davinci-ai/src/codex_ws.rs) | Verified merged fix |
| [WOR-188](https://linear.app/work-lpz/issue/WOR-188) | [fragmented_callback_request_is_reassembled](../crates/davinci-ai/src/oauth_callback.rs) | Verified merged fix |
| [WOR-189](https://linear.app/work-lpz/issue/WOR-189) | [head_request_does_not_consume_the_callback](../crates/davinci-ai/src/oauth_callback.rs) | Verified merged fix |
| [WOR-190](https://linear.app/work-lpz/issue/WOR-190) | [handshake_rejects_missing_upgrade_or_connection_headers](../crates/davinci-ai/src/codex_ws.rs) | Verified merged fix |
| [WOR-191](https://linear.app/work-lpz/issue/WOR-191) | [handshake_rejects_malformed_status_codes](../crates/davinci-ai/src/codex_ws.rs) | Verified merged fix |
| [WOR-192](https://linear.app/work-lpz/issue/WOR-192) | [truncated_and_image_reads_are_not_complete](../crates/davinci-agent/tests/codemode_projection.rs) | Verified merged fix |
| [WOR-193](https://linear.app/work-lpz/issue/WOR-193) | [clearing_codemode_unregisters_it_and_allows_reenable_on_the_same_runtime](../crates/davinci-agent/tests/common/codemode.rs) | Verified merged fix |
| [WOR-194](https://linear.app/work-lpz/issue/WOR-194) | [child_ordinals_follow_request_ids_not_arrival_order](../crates/davinci-agent/tests/common/codemode.rs) | Verified merged fix |
| [WOR-195](https://linear.app/work-lpz/issue/WOR-195) | [queued_codemode_child_cancels_before_active_child_returns](../crates/davinci-agent/tests/common/codemode.rs) | Verified merged fix |
| [WOR-196](https://linear.app/work-lpz/issue/WOR-196) | [native_truncated_output_is_partial_not_completed](../crates/davinci-coding-agent/tests/codemode_supervisor_budget.rs) | Verified merged fix |
| [WOR-197](https://linear.app/work-lpz/issue/WOR-197) | [host_catalog_bootstrap_does_not_spend_guest_metadata_budget](../crates/davinci-agent/tests/common/codemode.rs) | Verified merged fix |
| [WOR-198](https://linear.app/work-lpz/issue/WOR-198) | [a_panicking_callback_still_answers_its_request](../crates/davinci-coding-agent/src/codemode_host/supervisor.rs) | Verified merged fix |
| [WOR-199](https://linear.app/work-lpz/issue/WOR-199) | [a_stuck_callback_returns_its_run_slot_when_the_run_ends](../crates/davinci-coding-agent/src/codemode_host/supervisor.rs) | Verified merged fix |
| [WOR-200](https://linear.app/work-lpz/issue/WOR-200) | [native_slow_catalog_discovery_counts_against_the_timeout](../crates/davinci-coding-agent/tests/codemode_supervisor_budget.rs) | Verified merged fix |
| [WOR-201](https://linear.app/work-lpz/issue/WOR-201) | [failed_append_leaves_in_memory_state_unchanged](../crates/davinci-coding-agent/src/native_extensions/learning/store.rs) | Verified merged fix |
| [WOR-202](https://linear.app/work-lpz/issue/WOR-202) | [append_after_a_torn_final_record_is_not_glued_onto_it](../crates/davinci-coding-agent/src/native_extensions/learning/store.rs) | Verified merged fix |
| [WOR-203](https://linear.app/work-lpz/issue/WOR-203) | [unreadable_journals_fail_open_instead_of_becoming_empty](../crates/davinci-coding-agent/src/native_extensions/learning/store.rs) | Verified merged fix |
| [WOR-204](https://linear.app/work-lpz/issue/WOR-204) | [create_rolls_back_skill_file_when_ledger_append_fails](../crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs) | Verified merged fix |
| [WOR-205](https://linear.app/work-lpz/issue/WOR-205) | [patch_with_invalid_applicability_leaves_the_skill_untouched](../crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs) | Verified merged fix |
| [WOR-206](https://linear.app/work-lpz/issue/WOR-206) | [write_file_cannot_replace_skill_md_or_overwrite_without_hash](../crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs) | Verified merged fix |
| [WOR-207](https://linear.app/work-lpz/issue/WOR-207) | [workspace_symlink_outside_the_repository_is_refused](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Verified merged fix |
| [WOR-208](https://linear.app/work-lpz/issue/WOR-208) | [cached_symbol_follows_the_conditional_type_entry](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Remaining gap fixed here; pending merge |
| [WOR-209](https://linear.app/work-lpz/issue/WOR-209) | [pnpm_virtual_store_link_outside_the_repository_is_refused](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Verified merged fix |
| [WOR-210](https://linear.app/work-lpz/issue/WOR-210) | [pnpm_virtual_store_finds_scoped_packages_without_a_top_level_link](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Verified merged fix |
| [WOR-211](https://linear.app/work-lpz/issue/WOR-211) | [pnpm_v9_snapshots_provide_dependency_edges](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Verified merged fix |
| [WOR-212](https://linear.app/work-lpz/issue/WOR-212) | [root_importer_version_wins_over_the_name_keyed_package_map](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Verified merged fix |
| [WOR-213](https://linear.app/work-lpz/issue/WOR-213) | [pnpm_optional_dependencies_are_lock_dependencies](../crates/davinci-coding-agent/tests/package_intelligence_audit.rs) | Verified merged fix |
| [WOR-214](https://linear.app/work-lpz/issue/WOR-214) | [every_lockfile_keeps_multiple_versions_of_one_package](../crates/davinci-coding-agent/src/native_extensions/package_intelligence/locks/mod.rs) | Verified merged fix |
| [WOR-215](https://linear.app/work-lpz/issue/WOR-215) | [test_security_guards_option_injection_and_traversal](../crates/davinci-coding-agent/tests/git_intelligence.rs) | Verified merged fix |
| [WOR-216](https://linear.app/work-lpz/issue/WOR-216) | [body_only_edit_on_the_same_lines_is_a_modified_symbol](../crates/davinci-coding-agent/tests/git_intelligence_audit.rs) | Verified merged fix |
| [WOR-217](https://linear.app/work-lpz/issue/WOR-217) | [test_changed_symbols_keeps_same_named_methods_in_different_scopes](../crates/davinci-coding-agent/tests/git_intelligence.rs) | Verified merged fix |
| [WOR-218](https://linear.app/work-lpz/issue/WOR-218) | [paths_with_spaces_survive_changed_symbols_and_branch_diff](../crates/davinci-coding-agent/tests/git_intelligence_audit.rs) | Verified merged fix |
| [WOR-219](https://linear.app/work-lpz/issue/WOR-219) | [branch_diff_reports_the_true_file_count_beyond_max_files](../crates/davinci-coding-agent/tests/git_intelligence_audit.rs) | Verified merged fix |
| [WOR-220](https://linear.app/work-lpz/issue/WOR-220) | [blame_keeps_authors_for_repeated_commit_hunks](../crates/davinci-coding-agent/tests/git_intelligence_audit.rs) | Verified merged fix |
| [WOR-221](https://linear.app/work-lpz/issue/WOR-221) | [symbol_history_keeps_the_rename_commit_under_its_new_path](../crates/davinci-coding-agent/tests/git_intelligence_audit.rs) | Verified merged fix |
| [WOR-222](https://linear.app/work-lpz/issue/WOR-222) | [merge_commit_context_reports_changes_against_the_first_parent](../crates/davinci-coding-agent/tests/git_intelligence_audit.rs) | Verified merged fix |
| [WOR-223](https://linear.app/work-lpz/issue/WOR-223) | [a_failed_did_close_keeps_the_deleted_document_tracked](../crates/davinci-coding-agent/src/native_extensions/language_intelligence/session.rs) | Verified merged fix |
| [WOR-224](https://linear.app/work-lpz/issue/WOR-224) | [a_rejected_registration_batch_registers_nothing](../crates/davinci-coding-agent/src/native_extensions/language_intelligence/client_requests.rs) | Verified merged fix |
| [WOR-225](https://linear.app/work-lpz/issue/WOR-225) | [cancellation_interrupts_write_queue_backpressure](../crates/davinci-coding-agent/src/native_extensions/language_intelligence/transport.rs) | Verified merged fix |
| [WOR-226](https://linear.app/work-lpz/issue/WOR-226) | [dynamic_diagnostics_follow_the_registered_document_selector](../crates/davinci-coding-agent/src/native_extensions/language_intelligence/client_requests.rs) | Verified merged fix |
| [WOR-227](https://linear.app/work-lpz/issue/WOR-227) | [non_streaming_token_limit_is_not_reported_complete](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-228](https://linear.app/work-lpz/issue/WOR-228) | [non_streaming_reply_keeps_every_text_part](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-229](https://linear.app/work-lpz/issue/WOR-229) | [non_streaming_malformed_tool_arguments_keep_the_sentinel](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-230](https://linear.app/work-lpz/issue/WOR-230) | [vision_models_receive_images_in_every_serializer](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-231](https://linear.app/work-lpz/issue/WOR-231) | [vision_models_receive_images_in_every_serializer](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-232](https://linear.app/work-lpz/issue/WOR-232) | [vision_models_receive_images_in_every_serializer](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-233](https://linear.app/work-lpz/issue/WOR-233) | [vision_models_receive_images_in_every_serializer](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-234](https://linear.app/work-lpz/issue/WOR-234) | [vision_models_receive_images_in_every_serializer](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-235](https://linear.app/work-lpz/issue/WOR-235) | [rejects_https_proxy_urls_instead_of_plaintext_connect](../crates/davinci-ai/src/http_proxy.rs) | Verified merged fix |
| [WOR-236](https://linear.app/work-lpz/issue/WOR-236) | [provider_header_templates_are_resolved_after_model_composition](../crates/davinci-ai/src/model_config.rs) | Verified merged fix |
| [WOR-237](https://linear.app/work-lpz/issue/WOR-237) | [device_flow_shows_code_polls_and_returns_tokens](../crates/davinci-ai/src/device_login.rs) | Verified merged fix |
| [WOR-238](https://linear.app/work-lpz/issue/WOR-238) | [anthropic_oauth_environment_token_is_not_an_api_key](../crates/davinci-ai/src/auth.rs) | Verified merged fix |
| [WOR-239](https://linear.app/work-lpz/issue/WOR-239) | [bedrock_requests_are_signed_with_region_and_session_token](../crates/davinci-ai/src/cloud_auth.rs) | Verified merged fix |
| [WOR-240](https://linear.app/work-lpz/issue/WOR-240) | [authorized_user_adc_is_exchanged_for_a_bearer_token](../crates/davinci-ai/src/cloud_auth.rs) | Verified merged fix |
| [WOR-241](https://linear.app/work-lpz/issue/WOR-241) | [cloudflare_urls_are_materialized_and_ids_never_sent_as_headers](../crates/davinci-ai/src/auth.rs) | Verified merged fix |
| [WOR-242](https://linear.app/work-lpz/issue/WOR-242) | [cloudflare_gateway_token_uses_the_gateway_header](../crates/davinci-ai/src/auth.rs) | Verified merged fix |
| [WOR-243](https://linear.app/work-lpz/issue/WOR-243) | [pi_messages_uses_its_gateway_wire_protocol](../crates/davinci-ai/src/stream.rs) | Verified merged fix |
| [WOR-244](https://linear.app/work-lpz/issue/WOR-244) | [nested_and_negated_workspace_globs](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-245](https://linear.app/work-lpz/issue/WOR-245) | [symlinked_workspace_outside_the_repository_is_not_loaded](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-246](https://linear.app/work-lpz/issue/WOR-246) | [test_root_configuration_change_affects_every_package](../crates/davinci-coding-agent/tests/build_intelligence.rs) | Verified merged fix |
| [WOR-247](https://linear.app/work-lpz/issue/WOR-247) | [argv_is_arguments_only_and_targets_the_selected_workspace](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-248](https://linear.app/work-lpz/issue/WOR-248) | [argv_is_arguments_only_and_targets_the_selected_workspace](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-249](https://linear.app/work-lpz/issue/WOR-249) | [tsconfig_with_comments_and_trailing_commas_keeps_its_references](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-250](https://linear.app/work-lpz/issue/WOR-250) | [nx_cache_false_is_not_overridden_and_defaults_are_not_targets](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-251](https://linear.app/work-lpz/issue/WOR-251) | [nx_cache_false_is_not_overridden_and_defaults_are_not_targets](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-252](https://linear.app/work-lpz/issue/WOR-252) | [turbo_package_override_replaces_the_generic_task](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-253](https://linear.app/work-lpz/issue/WOR-253) | [package_manager_follows_corepack_field_and_bun_text_lockfile](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-254](https://linear.app/work-lpz/issue/WOR-254) | [package_manager_follows_corepack_field_and_bun_text_lockfile](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-255](https://linear.app/work-lpz/issue/WOR-255) | [repo_intelligence_reads_jsonc_tsconfig_aliases](../crates/davinci-coding-agent/tests/build_intelligence_audit.rs) | Verified merged fix |
| [WOR-256](https://linear.app/work-lpz/issue/WOR-256) | [completed_file_queues_are_released](../crates/davinci-agent/src/file_mutation_queue.rs) | Verified merged fix |
| [WOR-257](https://linear.app/work-lpz/issue/WOR-257) | [stalled_cancel_acknowledgment_preserves_the_original_request_budget](../crates/davinci-coding-agent/src/interaction_testing/browser_process.rs) | Verified merged fix |
| [WOR-258](https://linear.app/work-lpz/issue/WOR-258) | [repo_map_ten_thousand_module_chain_matches_reference_degrees](../crates/davinci-coding-agent/src/native_extensions/repo_intelligence/queries.rs) | Verified merged fix |
| [WOR-259](https://linear.app/work-lpz/issue/WOR-259) | [test_cache_runtime_caching_and_telemetry](../crates/davinci-coding-agent/tests/change_impact.rs) | Verified merged fix |
| [WOR-260](https://linear.app/work-lpz/issue/WOR-260) | [test_impact_results_reach_the_report](../crates/davinci-coding-agent/tests/change_impact_audit.rs) | Verified merged fix |
| [WOR-261](https://linear.app/work-lpz/issue/WOR-261) | [symbol_names_are_resolved_to_test_impact_ids](../crates/davinci-coding-agent/tests/change_impact_audit.rs) | Verified merged fix |
| [WOR-262](https://linear.app/work-lpz/issue/WOR-262) | [scope_limits_the_analyzed_inputs](../crates/davinci-coding-agent/tests/change_impact_audit.rs) | Verified merged fix |
| [WOR-263](https://linear.app/work-lpz/issue/WOR-263) | [uncommitted_edits_are_the_default_change_set](../crates/davinci-coding-agent/tests/change_impact_audit.rs) | Verified merged fix |
| [WOR-264](https://linear.app/work-lpz/issue/WOR-264) | [imports_match_by_resolved_module_not_by_name_suffix](../crates/davinci-coding-agent/tests/change_impact_audit.rs) | Verified merged fix |
| [WOR-265](https://linear.app/work-lpz/issue/WOR-265) | [background_review_status_changes_follow_the_write_policy](../crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs) | Verified merged fix |
| [WOR-266](https://linear.app/work-lpz/issue/WOR-266) | [background_review_status_changes_follow_the_write_policy](../crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs) | Verified merged fix |
| [WOR-267](https://linear.app/work-lpz/issue/WOR-267) | [create_rejects_frontmatter_that_names_another_skill](../crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs) | Verified merged fix |
| [WOR-268](https://linear.app/work-lpz/issue/WOR-268) | [an_interrupted_restore_journal_fails_closed_until_it_is_finished](../crates/davinci-coding-agent/tests/workspace_snapshot.rs) | Verified merged fix |
| [WOR-269](https://linear.app/work-lpz/issue/WOR-269) | [restore_staging_never_touches_existing_sibling_files](../crates/davinci-coding-agent/src/native_extensions/workspace_snapshot/mod.rs) | Verified merged fix |
| [WOR-270](https://linear.app/work-lpz/issue/WOR-270) | [a_grapheme_wider_than_the_viewport_does_not_recurse_forever](../crates/davinci-tui/src/word_wrap.rs) | Verified merged fix |
| [WOR-271](https://linear.app/work-lpz/issue/WOR-271) | [narrow_viewports_never_exceed_the_requested_width](../crates/davinci-tui/src/truncated_text.rs) | Verified merged fix |
| [WOR-272](https://linear.app/work-lpz/issue/WOR-272) | [set_value_snaps_the_cursor_to_a_character_boundary](../crates/davinci-tui/src/input.rs) | Verified merged fix |
| [WOR-273](https://linear.app/work-lpz/issue/WOR-273) | [searching_resets_the_selection_so_enter_acts_on_a_visible_row](../crates/davinci-tui/src/scoped_models.rs) | Verified merged fix |
| [WOR-274](https://linear.app/work-lpz/issue/WOR-274) | [reorder_follows_the_moved_model_across_disabled_rows](../crates/davinci-tui/src/scoped_models.rs) | Verified merged fix |
| [WOR-275](https://linear.app/work-lpz/issue/WOR-275) | [filter_is_unicode_aware_and_matches_the_visible_label](../crates/davinci-tui/src/item_select_list.rs) | Verified merged fix |
| [WOR-276](https://linear.app/work-lpz/issue/WOR-276) | [filter_is_unicode_aware_and_matches_the_visible_label](../crates/davinci-tui/src/item_select_list.rs) | Verified merged fix |
| [WOR-277](https://linear.app/work-lpz/issue/WOR-277) | [leaving_manual_input_disables_stale_submission](../crates/davinci-tui/src/login_dialog.rs) | Verified merged fix |
| [WOR-278](https://linear.app/work-lpz/issue/WOR-278) | [submitted_secrets_never_enter_the_transcript](../crates/davinci-tui/src/login_dialog.rs) | Verified merged fix |
| [WOR-279](https://linear.app/work-lpz/issue/WOR-279) | [hyperlinks_cannot_inject_terminal_sequences](../crates/davinci-tui/src/login_dialog.rs) | Verified merged fix |
| [WOR-280](https://linear.app/work-lpz/issue/WOR-280) | [browser_spawn_failure_preserves_the_manual_url](../crates/davinci-tui/tests/launcher_failures.rs) | Verified merged fix |
| [WOR-281](https://linear.app/work-lpz/issue/WOR-281) | [clipboard_spawn_failure_never_claims_a_copy](../crates/davinci-tui/tests/launcher_failures.rs) | Verified merged fix |
| [WOR-282](https://linear.app/work-lpz/issue/WOR-282) | [fixture_substring_does_not_skip_the_production_launcher](../crates/davinci-tui/tests/launcher_failures.rs) | Verified merged fix |
| [WOR-283](https://linear.app/work-lpz/issue/WOR-283) | [default_components_that_symlink_outside_the_root_are_ignored](../crates/davinci-coding-agent/src/plugins/manifest.rs) | Verified merged fix |
| [WOR-284](https://linear.app/work-lpz/issue/WOR-284) | [default_components_that_symlink_outside_the_root_are_ignored](../crates/davinci-coding-agent/src/plugins/manifest.rs) | Verified merged fix |
| [WOR-285](https://linear.app/work-lpz/issue/WOR-285) | [concurrent_mcp_json_edits_are_not_lost](../crates/davinci-coding-agent/src/plugins/manager.rs) | Verified merged fix |
| [WOR-286](https://linear.app/work-lpz/issue/WOR-286) | [uninstall_reports_a_real_cache_cleanup_failure](../crates/davinci-coding-agent/src/plugins/command.rs) | Verified merged fix |
| [WOR-287](https://linear.app/work-lpz/issue/WOR-287) | [pairing_requires_the_same_module_directory](../crates/davinci-coding-agent/src/native_extensions/test_impact/analysis.rs) | Verified merged fix |
| [WOR-288](https://linear.app/work-lpz/issue/WOR-288) | [helpers_and_fixtures_under_test_directories_are_not_tests](../crates/davinci-coding-agent/src/native_extensions/test_impact/analysis.rs) | Verified merged fix |
| [WOR-289](https://linear.app/work-lpz/issue/WOR-289) | [repo_intelligence_package_exports_conditions_subpaths_and_tsconfig_extends](../crates/davinci-coding-agent/tests/repo_intelligence_edges.rs) | Verified merged fix |
| [WOR-290](https://linear.app/work-lpz/issue/WOR-290) | [repo_intelligence_package_exports_conditions_subpaths_and_tsconfig_extends](../crates/davinci-coding-agent/tests/repo_intelligence_edges.rs) | Verified merged fix |
| [WOR-291](https://linear.app/work-lpz/issue/WOR-291) | [repo_intelligence_package_exports_conditions_subpaths_and_tsconfig_extends](../crates/davinci-coding-agent/tests/repo_intelligence_edges.rs) | Verified merged fix |
| [WOR-292](https://linear.app/work-lpz/issue/WOR-292) | [repo_intelligence_export_star_barrels_resolve_calls](../crates/davinci-coding-agent/tests/repo_intelligence_edges.rs) | Verified merged fix |
| [WOR-293](https://linear.app/work-lpz/issue/WOR-293) | [oversized_attachments_are_refused_before_they_are_read](../crates/davinci-coding-agent/src/file_processor.rs) | Verified merged fix |
| [WOR-294](https://linear.app/work-lpz/issue/WOR-294) | [file_wrappers_cannot_be_closed_or_forged_by_names_or_content](../crates/davinci-coding-agent/src/file_processor.rs) | Verified merged fix |
