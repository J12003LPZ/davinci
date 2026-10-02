# OpenAI harness implementation — 2026-10-02

Execution record for the supplied October 1 OpenAI production-harness plan (`2026-10-01-openai-harness-production-plan.md`). The supplied plan remains outside this commit. This record supersedes neither the release validator nor the historical measurements in the earlier readiness report. It records local engineering and its acceptance limits.

## Identity and authority

The input is an exported source tree at `C:\Users\sergi\Desktop\davinci-main`, without Git metadata. No commit or branch is asserted. The original 4,462-file export is backed up in `C:\Users\sergi\.codex\tmp\openai-harness-01a0fac0\source-before.zip`, with a SHA-256 file inventory in `source-before.json`. Changes are compared against that archive, preserving pre-existing content.

Implementation and review were solo, as requested. Tests used an external Cargo target directory, offline dependency resolution after the initial locked dependency fetch, disabled live-provider paths, and isolated fixtures. The initial implementation phase performed no paid campaign, credential change, installation, release, commit, push, deployment, or competing harness run. The implementation does not claim measured savings or superior task success.

The subsequent authorized PR handoff uses a separate Git worktree based on [audit PR #81](https://github.com/J12003LPZ/davinci/pull/81), commit `ae96d00d715438bc620094da74d8f5d6e3fb73ca`. Its tracked baseline matches the original export after line-ending normalization; all 70 implementation paths and the 19 tracked dependency fixtures omitted from the original inventory were checked against the tested export before transfer. The PR is stacked on the audit branch to preserve those fixes without duplicating them. This handoff establishes a reviewable Git revision; successful CI for the submitted commit and production acceptance remain separate gates.

The evidence directory contains command receipts, exit codes, complete logs and log hashes. `final-source.json` and `local-build.identity.json` identify the final source and fixture executable after the final build; `validation-summary.json` indexes the executed checks. These are local evidence, not release provenance. A fixture-enabled build is rejected by release/campaign admission.

The subsequent production-acceptance configuration fixes `gpt-6-luna` with high reasoning effort on the user's existing Codex subscription, through `openai-codex-responses` and Codex OAuth. Desktop-app usage is not a prerequisite. PR [#82](https://github.com/J12003LPZ/davinci/pull/82) precedes live acceptance. The user subsequently authorized reader subagents for acceptance work; source changes remain with one writer. The accepted native Windows artifact may be installed locally after the gates pass and its installed SHA-256 must match the tested executable. Public release publication and external deployment remain unauthorized.

No API spend authorized. Subscription-only campaign. Budget by subscription usage allowance, request count, task count, and wall-clock time; API-equivalent dollars are reporting-only.

The first pass uses at most 25% of the currently available subscription allowance only if that allowance can be measured and enforced reliably. The present adapter has no reliable durable allowance measurement, so the campaign instead freezes hard request, task and wall-clock caps, with available account usage snapshots recorded before and after. Account-wide changes are not attributed solely to DaVinci. Quota exhaustion, transport failure and unsupported functionality stop the campaign; there is no billing-route fallback. The initial suite requires 40–60 real tasks plus an untouched holdout. A future superiority claim requires at least 150 independent tasks across 3–5 real repositories and repeated matched-model runs.

## Delivered behavior

- Actual provider sends carry root/actor/request/attempt identities and requested/returned model metadata. Foreground, worker, reviewer, compaction and enabled background purposes retain raw usage provenance. Missing cache-write counts, missing final usage, overflow and conflicting duplicate receipts remain unknown. Message summaries do not charge the same attempts again.
- A durable root budget reserves request/output allowances at the transport boundary, shares the ledger with children, retains uncertain reservations, and refuses new sends on unresolved accounting. Resumption reopens the same allowance. Strict monetary admission refuses when a trustworthy route-specific upper bound is unavailable.
- Requirement text and targeted corrections survive two compactions and session reload, including oversized requirements. A green subset of checks does not manufacture coverage of unrelated requirements. No mandatory extra planner call was added.
- Verification needs host-owned execution receipts, unchanged relevant inputs and discovered tests for recognized test runners. Printed success text, synthetic exit metadata, zero discovered tests and stale graph verification are insufficient.
- Repeated unchanged test failures produce bounded diagnostic feedback, then stop further provider dispatch. The repair state is checkpointed at provider boundaries and restored with the session. Read repetition remains diagnostic; it cannot independently block progress through this repair gate.
- Safe concurrent read regions refill freed scheduler slots while preserving barriers, source-order results and cancellation behavior. Expanded operations remain visible separately from model tool-call envelopes.
- Risk-specific acceptance packs and an evidence-bound delivery record distinguish implemented, locally verified, integration verified, release candidate and deployed/operationally verified. Deployment authority and installed-artifact identity are separate requirements.
- An owned local product fixture exercises persistence across restart, negative authorization, startup failure, partial migration rollback, database unavailability, request timeout, redacted logs and teardown through the real process supervisor.

## Budget operation

`--root-budget <path>` is an opt-in JSON configuration. The path is resolved from the launch directory; the ledger path inside it is resolved from the configuration's directory. The schema contains `root_id`, `ledger`, `limits`, `per_attempt_max_output_tokens` and optional `parent_actor_id`. Limits contain positive `max_requests` and `deadline_unix_ms`. The existing strict mode also requires positive `max_output_tokens`; its optional `max_cost_microusd` is in millionths of USD. The deadline is absolute Unix milliseconds, so resume cannot renew it. Configuration is bounded to 16 KiB and rejects unknown fields.

Child CLIs inherit an immutable descriptor through `DAVINCI_INHERITED_ROOT_BUDGET`, reopen the existing ledger, and reject a replacement CLI budget flag. Persisted sessions require the same budget binding before execution. Starting or resuming without that binding does not reset the root. A cold resume retains unfinished reservations as unknown; reconcile them from trustworthy provider evidence before admitting more work.

Request and output limits can be enforced on supported public/Azure Responses and configured completion routes. The private Codex route does not have a verified output-ceiling contract here, so strict output-bounded admission refuses it. Price estimates do not establish a strict spend bound. The absolute deadline gates new admissions; it does not guarantee termination of a request already in flight. This is host accounting, not an OS security boundary against arbitrary programs running as the same user.

The separate subscription mode declares `limits.codex_subscription` with the fixed model and effort and requires both output and money ceilings, including the per-attempt output ceiling, to be null. It counts actual sends durably without asserting a public Responses output/cache contract. Before transport dispatch it requires Codex OAuth, the exact ChatGPT Codex endpoint, matching model/high effort, explicit SSE and zero retries. Budgeted HTTP requests do not follow redirects. Failed or unidentifiable terminal responses halt the ledger, while missing token counters remain null. Raw unaccounted provider probes are refused. The campaign supervisor additionally enforces task and wall-clock caps and requires new committed receipts for each launch without dropping previous receipts. Isolated credentials contain only the Codex OAuth entry; inherited API keys are stripped. Native process ownership does not isolate arbitrary tool code from all files or network access available to the OS user.

## Telemetry interpretation

JSON print mode emits a schema-1 `harness_stats` snapshot before any final approval-required record. RPC `get_session_stats` exposes the same counters under `runtime`; status rendering is read-only. The benchmark parser accepts only numeric, allowlisted fields from the snapshot and takes the latest cumulative snapshot rather than summing duplicates.

`wallMs` is observed host elapsed time. `preparationMs`, `queueMs`, `providerMs` and `retryWaitMs` separate foreground preparation, admission, completion-adapter execution and retry backoff. Existing `modelWallMs` retains its compatible meaning, including queue/backoff. `toolWallMs` counts overlap once per batch. `verificationWorkMs` is summed process work and can overlap tools or workers: it must never be added to host elapsed time as though it were a disjoint stage. Preparation may include nested compaction. Missing observations load as null, including old saved statistics.

`digestRetrievalMs` measures immutable context-image preparation (including fingerprinting and failed preparation) and explicit context retrieval. `integrationMs` measures dispatched `patch_apply` publication, including failures; it does not measure manual merges or certify combined changes. These are observed work and can overlap preparation or tool time.

Read/search diagnostics retain at most 256 hashes per in-process runtime root. `repeatedReads` and `repeatedSearches` require matching arguments, access policy/revision, contract scope, source bytes before/after execution, successful output and prior unpruned evidence still present in the actor's context. `workerDuplicateOperations` counts matching successful work by different actors in that root; it identifies potential duplicate investigation without asserting that the actor had the other worker's evidence. Root snapshots are cumulative and must not be summed across worker snapshots. None of these counters veto execution.

Coverage is explicit: `diagnosticComparableOperations` and `diagnosticUnknownOperations` accompany the counts. Fresh confined reads are bounded to 1 MiB. Directory searches, larger files, aliases, failed operations and unavailable source/permission evidence are unknown; active context paging does not assert visible repetition without a provider-view receipt. Evicted history and separate worker processes are outside this diagnostic window. `diagnosticsMs` measures the collection overhead. All these fields are null before observation, including legacy records. Existing cache/governor counters retain their narrower meanings. Aggregate telemetry does not include prompts, source bodies or raw URLs; detailed existing traces remain governed by their separate opt-in controls.

## Plan acceptance matrix

“Fixture verified” means deterministic local coverage, not live efficacy or production acceptance. Existing controls were retained where the current suite already covered the required boundary; proposed test-file names were mapped to those existing owners rather than duplicated.

| Tasks | Implementation / evidence owner | Acceptance status |
| --- | --- | --- |
| S01 | Existing permission/web destination, redirect and URL audit fixtures | Local regressions verified; no new URL implementation needed |
| S02 | Delegation restriction parser and regressions, including curly apostrophes | Fixed and fixture verified |
| S03–S04 | Existing plugin transaction, corruption, disabled-state and publication tests | Local regression coverage retained |
| S05 | AI stream cancellation/backpressure fixtures and owned process lifecycle tests | Local cleanup covered; remote cancellation is not proof of zero usage |
| M00 | `scripts/release_identity.py`, runner identity tests | Export transferred to isolated PR worktree; exact submitted-head green CI and release build still required |
| M01 | Provider observations, agent stats, background ledger, Python receipt reconciliation | Fixture accounting verified; live print/RPC/interactive reconciliation remains unmeasured |
| M02 | `harness_root_budget`, AI process-budget wire test, capacity tests | Durable admission and resume verified; monetary/private-Codex bounds limited as above |
| M03 | `harness_accounting`, `tool_diagnostics`, stats, transaction dispatch and benchmark/readiness metrics | Stage separation, overlap and bounded state-matched diagnostics; coverage limits above remain explicit |
| Q01 | Requirement completion and compaction/resume regressions | Exact clauses preserved; independent semantic task success still requires acceptance evidence |
| Q02 | Receipt/workspace/transaction checks, test discovery, graph verification | Freshness and discovery regressions verified; protected-grader isolation is platform dependent |
| Q03 | Progress watchdog, durable repair checkpoint and provider-boundary tests | Bounded stop/resume covered; a crash before the next checkpoint may lose the latest diagnostic observation |
| C01 | Existing route capability registry and conservative wire fixtures | Retained; no fresh authenticated capability probe or efficacy claim |
| C02–C03 | Existing prepared-context, prefix and cache-policy fixtures | Stable prefix/context ownership covered locally; workload cache benefit unmeasured |
| C04 | Existing authorization-aware tool-surface/exposure fixtures | Retained; Full remains default |
| C05 | Raw usage, cache report and weighted completeness tests | Missing writes/cold baseline remain unknown; prefix diagnostics are not bills |
| C06 | Existing Responses/Codex ownership, replay and recovery fixtures | Retained; private-route recovery limits preserved |
| C07 | Existing route-specific optional-field gates | Conditional; unsupported fields remain omitted, no probe authority inferred |
| X01 | Existing context budget, mandatory-policy, evidence recovery and prepared-image tests | Fixture verified; long-task semantic success unmeasured |
| X02 | Existing cache freshness, permission invalidation and coalescing tests | Retained; mutations are not coalesced as reads |
| X03 | Existing bounded session listing and Context VM tests | VM remains off by default; no measured promotion |
| T01 | Existing Full/Lean discovery controls and expanded-operation telemetry | Retained; no measured surface winner |
| T02 | Refilling scheduler tests | Implemented and fixture verified; no latency improvement claim |
| T03 | Existing managed-process readiness/reuse plus product journey fixture | Owned local lifecycle verified; production startup/reuse remains application specific |
| A01 | Existing task strategy/delegation contracts | Retained; no worker-count optimization claim |
| A02 | Existing worktree/worker isolation and graph artifact verification tests | Fixture covered; this export cannot prove a Git-backed final combined delivery |
| A03 | Existing durable graph/operation journals plus root admission | Recovery and root limits covered locally; no live multi-worker campaign |
| A04 | Existing fixed-model worker/bootstrap controls | Retained; no automatic model-routing promotion |
| P01 | Acceptance pack and planner tests | Risk-specific required checks implemented; a suggested pack is not a passing receipt |
| P02 | `harness_product_journey` and product acceptance evals | Owned synthetic app exercised; real product/browser/platform journeys remain external acceptance |
| P03 | `completion_delivery` and graph status | Evidence levels implemented; graph status stays at implemented when host semantic receipts are absent |
| E01 | Existing private-suite importer, holdout and grader fixtures | Real historical DaVinci tasks being curated; candidate count alone is not validated task admission |
| E02 | Existing paired protocol, subscription campaign policy, all-attempt metrics and admission gates | Subscription-only campaign authorized; fixed baseline and finite manifest required before launch |
| E03 | Existing ablation controls plus compaction/resume/background regressions | Offline boundaries covered; dev/holdout and long-session efficacy unmeasured |
| R01 | Existing release validator plus delivery levels | No candidate promoted, installed or deployed; canary/rollback acceptance not exercised |
| R02 | Keep/cut decisions below | Unmeasured components retained or left opt-in; no speculative deletion |

## Keep/cut and rollback

| Component | Decision | Basis |
| --- | --- | --- |
| Existing loop, hooks, journal, scheduler, cache and verification owners | Retain | Extend existing ownership rather than introduce orchestration machinery |
| Full tool surface and selected OpenAI model | Retain defaults | No matched campaign established a better default |
| Lean surface, Context VM, optional compaction/prewarm and routing candidates | Keep current opt-in/capability gates | No efficacy evidence from this run |
| Root budget | Opt-in; immutable once bound to a session | New request/output controls fail closed on uncertainty |
| Repeated-failure repair stop | Retain bounded host gate | Regression tests cover dispatch stopping and durable resume |
| Non-OpenAI providers | Preserve | Removing them was not required by OpenAI-only optimization |

Before rollback, preserve session and root-ledger files; never erase reservations to regain allowance. Restoring source requires comparing against the recorded pre-edit archive and retaining later unrelated changes. This session did not overwrite the installed CLI, so no installed-binary rollback is needed. A production promotion still requires an identified release artifact, independent task acceptance, platform lifecycle checks, and an explicitly authorized canary/rollback rehearsal.

## Validation record

See `C:\Users\sergi\.codex\tmp\openai-harness-01a0fac0\validation-summary.json` for exact executed commands and counts. The final validation lanes are the whole offline Rust workspace with `test-fixtures`, strict workspace Clippy, formatting, Python benchmark/identity regressions, and a freshly built isolated executable. Ignored platform/live/browser/benchmark tests are not counted as passed. Windows-native fixture success does not establish Linux/WSL/macOS containment or a real browser journey.

The acceptance level for this delivery is local engineering verification with the specific unmeasured gates above. Production readiness, measured efficiency and independent semantic superiority are not established by this record.

### Subscription acceptance follow-up

The subscription policy is now enforced at final request admission and each actual transport attempt. Budgeted HTTP clients refuse redirects; rejected or unidentified terminal receipts fail the operation and halt the subscription ledger. Each task must add fresh receipts without replacing prior reservations. Only Codex OAuth credentials are copied into the campaign settings directory, and inherited API-key environment variables are removed. Native Windows process ownership is verified separately from filesystem/network isolation; the native campaign remains diagnostic-only.

The updated local workspace run passed 5,325 tests with zero failures and 40 ignored tests. Python benchmark checks ran 169 tests, with 165 passing and four skipped. Workspace formatting and strict all-target Clippy with `test-fixtures` passed. Full command receipts and log hashes are recorded in `acceptance-local-validation.json` beside the earlier evidence. Initial failed runs exposed a nonblocking Windows fixture socket and an MCP fixture path prerequisite; the final run uses an explicit fixture executable path. The actual 303 redirect regression failed before the redirect fix and passed afterward; 307 was already refused.

The real-task catalog currently contains 40 development candidates and five unchanged holdout entries. Four historical Python cases have behavioral starter failures and passing reference tests; three Rust pilot candidates await validation. These counts are curation progress, not live task successes. Exact submitted-source CI, a fresh production release build, the authorized subscription campaign, operational acceptance and tested/installed hash equality remain required before installation.
