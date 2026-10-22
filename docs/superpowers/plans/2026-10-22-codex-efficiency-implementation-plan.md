# Da Vinci Codex Efficiency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **No steps have been executed; this plan does not authorize code changes, commits, provider calls or release.**

**Goal:** Increase independently verified Da Vinci coding tasks completed per Codex subscription allowance (where measurable), while preserving quality and making provider-request, caching and context costs observable and controllable.

**Architecture:** Extend existing Da Vinci measurement, completion, Context VM, retrieval/cache, and verification owners. Prioritize eliminating proven unnecessary model turns, then evaluate real Off/Hybrid/Active caching, and introduce narrow deterministic local artifact reuse only when justified by measurements. Ship improvements as independently reversible changes rather than a replacement inference harness.

**Tech Stack:** Existing Rust workspace and locked dependencies, Python 3.11+ benchmark scripts, JSONL native sessions, existing `davinci-agent`/`davinci-ai`/`davinci-coding-agent`/`davinci-evals` crates, Codex OAuth for separately authorized live testing.

**Spec:** `docs/superpowers/specs/2026-10-22-codex-efficiency-design.md`; supporting source: user-supplied *Optimizing Codex/OpenAI Performance and Cost Efficiency for the Da Vinci Terminal* (32-page report), existing repository evidence linked in the spec.

**Delivery:** Review this staged document, copy both staged Markdown files to the paths above on a task branch in the main checkout (no worktree; owner preference), then implement in the priority order of the sequencing table. No repository change was made to write this plan.

**Revision 2026-10-22 (owner-approved review):** added Task 0 (the Codex backend route has never carried a live DaVinci request, `docs/cache/chatgpt-backend-route.md:3`); Task 2 now extends `campaign.paired_metrics` instead of adding a sixth summarizer; Task 5 RED list corrected to tests that actually fail today plus the workspace-wide-check gap; Workstreams B, C and D deferred behind the requirement-review lever. Second pass: the 62.8% evidence predates `8a87a98e`, which already shipped the evidence-aware skip; post-fix data shows a smaller, two-gate residual (30% of requests after a reminder in 9 of 24 runs), so Task 4 now starts by attributing it from existing logs and needs multi-requirement tasks before any policy change.

## Global Constraints

