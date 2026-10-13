# Universally failing task families

The report's 324-run population, excluding smoke and an incomplete earlier
campaign, has 27 attempts per family below and zero overall/grader passes.
These are existing observations, not newly executed benchmarks.

Frozen contracts live under
`recovery-runner-src/scripts/bench/tasks/<family>` in the local benchmark root.
Saved reference/starter checks reside under `fixture-validation-01`,
`fixture-validation-02`, and `fixture-validation-03`, each in `_validate`.
Reference solutions passed 6, 8, 6 and 8 tests respectively; starters failed.
That proves satisfiability against references, not prompt clarity or private
holdout integrity.

| Family | Evidence | Assessment |
|---|---|---|
| `m1-config-parser` | `task.json:11`, `repo/README.md:3`, `hidden/test_hidden_config.py:7–9`, `solution/config_parser.py:5–7` | All 27 fail BOM/comment case. Prompt discusses UTF-8/comments/whitespace, not BOM acceptance; reference strips BOM. Clarify intended encoding policy. |
| `m2-public-api-migration` | `task.json:16`, `repo/userlib/core.py:1–4`, `hidden/test_hidden_api.py:8–12,26–28,31–42`, `solution/userlib/core.py:1–10` | All 27 fail new-format, whitespace and stale-import cases. Hidden trimming/rejection and name-only uppercase differ from starter policy. Static scan also rejects a literal compatibility import in tests despite promised shim. Clarify scope/format contract. |
| `m3-state-persistence` | `task.json:12`, `hidden/test_hidden_state.py:30–49`, `solution/state_core/model.py:1–8` | Empty-delta rejection fails in 25/27; atomic rollback in six; scope/order in one. Empty mapping can reasonably mean no-op under the underspecified prompt. Some separate persistence failures are genuine. |
| `m4-cli-feature` | `task.json:12`, `repo/cli.py:9–11`, `repo/tests/test_public_cli.py:4–6`, `solution/tests/test_public_cli.py:4–6`, `hidden/test_hidden_cli.py:29–31` | Direct contradiction: starter public test requires `total 10 2` → `12.00`, reference replacement/hidden require `20.00`, while prompt promises legacy preservation. All 27 fail legacy-total. |

Baseline grader references are `<harness>/<family>-r0.grader.json:15–18`
under `luna-high-recovery-02/vm-off-cm-off+codex`, for both harnesses.

## Proposed contract repair, not an in-place grader relaxation

Preserve the current campaign and frozen artifacts. Create a new versioned task
set only after the owner selects intended semantics:

1. For m4, choose whether the existing addition command remains legacy or
   whether pairwise multiplication is explicitly a breaking migration. Align
   prompt, starter, public tests, reference and hidden tests to that choice.
2. State m1 BOM policy explicitly. State m2 whitespace/format/compatibility-scan
   scope explicitly. State m3 empty-delta validity explicitly.
3. Validate starters fail and references pass in the same supported environment.
4. Retain an independently authored holdout, inaccessible to agent processes,
   and separate it from public fixture generators.
5. Record a new fixture identity and report comparability as changed. Do not
   pool corrected-contract results into the old pass denominator.

No benchmark source or saved campaign artifacts were changed in this task.
Choosing intended behavior without an owner decision would be guessing.

## Ranked subsequent proposals

1. **Trustworthy Python discovery and coverage evidence.** Reuse receipt capture
   and the existing verification engine. Demonstrate nonempty discovery from
   actual supervised output, reject spoofed/partial/zero summaries, and bind
   checks to source before evaluating redundant-follow-up reduction.
2. **Versioned benchmark contract repairs and protected holdouts.** Settle the
   m4 contradiction before drawing capability conclusions from universal fails.
3. **Behavioral batching/evidence reuse eval.** Preserve needed reads and
   verification while removing specifically traced duplicate components.
   A single aggregate tool count cannot establish net savings.
4. **Bounded final-request/backend investigation.** Requires separately
   authorized captures/probes; preserve unknown routing/cache lifetime causes.
5. **Feature-specific evals.** Follow [evaluations](evaluations.md): real
   compactions for VM, noisy evidence plus retrieval cost for Governor, and
   matched session behavior for Code Mode.
