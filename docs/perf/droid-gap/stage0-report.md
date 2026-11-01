# Stage 0 report: DaVinci vs Droid 1.1.0 on 17 tasks (P0.5) and Stage 1 selection (P0.6)

Status: complete for the preregistered analysis. This is a measured gap on 17 tasks, one repetition,
three arms, one machine and one account. It is not a parity or superiority claim (plan, Stage 2;
`docs/competitor-benchmarks.md` needs at least 150 scenarios and three repetitions for that).

Everything below traces to files under `C:\bench\droid-gap-20261031\` (outside the repo, read-only
here) or to the committed docs named in each section. Where a number was computed for this report
rather than copied, the file it was computed from is named and the computation is stated.

## 1. Summary

- Preregistered result (`gap_study.txt`, seed 20261031): DaVinci uses about 2.5x (direct) to 3.3x
  (script) the model requests and about 2.8x to 3.4x the total input tokens of Droid on the tasks both
  pass, and about 1.6x to 1.7x the output plus reasoning tokens. Droid uses more uncached input than
  DaVinci (1.61x for direct, `AHEAD`; script is `NO_CLEAR_GAP`). Wall time is `NO_CLEAR_GAP` in both
  cross pairs.
- Pass counts (task_success): DaVinci 10/17, Droid-direct 12/17, Droid-script 11/17. Two discordant
  tasks (`t5-csv`, `t8-calc`) with different causes. Success gap (at least 3 same-cause tasks): not met.
- Within Droid, Script vs direct is `GAP` on requests only (0.750); input, output and wall are
  `NO_CLEAR_GAP`.
- All 3 research tasks (`research-01..03`) failed for all 3 arms. The cause is a harness defect in the
  research adapter (Section 8), disclosed here; the preregistered result is reported unchanged.
- Stage 1: W3 fires on its letter (requests). W1, W2, W4 and W5 do not fire and are closed as "no
  measured gap". Live follow-ups need separate approval and none was started.

## 2. Pins and environment

| Item | Value | Source |
|---|---|---|
| Date | study 2026-10-31T23:38:40Z to 2026-11-01T00:39:34Z (about 61 min) | `study/results.jsonl` first `started_at`, last `finished_at` |
| Machine | Windows-11-10.0.26300, AMD64 Family 26 Model 68 (Ryzen 7 9800X3D), host `DESKTOP-IC4I6AQ` | `study/campaign.json` |
| Model | `gpt-6.1-sol`, DaVinci effort policy `high` (fixed) | `campaign.json` `model`, `effort_policy` |
| Droid model alias | `custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0` | rows `droid_model` |
| Droid autonomy | `BENCH_DROID_AUTO` = `high` (all Droid rows `droid_auto: high`) | rows |
| Droid reasoning effort | not forced by the bench; the session file records `high` in all 34 Droid rows (`droid_session_reasoning_effort`) | rows |
| Droid binary | `C:\Users\sergi\bin\droid.exe` 1.1.0, SHA-256 `7f23da2d6d271edd45f198b946f51a0ea7a779d7c48c7dce3dda2c81c52aa775` | `campaign.json` identities; `droid-1.1.0-identity.md` |
| CLIProxyAPI binary | SHA-256 `11DC49F746325F951096152B6B6499422E26E84D85CB5991023F2AAAC4225029` | `droid-1.1.0-identity.md` (not re-hashed for this report) |
| DaVinci binary | `C:\bench\droid-gap-20261031\checkpoint\davinci.exe` 1.1.7, SHA-256 `11e6b08f4c51aa3f38edfdf4b605cee64bf21e1ab45f1b046c3e7827c2099a6e`, built from commit `c3868d87b622138f11be22eb64ef04043caddd31` (clean tree, CI run 41246701787 verified) | `checkpoint/davinci.exe.identity.json`; `sha256sum` re-checked |
| Runner (bench scripts) | commit `ad094715b34e000e868967c68700734f4977e232`, clean | `campaign.json` `runner_source` |
| Arms | `davinci`, `droid-direct` (`direct-only`), `droid-script` (`script-only`); resolved modes `direct_only` / `script_only`, no mismatch | rows |
| Order | counterbalanced, `--order-seed 20261031`; bootstrap seed 20261031, 10,000 resamples, 90% | `campaign.json`; `gap_study.json` |
| Route | DaVinci on `openai-codex` (WebSocket); Droid via CLIProxyAPI. Class `product-system`, proxy is a recorded confounder | plan D1 |
| Login | `openai-codex` OAuth token expiry 2026-11-06T23:14:35Z (timestamp only) | `~/.pi/agent/auth.json`, expiry field only |
| Grading isolation | `diagnostic-only` for all arms | `campaign.json` |

The DaVinci binary was built from `c3868d87`; the bench scripts that ran it are at `ad094715`
(which only changed Droid key-name and mode-spelling handling in the bench). The two commits differ
and are both recorded.

## 3. Run ledger and deviations

| Block | Runs | Where |
|---|---:|---|
| Smoke | 3 (`t1-intervals`, one per arm) | `smoke/results.jsonl` |
| Warm-up | 2 (`t2-duration`, DaVinci and Droid-direct), `scored: false` | `warmup/warmup.jsonl` |
| Study | 51 (17 tasks x 3 arms, rep 0) | `study/results.jsonl` |
| Total | 56 | approved by the user (the preregistration flagged 56 vs the D5 cap of 55) |

Checked in the study rows: all 51 exit codes are 0, every stderr file is empty, no row has a
`stop_reason`, an INCOMPLETE flag (`usage_available` is true everywhere), a `droid_mode_mismatch`, or a
`droid_usage_error`, and the three arm counts are 17 each. No OAuth or proxy failure and no stop-rule
event shows in those row fields (the raw provider and proxy logs were not read for this report). The warm-up used `t2-duration`, which is also a study task;
warm-up rows are in `warmup.jsonl` only and are in no table here.

Deviations, stated plainly:

1. **Smoke Droid rows were INCOMPLETE.** The two smoke Droid rows had `usage_available: false`:
   Droid 1.1.0 renamed the usage keys (`cacheReadTokens`, `thinkingTokens`) and spells the mode
   `direct_only` / `script_only`. The rule "abort on INCOMPLETE" applied; the parser was fixed in
   `ad094715` before the study and the smoke rows were re-derived offline with
   `droid_usage.py rederive`. Study rows used the fixed parser.
2. **Usage windows were not recorded per block (preregistration G8).** The `x-codex-*` windows or the
   ChatGPT usage page were not captured before and after each arm block. The token totals here are
   therefore Droid's own report (relayed by the proxy) and DaVinci's provider usage records, with no
   independent window cross-check.
3. **Research tasks 01 to 03 failed for all arms for a harness reason** (Section 8).
4. **`verified-dedupe` and `verified-path`**: every arm passes the grader but is counted as not
   successful, because every arm also edited `tests/dedupe.rs` or `tests/path.rs`, and the corpus sets
   `max_unrelated_files_changed: 0`. The preregistered rule (`task_success` requires no unrelated
   change) was applied as written. This is symmetric across the arms (rows: `grader_pass` true,
   `unrelated` is the test file, `pass` false for all three).
5. **No re-run** of anything was done (no budget). The study ran once.

## 4. The preregistered result (table verbatim)

Output of `gap_study.py ... --seed 20261031` (`C:\bench\droid-gap-20261031\gap_study.txt`; the
committed analysis code reproduces `gap_study.json` exactly, checked for this report):

```
pair                             metric                   n  geo-mean            interval  label
droid-direct vs davinci          requests                10     0.393      [0.341, 0.451]  GAP
droid-direct vs davinci          input_uncached          10     1.612      [1.251, 2.138]  AHEAD
droid-direct vs davinci          input_total             10     0.354      [0.301, 0.416]  GAP
droid-direct vs davinci          output_plus_reasoning   10     0.588      [0.484, 0.739]  GAP
droid-direct vs davinci          wall_ms                 10     1.040      [0.859, 1.237]  NO_CLEAR_GAP
droid-script vs davinci          requests                10     0.307      [0.263, 0.363]  GAP
droid-script vs davinci          input_uncached          10     1.274      [0.919, 1.680]  NO_CLEAR_GAP
droid-script vs davinci          input_total             10     0.298      [0.252, 0.360]  GAP
droid-script vs davinci          output_plus_reasoning   10     0.633      [0.520, 0.798]  GAP
droid-script vs davinci          wall_ms                 10     1.013      [0.812, 1.253]  NO_CLEAR_GAP
droid-script vs droid-direct     requests                11     0.750      [0.669, 0.840]  GAP
droid-script vs droid-direct     input_uncached          11     0.804      [0.508, 1.137]  NO_CLEAR_GAP
droid-script vs droid-direct     input_total             11     0.807      [0.713, 0.915]  NO_CLEAR_GAP
droid-script vs droid-direct     output_plus_reasoning   11     1.053      [0.947, 1.181]  NO_CLEAR_GAP
droid-script vs droid-direct     wall_ms                 11     0.946      [0.794, 1.101]  NO_CLEAR_GAP