- Preserve `contextVm=off` as default; Shadow, Active and Hybrid stay selectable; provider-side KV cache is never implemented locally.
- Keep subscription `openai-codex` OAuth endpoint, chosen model and `effortPolicy=fixed` unless explicitly separately approved. No API-key fallback or silent effort/model switching.
- Keep documented provider cache capability gating, exact prefix/identity derivation, native Responses replay, tool-call completeness, lineage and transport isolation.
- Do not conflate local prefix fingerprint, provider cache hits, WebSocket reuse or subscription billing. Unknown token values remain unknown; account usage is not inferred from API pricing.
- Do not reduce permissions, session durability, source provenance, cold recovery, requirement corrections, live-tool authorization or cancellation. Maintain existing verification and completion hooks when experimenting with requirement review.
- No automatically installed dependencies, changed personal settings, credentials, sessions, publication, deployment or paid traffic. Consent and per-route caps are prerequisites, not recommendations.
- Do not rewrite overlapping existing owners (`CacheRuntime`, `EngineeringSnapshots`, root budget, token governor, `autoVerify`) unless a verified gap cannot be closed locally.
- Pin actual default-branch SHA, clean source fingerprint, `rust-toolchain*`, `Cargo.lock`, fixture manifests and binary identity at implementation start; document environment and OS. Respect repository-pinned compiler version rather than values in older plans.
- Tests default to offline/loopback. All mutations follow RED -> observed failure -> minimal GREEN -> observed pass -> review -> optional approved commit.
- The existing stricter [cache benchmark protocol](https://github.com/J12003LPZ/davinci/blob/main/docs/cache/openai-cache-benchmark.md) and [readiness gates](https://github.com/J12003LPZ/davinci/blob/main/docs/readiness/openai-harness-implementation.md) win if they conflict with proposals here.

## Review Focus (each explicitly covered below)

1. **Missing/partial usage:** never produce zero cached tokens or an efficiency win from an absent/duplicate provider receipt (Tasks 0, 2, 3).
2. **Unverified edits and hidden requirements:** suppressing review must not falsely bless failed, stale, partial or zero-test verification (Tasks 4, 5).
3. **Compaction/cold resume/branch:** no lost user corrections, cited source refs, attached images, native tool pairs or state authority (Tasks 6, 7).
4. **Stale/unauthorized local memo:** changed content, permission revocation, branch changes or cross-project identities never serve an old fact (Tasks 9, 10).
5. **Interrupts, transport errors and budget exhaustion:** no repeated mutation, provider reroute, unknown usage silently cleared, leaked worker or unbounded extra LLM attempts (Tasks 3, 11, 13).

---

## Workstream A — Measure first, then remove demonstrated excess model calls

### Task 0: Confirm the Codex backend route reports usage (live smoke, requires approval)

**Why first:** every live measurement this plan cites (Oct 12 diagnosis, A1 Sep 26, Oct 17 public-task comparison) ran on the public `https://api.openai.com/v1` route. The `openai-codex` login routes to `https://chatgpt.com/backend-api/codex/responses` and `docs/cache/chatgpt-backend-route.md:3` records that no live DaVinci request has used it. If that route omits `usage` or `input_tokens_details.cached_tokens`, the accounting gates in Tasks 2, 3, 8 and 13 return NO-GO by construction, and the plan must be re-scoped before any engineering.

**Files:** Create `docs/perf/codex-efficiency/backend-route-smoke.md`. No production code. Raw responses stay outside the repo under the existing bench evidence root.

**Interfaces:** Consumes a fresh `/login openai-codex` credential, the current frozen binary and one small fixed prompt. Produces a field-presence table per request: `usage` present, `input_tokens`, `cached_tokens`, `output_tokens`, `reasoning_tokens`, response/request ID, HTTP vs WebSocket transport, TTFT, wall time, and any error class.

- [ ] **Step 1 — Manifest:** binary sha256, source SHA, model, effort, transport, prompt hash, hard cap of 5 requests and 10 minutes. Owner approves this manifest by name before any send.
- [ ] **Step 2 — Send:** 2 identical requests (cold then warm, to see whether a cache read is ever reported), 1 with a tool call round trip, at most 2 retries for transport failures. Stop on any auth, quota or route error; no fallback to `api.openai.com` or an API key.
- [ ] **Step 3 — Record:** fill the field-presence table; unknown stays unknown. Note subscription usage before and after if the account page shows it.
- [ ] **Step 4 — Decision:** `ROUTE_REPORTS_USAGE` (continue as planned), `ROUTE_PARTIAL_USAGE` (Tasks 2 and 3 must label the missing fields unavailable and the promotion gate uses only the reported ones), or `ROUTE_NO_USAGE` (stop; token-per-success goals are unmeasurable on this route; report to owner).
- [ ] **Step 5 — Approved doc commit:** `git commit -m "docs: record Codex backend route usage smoke"`.

**Acceptance:** A reviewer can see from the table exactly which accounting fields the backend route provides, from at most 5 approved requests.

### Task 1: Freeze reproducible baseline and audit existing feature owners

**Files:** Create `docs/perf/codex-efficiency/2026-10-22-baseline.md`; reference/inspect `docs/completion.md`, `docs/context-vm.md`, `docs/openai-efficiency.md`, `docs/tool-discovery.md`, `docs/readiness/openai-harness-implementation.md`; do not change production Rust.

**Interfaces:** Consumes repo source + manifest + existing smoke/bench commands. Produces a sealed baseline inventory (SHA, toolchain, feature defaults, task hashes, provider route, receipt schemas, commands, known unknowns) used by all other tasks.

- [ ] **Step 1 — Inventory:** record `git rev-parse HEAD`, `git status --porcelain`, toolchain, `Cargo.lock` hash, feature flag defaults, and fixture hashes; explain pre-existing staged/dirty state rather than deleting it.
- [ ] **Step 2 — Inspect current behavior:** cross-check requirements review gate, autoVerify, native replay, Off pruning, Hybrid compaction, context broker, `EngineeringSnapshots`, vector memory and Lean; mark implemented/tested/uncertain rather than calling them missing.
- [ ] **Step 3 — Run offline baseline tests:** `python -m unittest discover -s scripts/bench/tests -p 'test_*.py'`; `cargo test -p davinci-evals --lib context_lifecycle --offline --locked`; and applicable focused Rust tests, recording exit codes and failures without repairs outside scope.
- [ ] **Step 4 — Produce locked baseline report:** include confidence qualifiers and the original October 12 and October 19 source results; explicitly distinguish historical measurements from new data.
- [ ] **Step 5 — Review/commit if separately approved:** only the baseline doc, `git commit -m "docs: freeze Codex efficiency baseline"`.

**Acceptance:** An independent reviewer can reproduce the offline suite and identify precisely which implementation defaults and sources the experiment uses.

### Task 2: Add per-verified-success denominators to the existing paired analysis (no new module)

**Existing owners (do not duplicate):** `scripts/bench/campaign.py:197` `paired_metrics` already pairs exact task/repetition sets, includes failures, rejects mismatched membership and reports `unavailable_pairs` per field. `scripts/bench/readiness_protocol.py:29` `validate_pairs` already enforces frozen `source_sha`, `binary_sha256` and suite digest. `scripts/bench/bench.py:742` `summarize`, `bench.py:813` `paired_report` and `readiness_metrics.py:95` `summarize` already aggregate with unknowns kept as `None` (`aggregate_available`). The gap is only per-success efficiency.

**Files:** Modify `scripts/bench/campaign.py` and `scripts/bench/tests/test_campaign.py`; update `scripts/bench/README.md`. No new analysis module.

**Interfaces:** `paired_metrics(baseline, candidate)` gains a `per_verified_success` block per arm: `uncached_input_tokens`, `output_tokens`, `logical_requests`, `provider_attempts`, `wall_s`, each = arm total over all runs (failures included in the numerator) / arm passes. A value is `None` with a reason (`no_passes`, `unavailable_usage`) when not computable. Existing keys and ratios are unchanged.

- [ ] **Step 1 — RED:** in `test_campaign.py`, add `test_per_success_counts_failed_run_cost`, `test_per_success_unknown_usage_is_none_not_zero`, `test_per_success_zero_passes_is_none`, `test_per_success_does_not_double_count_cached`. Confirm the existing pairing tests already cover mismatched task/rep membership; add a case only if one is missing.
- [ ] **Step 2 — Confirm RED:** `python -m unittest discover -s scripts/bench/tests -p 'test_campaign.py' -v` (the tests directory has no `__init__.py` and imports via `sys.path.insert`, so dotted-module invocation does not work). Failures must be missing-key assertions, not import errors.
- [ ] **Step 3 — GREEN:** minimal addition inside `paired_metrics`, reusing its `metric()` and validity rules; do not subtract `cached_tokens` twice and do not treat cache writes as ordinary input.
- [ ] **Step 4 — Confirm GREEN:** `python -m unittest discover -s scripts/bench/tests -p 'test_*.py'`; run against one existing campaign under `C:\bench` and check the numbers by hand for one task.
- [ ] **Step 5 — Approved commit:** `git commit -m "bench: report cost per verified success in paired metrics"`.

**Acceptance:** A missing cache report cannot make a candidate look cheaper; failed runs stay in the cost numerator; no sixth summarizer exists.

### Task 3: Establish privacy-safe, end-to-end request attribution

**Files:** Modify only if needed: `scripts/bench/bench.py`, `scripts/bench/observations.py`, `scripts/bench/turn_cache_audit.py`, `scripts/bench/tests/test_bench.py`, `scripts/bench/tests/test_turn_cache_audit.py`; inspect existing `crates/davinci-agent/src/stats.rs`, `crates/davinci-ai/src/provider_observation.rs`, `crates/davinci-ai/src/stream_decoder.rs` first.

**Interfaces:** Each logical request report references provider attempts, purpose (`foreground`, `verification`, `compaction`, `reviewer`, etc.), known input/cache read/cache write/output counts, known/missing flags, elapsed time, completion-review reason and correlation-only fingerprint. Transport and prompt bytes stay outside reports.

- [ ] **Step 1 — RED:** fixture rows with a successful attempt plus failed retry, late/duplicate usage, missing cache-write field, worker call, compaction and cancelled stream. Assert exactly one normalized value per provider attempt and no secret prompt body in the report.
- [ ] **Step 2 — Confirm RED:** run focused Python and any affected Rust decoder tests; capture the contract failure, not merely missing fixture setup.
- [ ] **Step 3 — GREEN:** extend existing parsers/attribution rather than creating a second counter; preserve source types and null states, and separate elapsed wall time from overlapping tool work.
- [ ] **Step 4 — Confirm GREEN:** run complete Python benchmark suite; relevant `cargo test -p davinci-ai --lib --offline --locked` and `cargo test -p davinci-agent --lib --offline --locked` if Rust was touched.
- [ ] **Step 5 — Approved commit:** `git commit -m "fix: preserve complete and nullable inference receipts"`.

**Acceptance:** One run yields internally consistent per-attempt totals and an auditor can prove exactly what remains unavailable.

### Task 4: Run controlled requirement-review ablations before modifying policy

**Files:** Create `docs/perf/codex-efficiency/requirement-review-decision.md`; add tests only if a current source bug is established: `crates/davinci-agent/tests/completion_requirements.rs`, `crates/davinci-agent/src/completion.rs`, `crates/davinci-coding-agent/src/settings.rs`.

**Interfaces:** Existing `requirementReview` / `DAVINCI_REQUIREMENT_REVIEW` is the treatment flag. Compare: enabled (control), disabled (diagnostic), and a proposed narrower policy **only if** the evaluator reveals a safe omission class. `autoVerify` and completion hooks stay unchanged.

**Starting point (measured 2026-10-22 from existing data, no new runs):** the 62.8% figure predates `8a87a98e` (2026-10-12, "skip the requirement review when a check passed on the latest change"), which shipped the diagnosis's option 1. In the post-fix Oct 17 public-task campaign (`C:\bench\pubcmp-20261016\rep{0,1,2}\results.jsonl`, 24 DaVinci runs, print mode, binary with the skip), a gate reminder fired in 9 of 24 runs; `requests_after_first_reminder` sums to 58 of 195 requests (30%); 3 of those 9 runs edited afterwards; 8 of the 9 passed. In all 9 runs **both** `completion.requirements` (`completion.rs:373`) and `verification_required` (`turn.rs:1213`, harness re-ran the last check and it failed or was inconclusive) fired, so the 30% is not attributable to the requirement review alone. The remaining lever is therefore smaller than the diagnosis reported, and unattributed.

- [ ] **Step 0 — Attribute the residual from existing logs (offline, deterministic):** for every run in the post-fix campaigns under `C:\bench` (pubcmp-20261016 at least), walk `davinci/<task>-r<rep>.stdout.jsonl` in request order and assign each request after the first reminder to the gate that preceded it (`completion.requirements`, `verification_required`, or model-initiated). Output per gate: runs fired, requests, uncached input, mutations, and whether the run's pass/fail changed relative to its pre-reminder state where determinable. The attribution extends the existing `scripts/bench/turn_cache_audit.py` (it already classifies each request's trigger as completion-reminder / verification-notice / retry / tool-results / start) with a per-request `gate` field and a per-gate summary; tests live in `scripts/bench/tests/test_turn_cache_audit.py`. The counts are computed by the script, never estimated by hand. **If requirement-review requests are under 10% of total requests, close Tasks 4 and 5 except RED case (b) and report that the lever was already captured by `8a87a98e`.**
- [ ] **Step 1 — Freeze task contracts:** the 8 public tasks are short, single-purpose and were used in tuning; they cannot show what the review is for. Add multi-requirement tasks (at least 3 explicit acceptance criteria, 2+ files) authored independently, reusing the H1 holdout framework on `origin/bench/holdout-h1-framework` (`scripts/bench/holdout_h1.py`, allowed-files rule). Include: stale check after edit, no tests discovered, failed test, unexpected changed file. No model self-grades.
- [ ] **Step 2 — Offline RED/controls:** require regression tests proving disabled review never alters verification/hook permissions; show that a `verified` check suppresses only the existing review condition while stale/partial/failed states do not.
- [ ] **Step 3 — Candidate policies (from the diagnosis, §4.4, minus the shipped option 1):** (i) raise the trigger threshold (`paths.len() < 2 && requirement_clause_count < 3`) for short single-purpose requests; (ii) a different default for print/JSON modes, only after establishing why `main_tests.rs` already runs print mode with review off; (iii) suppress the requirement review when `verification_required` fired in the same finalization attempt, if Step 0 shows the two gates duplicate each other. Each is one arm; never combine.
- [ ] **Step 4 — Decision gate:** a policy change needs a live A/B (separate approval, predeclared rule: candidate passes ≥ control on the multi-requirement set, requests per verified success ≤ 0.90x). Without it, do not change policy. If a safe class is established, document the exact condition plus test cases for Task 5.
- [ ] **Step 5 — Approved doc commit:** `git commit -m "docs: decide completion review efficiency from evidence"`.

