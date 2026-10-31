# 06 — Coding-task success, completion verification and evaluation machinery

Status: evidence record. No code changed for this subsystem in this audit.
Research basis: F-40-01…07, F-41-06/07, F-15-01/02, F-16-01…04, F-39-01…05.
Research reports: `40-effectiveness-and-quota.md`, `41-token-cache-effectiveness.md`,
`15-performance.md`, `16-token-and-cost.md`, `39-telemetry.md`.

The research corpus does **not** establish coding-task success, pass@k, verification
rate, false-claim rate, token cost per task, provider cache hits or subscription
savings for Claude Code (F-40-05…07, F-41-06/07). This document therefore records what
DaVinci can measure, not a comparison.

## 1. The interactive completion gate is advisory

Path: `turn.rs:1131-1242`. Order: requirement review (`:1132`), capability evidence gate
(`:1136-1164`), mutation-verification reminder (`:1166-1237`), completion hook
(`:1239`).

The reminder branch runs at most once per mutation generation:

```rust
if matches!(completion_evidence, Unverified | VerificationFailed)
    && verification_reminded_generation != Some(mutation_generation)
```
`turn.rs:1169-1173`

On the next finish attempt the condition is false and the turn ends with a host-side
notice (`:1311`, `emit_verification_notice`). Consequences:

- a text-only answer with no mutations is never checked
  (`lib.rs:1760-1762`, `mutation_generation == 0` → `NotRequired`);
- `Verified` requires a generation match, a successful run, `verified_generation`, and
  `Targeted|Broad` coverage (`lib.rs:1770-1785`);
- the capability gate gives up after `MAX_CAPABILITY_COMPLETION_REMINDERS = 2` and
  returns `AllowCompletion` (`prompt/capabilities/gate.rs:65,70-84`);
- the requirement review is skipped for small requests
  (`paths.len() < 2 && requirement_clause_count(request) < 3`, `completion.rs:309`), and
  further edits after the review do not re-review it (`docs/completion.md:78-83`);
- the notice is a display event, not model-authored text
  (`verification_notice.rs:1-27`).

**Verdict: the gate can delay completion once, never refuse it.**

## 2. Automatic re-verification exists in two narrow places

1. **Agent path**: one re-run of the *last recorded* verification command when
   `auto_verify` is on and the evidence is `Unverified`/`VerificationFailed`
   (`turn.rs:1179-1195`). It re-runs the command it already knows; it does not discover
   new checks.
2. **Graph mode** (opt-in): per-command execution with `passed` requiring at least one
   run and all results verified (`graph/verify.rs:315-385`), simulated-proof detection
   (`:494-517`), and a revision loop bounded by `budgets.max_revision_cycles`
   (`graph/controller.rs:3447-3481`). The standalone verification planner only plans;
   it executes nothing (`verification_planner/mod.rs:1-6`).

`docs/readiness/evaluation.md:97` records graph mode as off by default.

## 3. False-success claims are detected only in evaluation

- Keyword detector: `detect_verification_claims` matches phrases such as "all tests
  pass", "build succeeded" (`davinci-evals/src/behavior/trace.rs:336-358`).
- Freshness: `verified_after_last_mutation` requires verification at or after the final
  mutation generation (`:382-394`).
- Penalties: scenario requirement `NoUnverifiedSuccessClaim` is a hard failure
  (`scorer.rs:208-219`); the `verification_integrity` dimension scores 0/1
  (`:315-327`); suite aggregate `unverified_claim_rate` (`runner.rs:413-420`);
  CI budget 0.005 (`gate.rs:19,72-78`); promotion constant 0.005
  (`promotion.rs:13,95-99`).

Limits: it is a keyword list (a paraphrase such as "the suite is green" is not
matched), and **nothing in the product runtime parses model prose** — the runtime only
sees receipts and mutation generations.

## 4. The oracle harness is real

`behavior/executor.rs:116-277`: run baseline reproducers and fail if one already passes
(`:138-145`), run the real product process (`:164`), then independently re-verify
(`:209-228`) with workspace diffing (`:205-207`). Commands are sandboxed: no shell
operators, workspace-relative, executable allowlist (`behavior/verification.rs:23-79`).
The Codex A/B oracle refuses to treat assistant text as verification
(`codex_eval.rs:97-133`) and rejects a claimed success when the oracle fails (test
`:510-523`).

**The harness is trustworthy; its corpus is not (next section).**

## 5. The offline corpus cannot produce a meaningful "verified success"

Verified directly in this audit:

| Fact | Command / file | Result |
|---|---|---|
| Scenarios in `core-200.json` | PowerShell `ConvertFrom-Json` | 200 |
| Scenarios with empty `verification_commands` | same | **200 / 200** |
| Scenarios using `repo_fixture: core-repo` | same | 121 / 200 |
| Files in `fixtures/repos/core-repo` | `Get-ChildItem -Recurse -File` | only `README.md`, 21 bytes |