pass counts: davinci 10/17, droid-direct 12/17, droid-script 11/17
discordant droid-direct vs davinci: t5-csv: droid-direct passes, davinci fails
discordant droid-direct vs davinci: t8-calc: droid-direct passes, davinci fails
discordant droid-script vs davinci: t8-calc: droid-script passes, davinci fails
discordant droid-script vs droid-direct: t5-csv: droid-direct passes, droid-script fails
success gap (>= 3 same-cause tasks, qualitative): False
```

(The four discordant lines are shortened to the facts; the file also prints `stop_reason=None
cause=(read transcript)` on each. The causes are in Section 6.)

Ratio is the first arm over the second, so a value below 1 means the first arm used less. Labels use
the 90% bootstrap interval (`GAP` upper <= 0.85; `AHEAD` lower >= 1.15). The "droid vs davinci" pairs
use the 10 tasks both pass (n = 10: the 6 public tasks that DaVinci passes plus 4 verified tasks); the
Script-vs-direct pair has n = 11.

### Reading, per pair

- **Droid-direct vs DaVinci.** Droid-direct needs about 0.39x the requests (DaVinci about 2.5x) and
  0.35x the total input (DaVinci about 2.8x) and 0.59x the output plus reasoning. It sends 1.6x the
  uncached input (`AHEAD`): DaVinci's requests are mostly cached, so more requests do not mean more
  uncached tokens. Wall time is not clearly different (1.04, interval 0.86 to 1.24).
- **Droid-script vs DaVinci.** The same shape, larger: 0.31x requests (DaVinci about 3.3x), 0.30x total
  input, 0.63x output plus reasoning. Uncached input is `NO_CLEAR_GAP` (1.27, interval 0.92 to 1.68).
  Wall time is not clearly different (1.01).
- **Droid-script vs Droid-direct.** Script uses 0.75x the requests (`GAP`). Input (0.81), uncached input
  (0.80), output (1.05) and wall time (0.95) are `NO_CLEAR_GAP`.

What this does and does not say: DaVinci makes more, smaller, mostly cached requests to reach the same
passes, and that does not show up as slower wall time or more uncached input than Droid on these tasks.
The request and total-input gaps are real in the data, but their cost on the account is unmeasured
(usage windows were not recorded, deviation 2).

## 5. Per-task table

Source: `study/results.jsonl` (`pass`, `logical_requests`, `wall_s`). "req" is `logical_requests`;
wall is rounded seconds.

| Task | DaVinci pass / req / wall s | Droid-direct pass / req / wall s | Droid-script pass / req / wall s |
|---|---|---|---|
| `verified-range` | PASS / 22 / 69 | PASS / 5 / 31 | PASS / 5 / 47 |
| `verified-slice` | PASS / 9 / 24 | PASS / 6 / 45 | PASS / 5 / 45 |
| `verified-parser` | PASS / 18 / 78 | PASS / 6 / 88 | PASS / 5 / 35 |
| `verified-dedupe` | fail / 15 / 53 | fail / 7 / 57 | fail / 7 / 55 |
| `verified-errors` | PASS / 18 / 58 | PASS / 8 / 59 | PASS / 5 / 50 |
| `verified-path` | fail / 20 / 74 | fail / 7 / 77 | fail / 8 / 89 |
| `t1-intervals` | PASS / 12 / 46 | PASS / 4 / 49 | PASS / 3 / 52 |
| `t2-duration` | PASS / 13 / 61 | PASS / 6 / 64 | PASS / 4 / 63 |
| `t3-lru` | PASS / 13 / 62 | PASS / 6 / 85 | PASS / 5 / 106 |
| `t4-rename` | PASS / 14 / 63 | PASS / 6 / 76 | PASS / 7 / 94 |
| `t5-csv` | fail / 11 / 67 | PASS / 8 / 99 | fail / 6 / 83 |
| `t6-bookings` | PASS / 13 / 49 | PASS / 5 / 49 | PASS / 3 / 49 |
| `t7-cli` | PASS / 18 / 94 | PASS / 6 / 76 | PASS / 4 / 73 |
| `t8-calc` | fail / 14 / 57 | PASS / 8 / 67 | PASS / 4 / 48 |
| `research-01` | fail / 9 / 30 | fail / 5 / 31 | fail / 3 / 31 |
| `research-02` | fail / 17 / 79 | fail / 10 / 406 | fail / 9 / 114 |
| `research-03` | fail / 11 / 66 | fail / 8 / 93 | fail / 5 / 53 |

Descriptive totals over all 17 rows per arm (not preregistered, includes failed runs): requests 247 /
111 / 88, total input 3,390,756 / 1,451,806 / 1,279,494, uncached input 185,252 / 282,526 / 239,494
(DaVinci / Droid-direct / Droid-script), computed from the rows (`input_tokens`, `cached_tokens`).

## 6. Discordant tasks and their causes

Read from the saved streams (`<harness>/<task>-r0.stdout.jsonl`) and grader receipts
(`<harness>/<task>-r0.grader.json`). `stop_reason` is `None` on every row.

### `t5-csv` (Droid-direct passes; DaVinci and Droid-script fail)

- Task: fix `region_totals` for a finance export (BOM header, regions differing in case and
  whitespace, `$` amounts).
- DaVinci failed 0/4 hidden tests (`davinci/t5-csv-r0.grader.json`): the grader expected keys with the
  original capitalisation (`{'West': ...}`) and got lowercase (`{'west': ...}`). DaVinci said "normalize
  region keys to lowercase" and chose lowercase keys. It ran its own five tests and a direct assertion
  against its own expectation (`'north': 1500.50 ...`), all passing, so the verification confirmed its
  assumption and not the requirement.
- Droid-script failed 0/4 for the same reason: it wrote `.strip().lower()` ("keyed by stripped,
  lowercase region names") and verified against its own tests.
- Droid-direct passed: its docstring reads "Return title-cased region names", which matches the hidden
  tests.
- Cause: **wrong fix on an under-specified output convention (key case)**. Two of three arms picked
  lowercase, one picked title case. It is not "stopped early" and not "claimed done unverified": every
  arm ran tests, the tests just encoded the same wrong assumption. DaVinci made 11 requests and Droid-
  script 6, so the failure is not about effort.

### `t8-calc` (both Droid arms pass; DaVinci fails 17/18)

- Task (from the saved stream): "Fix calc.py so arithmetic follows normal precedence and left-to-right
  associativity. Unary minus should also work."
- DaVinci's grader failure: `test_errors[(1 + 2]` "DID NOT RAISE". The baseline `_atom` returns
  `rest[1:]` after an opening parenthesis without checking for `)`, so `(1 + 2` was already accepted
  before the task. DaVinci made minimal edits (loops for `+ -` and `* /`, unary minus in `_atom`) and
  left that unchanged. Its checks were the six visible tests and ten self-chosen cases, none with an
  unbalanced expression.
- Both Droid arms rewrote the parser with explicit errors (`expected closing parenthesis`, `expected
  operand`, `trailing input`, seen in their streams), which also satisfies the hidden error test.
- Cause: **narrow fix verified against the stated and self-chosen behaviours; the hidden grader checks
  an unrequested malformed-input behaviour that the baseline already got wrong, and the Droid rewrites
  happened to cover it.** Not "stopped early"; DaVinci ran tests and diffs repeatedly (14 requests).
  DaVinci did not claim anything it had not run, but its coverage statement ("existing tests cover every
  requested behavior") was about the stated requirements only.

### Success gap

The preregistered rule needs at least 3 tasks that Droid passes and DaVinci fails, sharing one cause.
There are 2 such tasks (`t5-csv`, `t8-calc`) with different causes. The success gap is not met.
This is qualitative, not a significance test.

## 7. Within-Droid Script effect

Script vs direct, from Section 4: requests 0.750 (`GAP`, interval 0.669 to 0.840, n = 11); total input
0.807 (`NO_CLEAR_GAP`, 0.713 to 0.915); uncached input 0.804 (`NO_CLEAR_GAP`, 0.508 to 1.137); output
plus reasoning 1.053 (`NO_CLEAR_GAP`); wall 0.946 (`NO_CLEAR_GAP`). Direct passes 12/17 and Script
11/17; the one difference is `t5-csv` (Section 6), which is not a Script mechanism finding.

Descriptive check on the 10 tasks DaVinci passes (all three arms pass them), from the rows: requests 150
(DaVinci), 58 (direct), 46 (script); median wall 61.4 s, 61.9 s, 51.3 s. The Script requests reduction is
in the data, but wall time is not clearly lower in the preregistered test. On the four verified tasks
both pass, Script/direct request ratios are 1.0, 0.83, 0.83 and 0.63 (`verified-range`, `-slice`,
`-parser`, `-errors`).

## 8. Research-task adapter defect (disclosed; no re-run)

The preregistered result stands unchanged. `research-01..03` failed for all three arms for a harness
reason, independent of the agent.

- **Cause.** The verbatim instruction says "Write workspace/answer.json" and refers to
  `workspace/notes/`. The adapter materialised the fixture at the run directory root and ran the agent
  there. Every arm therefore created a new `workspace/` subdirectory and wrote its artefacts there, and
  `grade.ps1 -Workspace <run dir>` found none (for example `research-01` receipts: "answer.json parses:
  false"). Row evidence: every research row has `workspace/answer.json`, `workspace/edit-report.json` or
  `workspace/results.json` in `unrelated`.
- **Offline diagnostic regrade** (artefacts moved to the fixture root in scratch copies, copying the
  `.git` Hidden attribute as in the originals; the grader and fixtures are unchanged; nothing was run
  against a provider):
  - `research-01`: 4/4 and pass for all three arms.
  - `research-02`: 6/6 and pass for all three arms.
  - `research-03`: 2/3 and still fails for all three arms, for fixture-semantics reasons, not the path
    defect. The fixture's `run-04` writes the seven-digit value `6db26f8`; all three arms zero-padded it
    to eight hex digits (`06db26f8`) following the task's `<8 hex>` wording, so value 04 mismatches in
    the Droid arms. DaVinci also deleted `jobs/out/*.txt` after use ("remove the generated intermediates"),
    so the grader reports all twelve out files missing.
- **What this corrects.** The first reading was that the research tasks fail identically for path reasons
  only. That holds for `research-01` and `-02`; `research-03` fails on top of the path defect.
- **Sensitivity (not preregistered, diagnostic only).** Re-running the same analysis code with
  `research-01` and `research-02` marked as passing in all arms (two more tasks in each pair): pass
  counts become 12 / 14 / 13 (DaVinci / direct / script), and 1 of the 15 labels changes
  (`droid-direct vs davinci output_plus_reasoning`: `GAP` to `NO_CLEAR_GAP`, interval 0.512 to 0.978).
  The requests and total-input `GAP` labels and the within-Droid requests `GAP` do not change. This is
  not used for any decision below.
- **Fix.** `scripts/bench/task_sources.py` now builds the fixture at `<run dir>/workspace`, runs the agent
  with cwd `<run dir>`, and grades `-Workspace <run dir>/workspace` (no `.git` inside it). Tested with a
  fake `subprocess`; see `scripts/bench/README.md` and the preregistration section 9, G1.

## 9. DaVinci cache behaviour (prefix-break evidence for W4)

`first_prefix_break` is not emitted in the DaVinci stream (zero matches in all 17 streams) and the runs
used no session journal, so the cache-identity diff is not available. What is available is per-request
provider usage (`provider_observation` `attempt_end` events: `usage.input` is uncached, `usage.cacheRead`
is cached) and `harness_stats.runtime` (`prunedResults`, `prunedChars`, `compactions`,
`peakContextTokens`). Computed from `davinci/<task>-r0.stdout.jsonl`.

- **No pruning or compaction happened.** `prunedResults` and `prunedChars` are 0 and `compactions` is 0
  in all 17 DaVinci runs. Peak context ranged 16,854 to 25,960 tokens (`research-02`), far below the
  pruning and compaction triggers.
- **Uncached input is not concentrated after pruning or compaction because there was none.** Totals over
  the 17 runs: uncached 185,252 tokens (matches the rows). First requests carry 32,222 (17.4%; 10,788 of
  it is `verified-dedupe`, whose first request had no cache read at all). 16 mid-run requests in 11 of
  the 17 runs carry 60,435 (32.6%): in each, `cacheRead` dropped by more than 1,000 tokens against the
  previous request. They are:

| Task | Request, previous cache -> new cache, uncached |
|---|---|
| `verified-range` | 8: 11,520 -> 9,472, 2,770; 13: 13,056 -> 11,648, 2,221 |
| `verified-slice` | 5: 11,008 -> 9,472, 1,818 |
| `verified-parser` | 13: 13,440 -> 9,472, 4,597 |
| `verified-dedupe` | 6: 11,392 -> 0, 11,746 |
| `verified-path` | 5: 11,136 -> 9,472, 2,206; 12: 13,440 -> 11,520, 3,189 |
| `t2-duration` | 4: 12,032 -> 9,472, 3,140; 10: 14,592 -> 12,032, 3,190 |
| `t4-rename` | 5: 12,288 -> 9,472, 3,192 |
| `t6-bookings` | 7: 11,520 -> 9,472, 2,539 |
| `t8-calc` | 7: 12,288 -> 9,472, 3,961; 12: 13,952 -> 12,800, 1,968 |
| `research-01` | 3: 10,496 -> 9,472, 1,347 |
| `research-02` | 8: 17,408 -> 9,472, 9,979; 12: 19,968 -> 17,792, 2,572 |

  Most reset to 9,472 tokens, the size of the cached first-request prefix, so the cache kept the system
  prompt and tools and lost the conversation tail for that request. The data does not say why (provider
  eviction, a prefix change, or something else), and they are not near any completion or verification
  reminder boundary consistently (reminders fire after requests 4 to 12; the drops occur at requests 3
  to 13). Tasks `verified-errors`, `t1-intervals`, `t3-lru`, `t5-csv`, `t7-cli` and `research-03` show no
  such drop.

## 10. First-request size (for W2)

Per task, DaVinci's `first_request_input_tokens` (row field) against the first `Streaming result`
`totalInputTokens` of the Droid session in `~/.factory/logs/droid-log-single.log*` (matched by
`droid_session_id`; 34 of 34 sessions found; request counts equal `logical_requests` for every Droid row).
Medians over the 17 tasks (first request total input tokens, including any cached portion):

| Arm | Median | Range |
|---|---:|---:|
| DaVinci | 10,802 | 10,622 to 11,100 |
| Droid-direct | 10,946 | 10,925 to 11,002 |
| Droid-script | 11,845 | 11,824 to 11,901 |

DaVinci's first request is 1.3% smaller than Droid-direct's and 8.8% smaller than Droid-script's, on the
same task prompt. The P0.2 offline byte anatomy (`first-request-anatomy.md`) has DaVinci Full at 36,447
wire bytes for the prompt `hi`; Droid's byte figures there are `not re-checked` / `no byte source`, so
bytes are not comparable and tokens are the comparison used here.

## 11. Stage 1 selection (P0.6)

Triggers are quoted from `docs/superpowers/plans/2026-10-31-droid-gap.md`. S1 and S2 have no trigger and
are not part of this selection.

| Workstream | Trigger (plan) | Verdict | Evidence |
|---|---|---|---|
| W1 Completion exit gate | "the Stage 0 success gap is 'stopped early or claimed done unverified'" | **Not fired** | Success gap is False (2 discordant tasks with different causes; 3 same-cause tasks are needed). Causes were a wrong output convention (`t5-csv`) and a narrow fix against an unrequested hidden check (`t8-calc`). In both, DaVinci ran tests and did not stop early. Section 6. Closed: no measured gap. |
| W2 Tool surface size and mid-session activation | "P0.2 shows DaVinci's first request materially larger than Droid's, and Stage 0 labels total or uncached input `GAP`" | **Not fired** | Total input is `GAP` (both Droid arms), but the first request is not larger: DaVinci median 10,802 vs 10,946 (direct) and 11,845 (script). Uncached input is `AHEAD` for Droid-direct (Droid uses more) and `NO_CLEAR_GAP` for script. Section 10. Closed: no measured gap. |
| W3 Script-first surface | "Droid-script vs Droid-direct is `GAP` on requests or input per verified task, and Droid-script vs DaVinci is also `GAP`" | **Fired on its letter (requests)** | Script vs direct requests `GAP` (0.750 [0.669, 0.840]); Script vs DaVinci requests `GAP` (0.307) and total input `GAP` (0.298). Caveats: the Script effect is on requests only (input, output and wall `NO_CLEAR_GAP`), the test pools all tasks both pass (n = 11) rather than only the verified tasks, and wall time is not lower. |
| W4 Pruning and compaction cost | "DaVinci's prefix-break log in Stage 0 concentrates uncached input after pruning or compaction on long tasks" | **Not fired** | Zero pruning and zero compaction in all 17 runs; peak context at most 25,960 tokens. Section 9. Closed as stated: no measured gap after pruning or compaction. The 16 mid-run cache drops (32.6% of uncached input) are a separate unexplained observation, not a W4 trigger. |
| W5 Startup and memory | "any gap above 25%" (filed under the resource-lifecycle plan) | **Not fired** | `startup-memory.md`: DaVinci `--version` 17.56 ms vs 82.78 ms and peak RSS 9.14 MiB vs 105.20 MiB; no row worse than Droid. Time to first provider request `not measured`. Closed: no measured gap. |

Live follow-ups (each needs its own approval naming runs, pins and model; none was started):

- **W3 probe** (at most 4 requests): one non-patch custom freeform tool case and one hosted
  `programmatic_tool_calling` case, then, only if accepted, the offline projection work and a
  Script-first A/B (at most 24 runs).
- **Cache-drop investigation** (not a plan workstream): enable cache-identity logging for DaVinci runs
  so the 16 mid-run drops in Section 9 can be explained, then a small A/B only if a DaVinci-side cause
  is found. This is the largest untriggered cost seen in the data (32.6% of uncached input).
- **Research tasks**: if the research fixtures are wanted in a future study, re-run them with the fixed
  adapter after deciding what `research-03` should accept (the seven-digit value and the out-file
  requirement), since that fixture would still fail under the current wording.

W3 is the only workstream whose trigger fired, and its fired condition rests on request counts that did
not translate into wall time or uncached input here. Whether to open it is a decision for the owner of
the Codemode plan; nothing in this report starts it.

## 12. Limits

- 17 tasks, one repetition, one machine, one account, one date. The intervals reflect task sampling
  only, not run-to-run noise.
- Public tasks expose graders and solutions and were used in DaVinci tuning (so a Droid win on them is
  conservative); `verified-*` and `research-*` are the other 9 tasks. Of the 10 tasks both DaVinci and
  Droid pass in the preregistered pairs, 6 are public tasks and 4 are verified tasks.
- Droid counts are Droid's own report relayed by the proxy, not backend-verified; usage windows were not
  recorded (deviation 2). DaVinci was on `openai-codex` directly, Droid through the proxy, so the proxy
  is a confounder.
- Droid's reasoning effort was not forced by the bench; the session file records `high` in every
  Droid row.
- DaVinci's binary commit (`c3868d87`) differs from the runner commit (`ad094715`).

## 13. Evidence files

- `C:\bench\droid-gap-20261031\study\results.jsonl`, `campaign.json`, `<harness>\<task>-r0.stdout.jsonl`,
  `.stderr.txt`, `.grader.json`.
- `C:\bench\droid-gap-20261031\gap_study.txt`, `gap_study.json`.
- `C:\bench\droid-gap-20261031\smoke\`, `warmup\`, `checkpoint\davinci.exe.identity.json`.
- Droid session files and logs under `C:\Users\sergi\.factory\` (read-only), matched through
  `droid_session_id`.
- Committed docs: `stage0-preregistration.md`, `first-request-anatomy.md`, `startup-memory.md`,
  `droid-1.1.0-identity.md`.