**Acceptance:** No global disable from the historical 62.8% screening result alone; no claims of live success absent authorized actual runs.

### Task 5: Conditional completion-review policy refinement (only after Task 4)

**Files:** Modify `crates/davinci-agent/src/completion.rs`, `crates/davinci-agent/tests/completion_requirements.rs`, potentially `docs/completion.md` and `crates/davinci-agent/src/stats.rs`; do not create a new standalone reviewer system.

**Interfaces:** Keep `requirement_review_enabled` user override and existing host verification gate. Any new predicate must consume existing verified evidence and mutation generation; return `Review | SkipWithReason`, and expose a stable diagnostic reason without modifying system/tool-schema prefixes. Final function name/type may be integrated privately in `completion.rs` after checking current types; do not introduce a public API solely for the experiment.

- [ ] **Step 0 — Pin current behavior (GREEN on arrival):** `docs/completion.md:9-15` already reviews after an unverified edit, a failed check, a partial check and a stale pass. Confirm `completion_requirements.rs` covers each; add any missing case as a characterization test that passes today. These are not RED tests.
- [ ] **Step 1 — RED:** only tests that fail on the current predicate: (a) the policy change Task 4 justifies, e.g. `skip_when_all_changed_requirements_have_fresh_verified_coverage`; (b) the known safety gap in `docs/completion.md:17-20`, `workspace_check_alone_does_not_suppress_review` (a workspace-wide `cargo check`/`clippy` currently counts as `verified` and suppresses the review even though no test ran); (c) `do_not_skip_multi_file_explicit_acceptance` if Task 4's change could reach that case. (b) tightens review and ships even if Task 4 finds no safe skip class.
- [ ] **Step 2 — Confirm RED:** `cargo test -p davinci-agent --test completion_requirements --offline --locked`, capturing exact failed assertions.
- [ ] **Step 3 — GREEN:** minimal change in `completion.rs`; never treat a plain successful exit, stale test run, or summary as verification for uncovered requirements. Keep at most one review per real prompt.
- [ ] **Step 4 — Confirm GREEN:** focused tests plus agent lib suite, verification/hook/permission regressions, full offline benchmark tests.
- [ ] **Step 5 — Approved commit:** `git commit -m "perf: avoid redundant verified completion reviews"`.