The loader substitutes `git diff --check` for a mutating scenario with no verification
command (`scenario.rs:108-120`). So for these 200 scenarios, "verified success" means
"the diff has no whitespace errors". Scenario `explore-001` additionally requires
`read_before_edit` and `file_changed` on `src/config.rs`, a file that does not exist in
`core-repo`.

Other corpus defects (from the subsystem audit):

- `fixtures/behavior/{verification,exploration,scope,tools,review}/*.json` use an
  obsolete schema (`required_events`/`forbidden_events`) and reference a fixture that
  does not exist; no code references them.
- `debugging-hard.json` has 51 scenarios, `long-horizon.json` has 30.

## 6. No campaign has been run

- `docs/readiness/evaluation.md:3-7`: the twelve public tasks remain smoke tests, no
  live baseline is asserted.
- `:100-106`: result cells are "not run" until campaign directories, binary hashes and
  commit identities exist.
- `:148-152`: superiority requires ≥150 private holdout tasks and 3 repetitions;
  nothing synthetic satisfies it.
- `docs/completion.md:108-113`: acceptance requires paired private dev-split and
  holdout campaigns; no accuracy or cost improvement is claimed.
- CI runs only the offline unit tests of the suite (`behavior-evals.yml:33-37`); the
  live workflow is manual and secret-gated (`behavior-live.yml:28`).
- No `results.jsonl`, `summary.json` or `campaign.json` exists anywhere in the checkout.

## 7. Cost per verified success exists as a metric, with no values

`davinci-evals/src/optimization.rs:34-90`: `VerifiedRunMetrics`, and
`cost_per_verified_success` = total cost of **all** runs / number of verified
successes, so failed-attempt spend is included; any unknown cost returns `None`, never
0 (`:69-89`). It is re-exported and unit-tested but not wired to the behavior runner.
The Python bench tooling has the same metric in `scripts/bench/readiness_metrics.py:135-138`
and `campaign.py:428-438`; `docs/readiness/evaluation.md:110-122` states that failed
spend must be counted.

## 8. Crash and interruption honesty is strong

- Effects on the current branch whose mutation state was not recorded are treated as
  unresolved (`lib.rs:1478-1549`); `invalidate_for_interrupted_effects` bumps the
  mutation generation, clears `verified_generation`, coverage and evidence, and sets
  `unknown_mutation_paths` (`lib.rs:1551-1591`).
- Restoration fails safe when saved state is unreadable (`lib.rs:1721-1750`).
- Unpaired calls get "outcome unknown" or "not executed"; orphan results are refused
  (`turn.rs:330-474`).
- Tests: `agent_tool_resume_integration.rs:229,347`, `lib_tests.rs:6369,6430,6514,2995`,
  `session_persistence_tests.rs:448`, `operation_properties.rs:363`.

This is the area where DaVinci is strongest, and it is the part of the MCP fix in this
audit that was already correct on the journal path.

## 9. Requirement structure and coverage

- The interactive agent keeps prose plus a clause count; its own test asserts it does
  not infer requirements (`completion.rs:1195-1212`). The reminder asks the *model* to
  enumerate requirements (`:334-336`).
- Structured machinery exists in the runtime-task subsystem: `completion_allowed`
  requires every required dimension passed (`runtime/completion.rs:12-17`),
  `verification_requirements` live on the contract, and acceptance packs check per
  requirement ids and per check receipts (`verification/acceptance.rs:19-45`,
  `completion_delivery.rs:104-178`). It is not fed by the interactive path.
- Coverage is per path and conservative: a suite covers the directory it ran in plus
  what the assessment claims, and a changed dependency is covered without covering
  its unexecuted tests or siblings (`tests/inline_verification.rs:63-86`, `:233`).
- Dependency/impact tools (`test_impact`, `change_impact`, `build_intelligence`) are
  model-invoked queries; no runtime call from the completion gate
  (`grep` over `crates/davinci-agent/src`).

## 10. Subagents are gated but their claims are not verified

See `04-ORCHESTRATION-SUBAGENTS-WORKTREES.md` §10. The delegation ban is deterministic
(`delegation.rs`, enforced at `turn.rs:2811-2816`), caps and memory admission exist, and
a root budget is immutable (`root_budget.rs:48-52`) — but a worker's "success" is exit
code plus non-empty text.

## 11. What can and cannot be claimed today

| Claim | Status |
|---|---|
| "The completion gate cannot be silently bypassed by the model" | **false** — the model can finish after one reminder |
| "A modified workspace is never reported as verified without a matching receipt" | true for the interactive gate's own evidence model |
| "Interrupted effects are never promoted to success" | true, tested |
| "The eval harness scores verified success with an independent oracle" | true of the machinery |
| "DaVinci passes N% of coding tasks" | **not established** |
| "DaVinci costs less per verified task than X" | **not established** |
| "Cache hits reduce subscription spend" | **not established** |

`08-CODING-SUCCESS-AND-SUBSCRIPTION-METRICS.md` defines what a campaign would have to
produce before any of the last three can be stated.
