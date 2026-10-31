# Stage 0 gap study: pre-registration (P0.4)

**Status:** `pre-registered`
**Date:** 2026-10-31
**Implements:** plan step P0.4 of `docs/superpowers/plans/2026-10-31-droid-gap.md` (v2), under the design
`docs/superpowers/specs/2026-10-31-droid-openai-runtime-parity-design.md`.

**No live run has been made under this document yet.** It contains rules, definitions and the approval
request. It contains no results. Nothing here may be changed after the first live run; a change after
that point is a new pre-registration with a new date, and the earlier runs are labelled as run under the
old one.

## 1. Decisions carried from the plan

These are the plan's values (Decisions D1-D5), quoted, not re-derived.

- **D1 route.** Droid 1.1.0 runs through CLIProxyAPI. DaVinci runs on its production backend route,
  `/login openai-codex`, over WebSocket. Both arms use the same ChatGPT account, model and effort. Class
  `product-system`; the proxy is a recorded confounder (see `docs/competitor-benchmarks.md` for the class
  definitions). A DaVinci-through-proxy arm is rejected: `codex_capabilities.rs:121-162` gives a
  localhost HTTP endpoint no route features, so that arm would not represent DaVinci.
- **D2 arms.** `DaVinci-default` (the user's `/config`, with the Codemode and Context VM modes recorded),
  `Droid-direct` (`--tool-execution-mode direct-only`), `Droid-script` (`--tool-execution-mode
  script-only`).
  - **Task 3 check (`droid-1.1.0-identity.md` section 2):** the 1.1.0 mechanism is not different. The
    hidden `--tool-execution-mode direct-only|script-only` flag still exists, is absent from
    `droid exec --help`, and is validated at runtime (fact 1, `same as 1.0.0`). So D2 stands unchanged.
    A `--settings <path>` per-process file also exists but is not needed to force the mode.
  - `--list-tools` prints the same 30-tool base catalog in every mode, so it cannot show which mode
    ran. Both modes are therefore forced explicitly, and the resolved mode must be recorded from the
    session settings (see gap G2 in section 9).
- **D3 model.** `gpt-6.1-sol`, high effort. For Droid the model is passed as the proxy alias
  `custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0`. Whether that alias resolves to the Script allow-list
  entry `gpt-6.1-sol` is `not re-checked` (identity doc fact 12), which is why both modes are forced.
- **D4 corpus.** Section 2. Known weaknesses are quoted there.
- **D5 budget.** At most 120 live runs for the program unless re-approved; Stage 0 at most 55 runs
  (3 smoke + 51 study + 1 warm-up per harness). Every live step needs its own approval naming runs,
  pins and model. Section 10 carries that approval request, and records a one-run budget discrepancy
  that the approver must resolve.

## 2. Arms, corpus, order

**Arms (3):** `DaVinci-default`, `Droid-direct`, `Droid-script`.

**Corpus (17 tasks):**

| Corpus | Tasks | Grader |
|---|---|---|
| `verified-core` (`crates/davinci-evals/fixtures/behavior/verified-core.json`) | 6 Rust tasks: `verified-range`, `verified-slice`, `verified-parser`, `verified-dedupe`, `verified-errors`, `verified-path` | `cargo test` (`crates/davinci-evals`) |
| Public bench (`C:\bench\pubcmp`) | 8 tasks: `t1-intervals`, `t2-duration`, `t3-lru`, `t4-rename`, `t5-csv`, `t6-bookings`, `t7-cli`, `t8-calc` | the task tests |
| Research fixtures (`C:\Users\sergi\Desktop\lean-droid-1.0\droid-re-research\fixtures`) | 3 tasks: `01-read-many-filter`, `02-bulk-edit-verify`, `03-independent-parallel` | artifact-only `grade.ps1` |

6 + 8 + 3 = 17 tasks.

**Known weaknesses, stated verbatim from the plan:**

1. The public tasks expose graders and solutions (`fixture_secrecy`,
   `docs/perf/codex-efficiency/promotion-decision.md`), so they give direction only.
2. They were used in DaVinci tuning, which biases them toward DaVinci, so a Droid win on them is
   conservative.

Fixtures 04-11 of the research set test Droid-specific Script semantics and are mechanism probes only;
they are not in this study.

**Repetitions:** 1 (`rep` 0). 17 tasks x 3 arms x 1 repetition = 51 study runs.

**Warm-up:** one unscored warm-up request per harness. It is excluded from every metric and from the
pass counts. It exists to absorb cold-start effects (login refresh, WebSocket or proxy connection set-up,
cache creation) and must be run before the first scored run of that harness.

**Order:** the schedule comes from `campaign.py schedule()` (`scripts/bench/campaign.py:59`), which
rotates the first harness across tasks and repetitions so each pair stays adjacent and no arm is
systematically first. The seed (`--order-seed`) is chosen once before the first run, written into the
approval block, and recorded in `campaign.json` for every run. The order is not edited by hand.

**Smoke test:** 3 runs, one per arm, on one task. The smoke test aborts the study if any smoke row is
`INCOMPLETE` (any of `input_tokens`, `cached_tokens`, `output_tokens` or the request count is `None`,
i.e. `usage_available` is not true). Smoke rows are not part of the 51 and are not scored.

## 3. Metrics

The five primary metrics, each defined from a bench row (field names are in section 9):

| Metric | Definition from a row |
|---|---|
| `requests` | `logical_requests`. DaVinci: provider logical requests from the run's request telemetry. Droid: the number of `Streaming result` log lines for that run's session (one line is one request). |
| `input_uncached` | `input_tokens - cached_tokens`, valid only when both are integers and `0 <= cached_tokens <= input_tokens` (the same rule as `campaign.metric("uncached_input_tokens")`). For Droid, `input_tokens` is `inputTokens + cacheReadInputTokens` and `cached_tokens` is `cacheReadInputTokens`, so uncached input equals Droid's `inputTokens`. |
| `input_total` | `input_tokens`. |
| `output_plus_reasoning` | `output_tokens` when the row says `reasoning_included_in_output` is true; otherwise `output_tokens + reasoning_tokens`. If the flag is not true and `reasoning_tokens` is `None`, the value is missing. See gap G3. |
| `wall_ms` | `round(wall_s * 1000)`, the harness-measured wall time of the agent process (grading excluded). |

**Pairing rule:** a task enters a comparison only if both arms of that pair pass it. "Pass" is the
bench row's `pass` field, which is `campaign.task_success(row)`: exit code 0, grader pass, no unrelated
changes, workspace cleanup complete, and no artifact or transaction leak. Failed runs are never dropped
from the pass counts (section 6); they are only left out of the cost ratios, because cost on a failed run
is not comparable.

A metric value that is `None`, non-numeric, negative, non-finite or zero for either arm makes that task
ineligible for that metric in that pair (section 4).

## 4. Statistics

**Pairs (3):**

| Pair | Ratio per task |
|---|---|
| Droid-direct vs DaVinci | Droid-direct / DaVinci |
| Droid-script vs DaVinci | Droid-script / DaVinci |
| Droid-script vs Droid-direct | Droid-script / Droid-direct |

A ratio below 1 means the first-named arm used less of the metric.

**Computation, per pair and per metric (5 metrics x 3 pairs = 15 estimates):**

1. Take the tasks both arms pass (section 3) and whose two metric values are both finite and `> 0`.
2. Per-task ratio `r_i = numerator_i / denominator_i`.
3. Point estimate: geometric mean `exp(mean(ln r_i))`.
4. 90% interval: percentile bootstrap over tasks. The resampling unit is the task (the 17 tasks are the
   only units; there is one repetition). Draw `n` tasks with replacement from the `n` eligible tasks,
   recompute the geometric mean, repeat **10,000** times with a fixed seed written into the approval
   block, and take the 5th and 95th percentiles as the interval.
5. Report `n` eligible tasks for every estimate.

**Missing or zero metrics:** a task with a zero or missing value for either arm in a pair is excluded from
that estimate, and the count of excluded tasks and the reason (missing, zero) is reported next to the
estimate. Values are never imputed, never replaced by a median or a neighbouring arm's value, and a zero is
never replaced by a small constant.

**No multiplicity adjustment and no composite score.** All 15 estimates are reported. No metric is chosen
as "the" metric after the data is seen; the Stage 1 triggers that name a metric (total or uncached input)
read it from this table as it stands. With few eligible tasks the interval is wide and the label will
normally be `NO_CLEAR_GAP`; `n` is printed so the reader can see that.

### Clarifications (readings where the text above is silent; not changes)

These are implemented in `scripts/bench/gap_study.py` and fixed here before any run.

- **Arm of a row:** the harness name (`davinci`, `droid-direct`, `droid-script`). A plain `droid` row
  is assigned by its recorded `droid_tool_execution_mode`.
- **One repetition:** the analysis stops with an error if an arm has more than one row for a task.
- **Rows read:** `results.jsonl` only. `warmup.jsonl` and any row with `scored: false` are never read.
- **INCOMPLETE rows:** a row with `usage_available` not true or `logical_requests` missing keeps its
  pass or fail in the pass counts (a failure is a failure) but is left out of every ratio, counted
  as `incomplete`. A row with `droid_mode_mismatch: true` is left out of ratios the same way, counted
  as `mode_mismatch`.
- **Pairing:** a task enters an estimate only if both arms pass, neither row is excluded above, and
  both metric values are finite and greater than zero.
- **Bootstrap:** a task is drawn with replacement `n` times per resample (`n` = eligible tasks), the
  geometric mean is recomputed, and the interval is the 5th and 95th percentiles of the sorted
  resample means (indexes `floor(0.05 B)` and `ceil(0.95 B) - 1` for `B` resamples). The seed is an
  explicit required input.
- **Few or no tasks:** with `n = 0` there is no estimate and the label is `NO_CLEAR_GAP` with `n = 0`
  printed. With `n = 1` the interval collapses to the single ratio. No fourth label is added.
- **Output plus reasoning:** `output_tokens` when the row says reasoning is included, otherwise
  `output_tokens + reasoning_tokens`. The smoke-row check in G3 is applied with
  `--droid-reasoning-excluded`.
- **Success gap:** needs the human-read cause for each discordant task (`--causes`); without causes
  the analysis reports none.

## 5. Gap rule

| Label | Condition |
|---|---|
| `GAP` | the interval's upper bound is <= 0.85 |
| `AHEAD` | the interval's lower bound is >= 1.15 |
| `NO_CLEAR_GAP` | otherwise |

Applied to each of the 15 estimates, to the 90% interval from section 4.

Reading by pair: for the two cross-harness pairs the ratio is Droid / DaVinci, so `GAP` means Droid used
at most 85% of what DaVinci used (a gap against DaVinci), and `AHEAD` means Droid used at least 115%.
For Droid-script vs Droid-direct, `GAP` means Script used at most 85% of what direct mode used (a Script
effect), and `AHEAD` means Script used at least 115% (Script costs more).

This study describes a measured gap on 17 tasks. It is not a parity or superiority claim: a head-to-head
claim follows `docs/competitor-benchmarks.md` (at least 150 shared scenarios, three repetitions per
harness), which is outside this program's budget.

## 6. Pass guard

Reported for every arm, regardless of the cost labels:

1. Pass count per arm out of 17 (and the pass rate).
2. Every discordant task, between each pair of arms: tasks where one arm passes and the other fails. For
   each, the cause is **read from that run's transcript** (the saved stream
   `<workdir>.stdout.jsonl`, stderr, and the grader output), not inferred from totals. The cause is
   written as a short category, such as stopped early, claimed done without verifying, wrong edit,
   timeout, or infrastructure failure.