**Acceptance:** Explicit quality preservation evidence or revert/no-op. If Task 4 finds no safe skip class, only RED case (b) ships.

---

## Workstream B — Context VM / prompt-cache compatibility

### Task 6: Add a four-mode structural context matrix at real boundary transitions

**Files:** Create `scripts/bench/context_mode_matrix.py`, `scripts/bench/tests/test_context_mode_matrix.py`; use/extend existing `crates/davinci-agent/tests/context_vm_efficiency.rs`, `tests/context_vm_hybrid.rs`, `tests/context_fold_chunks.rs`, `crates/davinci-evals/src/behavior/context_lifecycle.rs` and `crates/davinci-coding-agent/src/ws_wire_diagnosis_tests.rs`.

**Interfaces:** `build_mode_matrix(records: list[dict]) -> dict` groups by frozen `mode, fixture, phase, restart, branch, model, provider` and reports structural longest-common-prefix bytes, compaction count, summarizer calls/bytes, required source recovery, and known provider cache observations. Distinguish `estimated_prefix_reuse` from `provider_cache_read_ratio`.

- [ ] **Step 1 — RED:** tests for unchanged prefix before first compaction, Off pruning at 65% versus Hybrid no-prune, 2+ compactions, a hot-window restart, cold resume, branch, unavailable summarizer, large late requirement and changed system/tool schema.
- [ ] **Step 2 — Confirm RED:** `python -m unittest discover -s scripts/bench/tests -p 'test_*.py'`; confirm targeted matrix tests exercise transition data rather than constant fixture assertions.
- [ ] **Step 3 — GREEN:** consume existing wire/usage and lifecycle fixtures; reuse current identity and diff logic; do not send provider calls and do not change provider request ordering.
- [ ] **Step 4 — Confirm GREEN:** `cargo test -p davinci-agent --test context_vm_hybrid --offline --locked`; `cargo test -p davinci-agent --test context_fold_chunks --offline --locked`; `cargo test -p davinci-evals --lib context_lifecycle --offline --locked`; relevant coding-agent WS tests.
- [ ] **Step 5 — Approved commit:** `git commit -m "test: compare Context VM modes at compaction boundaries"`.

