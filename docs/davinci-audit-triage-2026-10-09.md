# DaVinci audit triage: WOR-175–294

> **Triage plus the Sonnet 5.5 fixes.** This document records the source-review backlog and proposed implementation assignments. The "Implementation status" section at the end lists which issues this PR fixes and how each is covered; everything else is unfixed.

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

The Sonnet-assigned issues have source fixes with regression tests in this PR. The 44 Opus-assigned issues are untouched. Each finding was fixed from its Linear description and the current code; none was reproduced on an earlier commit first.

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

Known gaps: WOR-286 (plugin uninstall cleanup warning) and WOR-280/281 (launch and clipboard failure reporting) have no failure-path test; they need an undeletable directory or a missing launcher. `repo_intelligence_edges` does not compile on Windows because of an existing unix-only import, so it was not run here.