3. A **success gap** needs at least 3 tasks that Droid passes and DaVinci fails that share one cause
   (for example stopping early, or claiming done unverified). Fewer than 3 same-cause tasks is not a
   success gap, whatever the pass counts.

This guard is **qualitative, not a significance test.** No p-value or interval is reported for pass
counts, and a difference in pass counts alone does not produce a label.

Runs that stop because of an infrastructure failure (section 8) are not harness results and are not
counted as failures of the arm.

## 7. Usage cross-check

- **Where:** the ChatGPT backend usage windows, read from the `x-codex-*` response headers when
  available, otherwise from the ChatGPT usage page.
- **When:** recorded by hand before and after each arm block (one block per arm per window), with the
  timestamp, and kept with the raw evidence outside the repo under `C:\bench\droid-gap-<date>\`.
- **Purpose:** an independent check that token totals in the rows are the right order of magnitude. A
  window delta is not a per-run measurement and is not used to compute any ratio.
- **Labelling:** token counts for Droid come from Droid's own session settings file and log, relayed by
  the proxy. They are labelled "Droid's own report" wherever shown. They are not backend-verified
  figures.

## 8. Stop rules

1. **OAuth or proxy failure:** stop on any OAuth failure or proxy failure (either arm's login, or
   CLIProxyAPI not answering). The run counts as an infrastructure failure, not as a harness result: it
   is not a pass, not a fail, and not in any ratio.
2. **Exhausted window:** stop when the usage window is exhausted, and resume after the reset.
3. **Run that stops mid-block:** a run that stops mid-block is discarded. The affected arm block is
   re-run after the reset (or after the failure is fixed), starting from its first unfinished position in
   the same pre-computed schedule. Discarded runs are listed in the report with the reason and do not
   count toward the pass guard. They do count against the approved run budget, so the budget is tracked
   per attempted run.

## 9. Field contract

The bench-row fields the study reads, checked against `scripts/bench/bench.py`,
`scripts/bench/droid_usage.py` and `scripts/bench/campaign.py` as they exist after the Task 5 fixes.

| Field | Used for | DaVinci row | Droid row |
|---|---|---|---|
| `harness`, `task`, `rep` | pairing key | set | set |
| `exit`, `grader_pass`, `unrelated`, `cleanup_complete`, `transaction_leak`, `artifact_leak`, `pass` | pass definition (`campaign.task_success`) | set | set |
| `stop_reason`, `inspection_error` | infrastructure-failure classification | set | set |
| `wall_s` | `wall_ms` | set | set |
| `input_tokens` | `input_total`, `input_uncached` | set | Droid `inputTokens + cacheReadTokens`, or `None` |
| `cached_tokens` | `input_uncached` | set | Droid `cacheReadTokens` (1.0.0 name `cacheReadInputTokens` accepted), or `None` |
| `output_tokens` | `output_plus_reasoning` | set | Droid `outputTokens`, or `None` |
| `reasoning_tokens` | `output_plus_reasoning` | set | Droid `thinkingTokens` (1.0.0 name `reasoningTokens` accepted), or `None` |
| `reasoning_included_in_output` | `output_plus_reasoning` | `True` | `True` (hard-coded for every harness; see G3) |
| `logical_requests` | `requests` | set | equal to `requests` (count of `Streaming result` lines) |
| `requests` | cross-check of `logical_requests` | set | set |
| `provider_attempts` | not a study metric | set | always `None` (Droid exposes no retry visibility) |
| `usage_available` | `INCOMPLETE` test | set | true only if input, cached, output and requests are all known |
| `request_metrics_available`, `request_metrics_complete` | request-count completeness | set | both true exactly when `requests` is known |
| `droid_usage_error` | reason a Droid row is `INCOMPLETE` | not present | `None`, or why the session or its usage could not be identified |
| `droid_log_uncached_input` | cross-check only; summed log `count`, never used in a ratio | not present | sum of the log `count` values, or `None` |
| `droid_settings` (`path`, `sha256`) | records the requested tool-execution mode file | not present | set |
| `droid_tool_execution_mode` | which Droid arm a row is (requested mode) | not present | set (`direct-only` or `script-only`) |
| `droid_resolved_tool_execution_mode`, `droid_mode_mismatch` | the mode Droid resolved, and whether it differs; mismatched rows are left out of ratios | not present | set, or `None` when the session file has no `toolExecutionMode` |
| `droid_model`, `droid_auto`, `droid_reasoning_effort` | model alias, autonomy level, and `"not forced"` | not present | set |
| `scored` | `false` only on warm-up rows, which are written to `warmup.jsonl` and never read | `false` on warm-up only | `false` on warm-up only |
| `codemode_mode`, `context_vm` | the DaVinci `/config` modes to record (D2) | set | `None` |
| `execution_mode`, `model`, `effort_policy`, `service_tier`, `binary_sha256`, `campaign`, `fixture_hash` | provenance and pins (campaign rows) | set | set |

An arm row is `INCOMPLETE` when `usage_available` is not true or `logical_requests` is `None`. An
`INCOMPLETE` row is not scored, is not imputed, and aborts the smoke test.

### Parser gaps: status

The first version of this document named G1 to G9. G1, G2, G4, G5, G6 and G7 are now closed in the
parser; G3, G8 and G9 are procedural and unchanged. Nothing below has been run live.

- **G1. Task coverage: resolved in code.** `scripts/bench/task_sources.py` serves the 6
  `verified-core` tasks (`verified-*`) and the 3 research fixtures (`research-01..03`) through the
  same `run_one` path as the public tasks. `--tasks study` selects the 17 tasks; `--tasks all` is
  still the 8 public tasks only. The public tasks still need `scripts/bench/tasks` generated by
  `make_tasks.py`. Allowed-change rules per source are in `scripts/bench/README.md`. The research
  source needs `pwsh` and the fixtures root; it fails loudly without them.
- **G2. Resolved tool-execution mode: resolved in code.** Rows carry
  `droid_resolved_tool_execution_mode` (the session settings key `toolExecutionMode`, or `None`) and
  `droid_mode_mismatch`. A live 1.1.0 session file records the mode as `script_only` / `direct_only`;
  the comparison normalises `_` and `-`, and the raw value is stored. A `None` means "not recorded",
  not "default".
- **G3. Reasoning accounting for Droid: procedural, unchanged.** Apply the smoke-row rule, then run
  the analysis with `--droid-reasoning-excluded` if it says Droid's output excludes reasoning.
- **G4. Arm identity: resolved in code.** The harness names `droid-direct` and `droid-script` pin the
  mode, are accepted by `--harness`, the schedule, identities, the gate and campaign integrity, and
  both use `BENCH_DROID`. Every Droid-family row records `droid_tool_execution_mode`. `gate` now
  requires `davinci` plus at least one of `codex`, `droid`, `droid-direct`, `droid-script`.
- **G5. Model and effort: resolved in code, with one limit that remains.** The Droid model is
  `BENCH_DROID_MODEL` (fallback `BENCH_MODEL`); `--auto` is autonomy from `BENCH_DROID_AUTO`
  (default `high`) and no longer takes `BENCH_EFFORT`. Rows record `droid_model`, `droid_auto` and
  `droid_reasoning_effort: "not forced"`, and the session file's own `reasoningEffort` and `model` are
  recorded as `droid_session_reasoning_effort` and `droid_session_model`, so the effort Droid actually
  used is observed even though it is not forced. **Remaining:** Droid 1.1.0 documents no reasoning-effort
  flag for `droid exec`, so "effort high" is not forced on the Droid model; it follows the proxy model
  configuration, which the bench must not write. The study therefore compares DaVinci at high effort
  against Droid at whatever effort that configuration gives, and the report must say so.
- **G6. Warm-up marker: resolved in code.** `run --warmup` writes `warmup.jsonl` with `scored: false`
  in its own runs directory and never `results.jsonl`.
- **G7. Analysis script: resolved in code.** `scripts/bench/gap_study.py` implements sections 3 to 6
  with the readings in the section 4 clarifications. Its path and SHA-256 still go in the approval
  block once the commit to run is fixed.
- **G8. Usage windows: procedural, unchanged.** Recorded by hand (section 7).
- **G9. Concurrent-session ambiguity: procedural, unchanged.** Runs are sequential with no other Droid
  in use.

## 10. Approval block

Nothing below is approved. P0.5 must not start until the user approves this exact request in writing,
after the gaps G1, G4, G5, G6 and G7 are closed and G3's smoke check passes.

> **Request to run the Stage 0 live gap study (plan step P0.5).**
>
> - **Runs:** 3 smoke runs + 51 study runs (17 tasks x 3 arms x 1 repetition) + 2 warm-ups (one per
>   harness: DaVinci and Droid) = **56 live runs**.
> - **Budget note:** D5 caps Stage 0 at 55 runs, and the plan counts "3 smoke + 51 study + 1 warm-up
>   per harness". With two harnesses that is 56, one over the cap. The approver must either approve 56
>   explicitly, or strike one warm-up (leaving 55). This is not decided here.
> - **Model:** `gpt-6.1-sol` (Droid alias `custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0`).
> - **Effort:** high.
> - **Arms:** `DaVinci-default`, `Droid-direct` (`--tool-execution-mode direct-only`), `Droid-script`
>   (`--tool-execution-mode script-only`).
> - **Corpus:** the 17 tasks in section 2.
> - **Pins:**
>   - Droid 1.1.0, `C:\Users\sergi\bin\droid.exe`, SHA-256
>     `7F23DA2D6D271EDD45F198B946F51A0EA7A779D7C48C7DCE3DDA2C81C52AA775` (Task 3).
>   - CLIProxyAPI, `C:\Users\sergi\AppData\Local\CLIProxyAPI\cli-proxy-api.exe`, SHA-256
>     `11DC49F746325F951096152B6B6499422E26E84D85CB5991023F2AAAC4225029` (Task 3).
>   - DaVinci commit: `<to be filled at approval time: full SHA of the commit being measured>`.
>   - DaVinci binary SHA-256: `<to be filled at approval time: hash of the built davinci binary>`.
>   - Analysis script path and SHA-256: `<to be filled once written (G7)>`.
>   - Schedule seed (`--order-seed`) and bootstrap seed: `<to be filled before the first run>`.
> - **Account and route:** the same ChatGPT account for both arms. DaVinci on `/login openai-codex`
>   (`chatgpt.com/backend-api/codex`, WebSocket). Droid through CLIProxyAPI at `127.0.0.1:8317`
>   (`openai` provider, base URL `http://localhost:8317/v1`). Class `product-system`; the proxy is a
>   recorded confounder.
> - **Preflight to confirm before starting:** `/login openai-codex` has a fresh token and the proxy
>   login is valid; the allowance window is not exhausted (if the account is the one the Codex CLI
>   reported limited until 2026-11-04 01:35, start after that).
> - **Stop rules:** section 8. Raw evidence stays outside the repo under `C:\bench\droid-gap-<date>\`.