**Acceptance:** Each mode's actual behavior is observable, with no claim that offline structural reuse equals provider KV hits.

### Task 7: Protect native replay, provenance and cache identity across mode switches

**Files:** Inspect/modify only if a regression is found: `crates/davinci-agent/src/prepared_context.rs`, `crates/davinci-agent/src/runtime/cache/identity.rs`, `crates/davinci-agent/src/runtime/context_vm/fold.rs`, `crates/davinci-ai/src/openai_cache_policy.rs`; tests in `crates/davinci-agent/tests/context_page_cold_load.rs`, `context_vm_hybrid.rs`, `context_fold_chunks.rs` and existing `crates/davinci-ai` cache fixtures.

**Interfaces:** No new wire API. Identity fingerprints hash the exact provider-visible instruction/tool contract; conversation/transport identity remains independent. Compacted sources are retrievable via `retrieve_context` and completed tool/result pairs preserve their IDs.

- [ ] **Step 1 — RED:** fixtures for changed tool grammar, permission narrowing, different session/worker role, unknown native Responses item, partially streamed tool result, source file changed across branch, image reference after repeated compaction and storage failure.
- [ ] **Step 2 — Confirm RED only where a genuine defect exists:** source fixtures already covered by current tests should remain GREEN and be documented; do not manufacture a failing test by changing correct semantics.
- [ ] **Step 3 — GREEN if needed:** one minimal fix in the current owner; conservative fallback on missing/corrupt evidence, never cross-project or cross-session reuse, and no private Codex cache flags from public Responses profiles.
- [ ] **Step 4 — Verify:** focused cache/context tests, branch/cold-resume tests, native wire replay fixtures; record explicit zero-change result if the code passes all assertions.
- [ ] **Step 5 — Commit only if code changed:** `git commit -m "fix: guard cache identity and Context VM recovery"`.

**Acceptance:** No permission, provenance, replay or recovery regression. A test-only outcome is a successful outcome; no speculative production change required.

### Task 8: Decide mode default using one-variable, authorized live comparison

**Files:** Create `docs/perf/codex-efficiency/context-mode-live-decision.md`; integrate `scripts/bench/subscription_campaign.py`, `scripts/bench/bench.py`, `scripts/bench/campaign.py`, Task 2 and Task 6 analyzers. No new provider route or cache protocol fields.

**Interfaces:** Paired arm settings differ only in `contextVm`: Off / Hybrid / Active (Shadow diagnostic). Keep fixed model, effort, tool surface, prompt profile, endpoint, task seed, auth and deadlines. Measure cold/warm cache sequences and long sessions crossing at least two real compactions.

- [ ] **Step 1 — Offline prerequisite:** require Task 1–7 reports plus exact source/tool hashes, clean build, 0 safety regressions and a manifest with time/task/request caps.
- [ ] **Step 2 — Authorization prerequisite:** obtain explicit operator approval before sending a single real Codex request; run only through the existing subscription campaign supervisor and root budget. Fail closed on missing budget, OAuth, quota, unknown accounting, or transport failure. No API-key spend.
- [ ] **Step 3 — Pair runs:** use fresh isolated worktrees; record independent verifier outcome, all auxiliary model attempts (folds included), uncached input, output, cache reads, latency and actual compaction/resume markers; retain failed runs in denominators.
- [ ] **Step 4 — Evaluate:** use predefined thresholds, confidence intervals and holdout; distinguish cold prefix sharing from within-session cache reuse. Do not infer subscription allowance from discounted API token prices.
- [ ] **Step 5 — Decision:** recommend Hybrid default **only** if verified success/recovery and efficiency gates pass; otherwise keep Off default and document why. No automatic default change in this task.

**Acceptance:** A reviewer can trace each conclusion back to provider receipts and independent grading; offline-only evidence yields `NO-GO: provider effect unknown`.

---

## Workstream C — Retrieval and deterministic local reuse

### Task 9: Benchmark existing repository retrieval before tuning injection

**Files:** Inspect/possibly extend `crates/davinci-coding-agent/src/native_extensions/repo_intelligence/`, `native_extensions/vector_memory.rs`, `docs/vector-memory.md`, `docs/repo-intelligence.md`; create `scripts/bench/retrieval_ablation.py`, `scripts/bench/tests/test_retrieval_ablation.py`.

**Interfaces:** `rank_retrieval_case(case: dict, mode: str, token_budget: int) -> dict` accepts a frozen query, code anchors and expected file/symbol spans. `mode` in `{lexical, semantic_if_available, hybrid}`. Emits precision/recall@k, stale/unauthorized candidates, exact-code-span coverage and injected context-token estimates. This is evaluation code, not a second production retriever.

- [ ] **Step 1 — RED:** seed cases with exact symbol/path/error, synonymous natural-language description, relevant test hidden behind dependency edges, stale content hash, revoked path permission and missing Ollama. Assert exact lexical matching still works without embeddings.
- [ ] **Step 2 — Confirm RED:** focused Python tests; document any existing retrieval feature that already passes and do not replace it.
- [ ] **Step 3 — GREEN:** invoke existing repo intelligence/vector-memory interfaces or captured fixture outputs; select exact spans with deterministic reranking and compare injected tokens. Do not copy whole repository or full old logs into the provider prompt.
- [ ] **Step 4 — Confirm GREEN:** all Python tests and relevant `cargo test -p davinci-coding-agent --lib --offline --locked` retrieval filters, including unauthorized/missing-service cases.
- [ ] **Step 5 — Approved commit:** `git commit -m "test: add code-retrieval quality and budget ablations"`.

**Acceptance:** A retrieval recommendation names the cases improved and harmed, and demonstrates no code-anchor loss or unauthorized injection.

### Task 10: Reuse existing EngineeringSnapshots/CacheRuntime for one proven repeated artifact

**Files:** Inspect `crates/davinci-coding-agent/src/native_extensions/engineering_snapshot.rs`, existing `native_extensions/repo_intelligence/`, `crates/davinci-agent/src/runtime/cache/memory.rs`, `crates/davinci-agent/src/runtime/cache/persistent.rs`; extend `crates/davinci-coding-agent/tests/engineering_snapshot.rs` or an adjacent test. Update `docs/perf/codex-efficiency/local-reuse-decision.md`.

**Interfaces:** A candidate cacheable result is restricted to read-only deterministic operations (e.g. symbol index, file dependency graph or test discovery). Key comprises `workspace identity + path/content digest or Git tree + operation + relevant tool/config version + permission scope`. Return cannot grant authorization; recheck permission and freshness at consumption. Use the existing cache request/dependency types instead of creating a new durable cache API.

- [ ] **Step 1 — Prove repetition:** use existing repeatedRead/repeatedSearch and EngineeringSnapshots counters to demonstrate duplicated work in the same workspace state; if no material repeated work exists, close this task as `NO NEW CACHE`.
- [ ] **Step 2 — RED:** tests `same_immutable_input_reuses_result`, `changed_file_hash_invalidates`, `changed_permission_scope_rejects`, `branch_change_invalidates`, `corrupt_persistent_entry_rebuilds_safely`, `concurrent_request_only_computes_once` where current cache semantics guarantee single-flight.
- [ ] **Step 3 — Confirm RED:** `cargo test -p davinci-coding-agent --test engineering_snapshot --offline --locked`; record the behavior actually missing.
- [ ] **Step 4 — GREEN:** integrate with the existing appropriate snapshot/cache owner, preserve isolation and bounded memory/eviction limits, and return ordinary cache misses on unavailable or stale records. No caching of generated patches, live git status or mutable test results.
- [ ] **Step 5 — Verify and approved commit:** same focused test plus other cache/store tests, then `git commit -m "perf: reuse validated repository evidence"` only if positive.

**Acceptance:** Repeat work saved is measurable and no invalidated/unauthorized artifact can appear as valid evidence.

---

## Workstream D — Verify first; selectively consider other optimizations

### Task 11: Bound repair prompts around actual failing evidence

**Files:** Inspect/modify only if needed `crates/davinci-agent/src/verification.rs`, `crates/davinci-agent/src/verification_notice.rs`, `crates/davinci-agent/src/completion.rs`, `crates/davinci-coding-agent/src/runtime_host.rs`; add focused existing verification or integration tests rather than new reviewer infrastructure.

**Interfaces:** On a concrete failed test, repair input retains exact relevant assertion/exit code, changed file diff, required user constraints and recoverable source refs; long log bodies remain obtainable via `retrieve_output`. Verification outcomes require host-owned fresh receipts. Existing root budget limits all additional model requests.

- [ ] **Step 1 — RED:** fixtures for 100 KiB failing log, repeated identical failure, changed source after check, no tests discovered, zero tests reported as pass, malicious tool output and interrupted verification.
- [ ] **Step 2 — Confirm RED:** run focused verification and completion tests; prove which narrow context or safety problem is real before editing.
- [ ] **Step 3 — GREEN if justified:** reduce only redundant error context, not exact diagnostics; reuse Token Governor and the existing no-progress repair stop; never add an unconditional self-critique call.
- [ ] **Step 4 — Verify:** existing receipt, mutation, cancellation and replay tests; inspect failed-case trace for source recoverability and unchanged authorization.
- [ ] **Step 5 — Approved commit:** `git commit -m "perf: focus repair evidence without hiding failures"` only with demonstrated gains.

**Acceptance:** Fewer repeated tokens (if any) with unchanged valid failures, zero false-green results, and exact evidence recoverable.

### Task 12: Optional Lean/tool-profile and adaptive-effort screening — no default change

**Files:** Create `docs/perf/codex-efficiency/tool-and-effort-decision.md`; reuse `docs/tool-discovery.md`, `docs/perf/request-efficiency/A1.md`, existing `ToolSurface::Lean`, `crates/davinci-agent/src/effort.rs`, and `scripts/bench/bench.py`.

**Interfaces:** Fixed model, task and prompt. Independently vary `{toolSurface: full|lean}` and `{effortPolicy: fixed|adaptive}`; do not combine variables in the primary paired comparisons. Record schema tokens, newly discovered tool calls, expanded operations, total requests, token counts, verified success and real latency.

- [ ] **Step 1 — Snapshot known negative:** preserve the September adaptive result (more uncached input and observed task regressions); an unaltered candidate must not be promoted from a faster median alone.
- [ ] **Step 2 — RED:** offline tool-discovery fixtures for an undiscovered necessary specialist tool, permission revocation, search failure and repeated discovery; no schema/permission broadening.
- [ ] **Step 3 — Confirm and evaluate:** run existing tests and offline benchmark arms, followed only by separately approved live screening if warranted.
- [ ] **Step 4 — Decision:** keep Full/fixed defaults absent complete quality evidence. No new router/model selector and no automatic high-effort escalation just because a tool returned a transient error.
- [ ] **Step 5 — Commit decision doc if approved:** `git commit -m "docs: record tool-surface and effort ablation decisions"`.

**Acceptance:** Controlled one-variable comparison; current fixed policy remains baseline.

### Task 13: Freeze release gates, rollback and subscription-use claims

**Files:** Create `docs/perf/codex-efficiency/promotion-decision.md`; extend `docs/cache/openai-cache-benchmark.md` only for a verified missing report criterion; use `scripts/bench/subscription_campaign.py`, existing root budget and release fixtures.

**Interfaces:** `GO` requires independently verified task success, complete attempt accounting, unchanged permission/isolation/recovery, real endpoint/model consistency, credible improvement under predeclared thresholds, and independent reviewer signoff. `NO-GO` includes missing evidence, incomplete usage that affects comparisons, crash/stream failure, unidentified tool mutation, over-budget run, or unauthorized route.

- [ ] **Step 1 — RED:** negative fixtures for no live approval, expired OAuth, API fallback temptation, unknown cached tokens, negative cache-write arithmetic, provider stream cut, aborted root budget reservation, all-skipped tests and worker from another session.
- [ ] **Step 2 — Confirm RED:** existing offline subscription policy/root-budget/bench tests should reject forbidden cases; new failures require minimum owner-local fixes before considering speed features.
- [ ] **Step 3 — GREEN:** generate one decision report from Task 2 with baseline/treatment hashes, all trial outcomes, excluded pairs, confidence and exact reasons. Preserve rollback switches and data/schema migration compatibility.
- [ ] **Step 4 — Verification:** full format/Clippy/locked tests, frozen production binary, isolated process cleanup, repeatable measurement and privacy redaction; do not claim CI success unless actual CI ran.
- [ ] **Step 5 — Owner decision:** `GO` or `NO-GO` with one rollback target; do not deploy or flip defaults as part of this document-generation task. An authorized later PR/release can do that separately.

**Acceptance:** A reviewer can veto a candidate from the report without interpreting private tool logs or trusting assistant claims.

---

## Sequencing, dependencies and independent PRs

| Priority | Independently reviewable delivery | Tasks | What gates the next stage |
|---|---|---|---|
| 1 | Backend route usage smoke (≤5 approved requests) | 0 | `ROUTE_REPORTS_USAGE` or `ROUTE_PARTIAL_USAGE`; `ROUTE_NO_USAGE` stops the plan |
| 2 | Light baseline + per-success metrics | 1–2 | Frozen source, offline suites green |
| 3 | Completion-review decision and policy | 4–5 | Required-context safety checks + quality gate |
| 4 | Promotion decision for the review change | 13 | Predeclared thresholds, independent signoff |
| Deferred | Request attribution | 3 | Per-gate attribution is done offline in Task 4 Step 0; the rest of Task 3 only if Task 0 or Task 4 shows a receipt gap the existing parsers miss |
| Deferred | Context-mode matrix, replay safety, live mode decision | 6–8 | Evidence that compaction is a material cost on real tasks |
| Deferred | Retrieval and local reuse | 9–10 | Evidence that retrieval or repeated reads cost material tokens or time (Oct 17: tools under 1 s, 98% of wall is provider wait) |
| Deferred | Repair focus, tool/effort screening | 11–12 | Evidence of repeated-failure token waste |

Why this order: completion-time harness turns are the only cost with direct evidence. Before `8a87a98e` (Oct 12) the requirement review drove 61% of uncached input and turning it off cut uncached input 62.8% with no pass drop (`docs/diagnostics/2026-10-efficiency-diagnosis.md:142,172`). After it, gate reminders still precede 30% of requests on the public tasks (Task 4, starting point), split between two gates. Task 4 Step 0 sizes the remainder from existing logs before any code or live run; if it is small, the plan shrinks to Tasks 0–2, RED case (b) of Task 5, and 13. No other workstream has comparable evidence. Deferred tasks are reopened only by a measurement that names them. Run one candidate feature per live comparison to prevent attribution errors. Work happens on a task branch in the main checkout (owner preference, no worktrees); every PR gets review before merging.

Out of scope here but worth stating: on the Oct 17 public tasks DaVinci already used 0.93x Codex's tokens per success; the user-visible gap is 1.81x median wall time, attributed to the public sign-in route generating about 3x slower (`docs/cache/chatgpt-backend-route.md`). This plan does not close that gap; Task 0 is the first evidence on whether the backend route does.

## Reproduction and evidence ledger

Record for every experiment: Git SHA and dirty-tree status, fixture tree hash, binary hash, OS/toolchain, model and effort, Codex backend/transport, Context VM mode, prompt profile, ToolSurface mode, requirementReview and autoVerify settings, native replay/cache switches, cold/warm status, all provider attempt IDs and purposes, unknown usage, exact task verifier results, total wall time, source retrieval state, and root-budget policy. Do not publish prompts or secrets.

**Provider-free checks at preflight:**

```bash
python -m unittest discover -s scripts/bench/tests -p 'test_*.py'
cargo test -p davinci-agent --test completion_requirements --offline --locked
cargo test -p davinci-agent --test context_vm_hybrid --offline --locked
cargo test -p davinci-agent --test context_fold_chunks --offline --locked
cargo test -p davinci-evals --lib context_lifecycle --offline --locked
cargo test -p davinci-coding-agent --test engineering_snapshot --offline --locked
```

The actual project toolchain, compiler availability, platform and existing failures determine whether these can be executed; recording a command here is **not** evidence it passed. Python tests run only through `discover -s scripts/bench/tests` (no `__init__.py`; modules import via `sys.path.insert`); narrow with `-p 'test_<name>.py'`.

**No paid experiment by default.** Before any approved live run, use the repo's existing subscription policy, hard maximum requests/tasks/time, same model/effort, OAuth-only auth, isolated grading and secrets, and root-budget ledger. A consent failure or unknown provider attempt blocks the send rather than rerouting to a different billing surface.

## Promotion metrics and decision table

| Metric | Baseline definition | Candidate rule |
|---|---|---|
| Verified success rate | All independent task attempts in denominator | No unexplained regression; qualified holdout must pass |
| Uncached input per verified success | Sum noncached provider input across all relevant attempts, divided by independently verified successes | Aim >=10–15% reduction or another predeclared material benefit, paired and uncertainty-reported |
| Requests per verified success | Every logical/model request including auxiliary folds/reviews | Directional reduction without compensating failures |
| Prefix reuse | Offline longest common prefix, labeled estimate | Diagnostic only; never substitute for cache read |
| Provider cache read ratio | Actual reported cached read / raw reported input when complete | Report separately; missing means unavailable |
| Real subscription allowance | Account window snapshots + enforceable provider product counters if available | Do not claim longer quota from API token totals alone |
| Latency p50/p90 | Whole task wall, failures separately retained | Proposed p90 regression ceiling <=10%; specify valid sample size |
| Safety/recovery | Permission, stale source, native tool pair, branch, cold resume, cancellation | Zero new failures, unknowns block promotion |

**STOP** when evidence is missing, task success regresses, capability probing requires unauthorized spend, requested model/backend changes, accounting becomes ambiguous, or execution could mutate outside the approved workspace. Roll back via existing documented switches and previously tested build; do not discard acknowledged tool output or session entries.

## Plan self-review and handoff checklist

- [ ] Source preflight matches these path names/types and records any default-branch drift before implementation.
- [ ] Every production change is tied to a failing regression or complete measured need; existing features are reused.
- [ ] Both report recommendations and repository-specific counterevidence are present; no false claims of missing features.
- [ ] Cache-mode results include real fold boundary/resume, not only append-only steady-state wire equality.
- [ ] Account usage, cached tokens, logical request counts, provider attempts, verification results and memory cost remain separate.
- [ ] All negative-input classes in **Review Focus** have targeted tests in their owning tasks.
- [ ] Explicit owner review approves scope and, separately, any paid/live budget and implementation method.

**Recommended execution method:** Sequential, one task at a time on a branch in the main checkout, one implementing agent per task and one independent cold reviewer per PR. Task 0 and any live run need a separate named approval. No implementation, commit, push or paid call should occur solely because this plan exists.
