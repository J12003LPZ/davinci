# Stage 2 - promotion ledger, claims and crosswalk status (2026-11-01)

Plan: `docs/superpowers/plans/2026-10-31-droid-gap.md`, Stage 2. This record answers
the three Stage 2 questions for the program as it stands: which candidates reached a
gate, what may be claimed today, and what the coverage matrix says after the work that
merged. It changes no default and claims no parity. Every number below is copied from
`stage0-report.md` or from a command whose output is recorded in section 5.

## 1. What the gate can decide today

| Fact | Value | Source |
|---|---|---|
| Live runs used / program cap | 56 / 120 | `stage0-report.md` section 3 |
| Runs left | 64 | 120 minus 56 |
| Runs a head-to-head superiority claim needs | at least 150 scenarios x 3 repetitions, about 900 | plan, Stage 2 |
| Candidate default changes awaiting a gate | none (section 2) | this record |
| Workstreams whose trigger fired | W3 only | `stage0-report.md` section 11 |
| Workstreams with a live step still unrun | W3 | plan, Stage 1 W3 |

The gate has nothing to decide yet: no candidate default change exists, because the
only fired workstream (W3) has not run its probe. The 900-run requirement is larger
than the 64 runs left, so a head-to-head claim cannot be produced inside this
program's budget. That is a budget fact, not a result, and it is why the program ends
at a measured gap rather than a parity claim.

## 2. Ledger entries

The fields are the plan's ledger. One entry per candidate the program considered.
"No change" is a real outcome: the plan adopts a mechanism only when a measured gap
points to it.

Common to every entry below: model `gpt-6.1-sol`, DaVinci effort policy `high` (fixed),
DaVinci route `openai-codex` (WebSocket), Droid 1.1.0 (`7f23da2d...a775`) via
CLIProxyAPI with `BENCH_DROID_AUTO=high`; dataset 17 tasks x 3 arms x 1 repetition =
51 scored runs, plus 3 smoke and 2 unscored warm-up rows; bootstrap seed 20261031,
10,000 resamples, 90%. Pins, deviations and the run ledger are in `stage0-report.md`
sections 2 and 3.

### W1 - completion exit gate

| Field | Value |
|---|---|
| Hypothesis | one bounded reminder when a turn ends with open plan items reduces the "stopped early or claimed done unverified" gap |
| Change | none |
| Baseline | `main` `ecc84853`; default settings unchanged |
| Raw results | `stage0-report.md` sections 6 and 11 |
| Gate | trigger not fired: the success gap is False - two discordant tasks with different causes (`t5-csv` a wrong output convention, `t8-calc` a narrow fix against an unrequested hidden check), where three same-cause tasks are required. DaVinci ran tests and did not stop early in either. |
| Outcome | **keep current default.** No A/B was run (0 of the at most 24 runs the plan allows). |

### W2 - tool surface size and mid-session activation

| Field | Value |
|---|---|
| Hypothesis | every mid-session tool activation re-sends the conversation uncached, so a smaller or deferred surface would cut total input |
| Change | none |
| Baseline | Full tool surface (40 tools); Lean (16) remains selectable |
| Raw results | `stage0-report.md` section 10; `first-request-anatomy.md` |
| Gate | trigger not fired. Total input is `GAP` for both Droid arms, but the first request is not larger: DaVinci median 10,802 tokens against 10,946 (Droid-direct) and 11,845 (Droid-script). Uncached input is `AHEAD` for Droid-direct and `NO_CLEAR_GAP` for Droid-script. |
| Outcome | **keep Full as the default.** The two probe cases the plan conditions on `tool_search`/`additional_tools` were not written, because the trigger that would justify them did not fire. The closure reason is recorded in `docs/tool-discovery.md`. |

### W3 - Script-first surface

| Field | Value |
|---|---|
| Hypothesis | Droid's Script-first surface spends fewer model requests per verified task than DaVinci's add-on Codemode |
| Change | step 2 was implemented without the probe, on the operator's decision (`e84830a7`, PR #271): `codemode` is projected as a custom freeform tool and the decoded `{"input": …}` maps to `code`, behind the same freeform rollout flag as `apply_patch`; no default changed. The capability probe ran on 2026-10-31 (4 requests, `gpt-5.6-luna`): the non-patch custom freeform shape was accepted (HTTP 200, 0 cached, 0 cache-write) and hosted `programmatic_tool_calling` was rejected (HTTP 400). The probe now routes through the backend login; before the fix it sent the subscription token to `api.openai.com` and recorded 401. |
| Baseline | `main` `ecc84853`; Codemode as an add-on tool, `ReadOnly` production mode |
| Raw results | `stage0-report.md` sections 4 and 7 |
| Gate | trigger **fired on its letter**: Droid-script vs Droid-direct requests `GAP` 0.750 [0.669, 0.840]; Droid-script vs DaVinci requests `GAP` 0.307 and total input `GAP` 0.298. Caveats recorded in the report: the Script effect is on requests only (input, output and wall are `NO_CLEAR_GAP`), the test pools all tasks both arms pass (n = 11) rather than verified tasks only, and wall time is not lower. |
| Outcome | **no default change.** Step 2 shipped behind the flag, and the probe now confirms the custom shape is accepted. Hosted `programmatic_tool_calling` is unavailable on this route. Step 3 (the A/B, at most 24 runs) and step 4 stay unrun; step 3 needs its own preregistration and approval, and step 4 needs step 3 to win plus the C07/C08 gates. |

### W4 - pruning and compaction cost

| Field | Value |
|---|---|
| Hypothesis | DaVinci's 65% to 35% pruning trigger and its compaction reserve concentrate uncached input on long tasks |
| Change | none |
| Baseline | `main` `ecc84853`; Context VM `off` |
| Raw results | `stage0-report.md` section 9 |
| Gate | trigger not fired: zero pruning and zero compaction events in all 17 runs; peak context at most 25,960 tokens. |
| Outcome | **keep current settings.** The 16 mid-run cache drops (32.6% of uncached input) are a separate unexplained observation, not a W4 trigger, and were not chased inside this program's budget. |

### W5 - startup and memory

| Field | Value |
|---|---|
| Hypothesis | DaVinci starts slower or uses more memory than Droid 1.1.0 |
| Change | none |
| Baseline | `main` `ecc84853`; Droid 1.1.0 `7f23da2d...a775` |
| Raw results | `startup-memory.md` sections 3 to 5 |
| Gate | trigger not fired: `--version` 17.56 ms against 82.78 ms and peak RSS 9.14 MiB against 105.20 MiB, 10 repetitions each. No row is worse than Droid by 25%, so nothing was filed under the resource-lifecycle plan. Time to first provider request was not measured on either side. |
| Outcome | **no change, nothing filed.** |

### S1 and S2 - delivered, no gate required

The plan exempts these two from the trigger rule because they fix correctness hazards
found in review. They are behavior changes, so they are recorded here with their
anchors even though no default moved.

| Field | S1 loop stop | S2 Codemode output tail |
|---|---|---|
| Change | the watchdog's repeated-command, repeated-read and oscillation signals now reach the live path: a nudge on the first signal, a stop on a second signal on the same digest (`ea06303f`, `4b5dc43e`, `217044c3`) | over the 16 KiB model budget, Codemode keeps head and tail with an explicit omitted-bytes marker instead of dropping the tail (`9811182f`) |
| Test anchors | `progress_watchdog.rs::repeated_unchanged_reads_nudge_then_stop`, `repeated_command_without_new_output_nudge_then_stop`, `oscillating_edits_nudge_then_stop`, `oscillating_edits_observe_path_escalates_to_stop`, `polling_and_new_evidence_loops_never_signal`; turn level `watchdog_pending_signal_queues_repair_reminder_and_continues`, `watchdog_escalated_signal_stops_run_without_provider_call` | `codemode_supervisor_budget.rs::native_truncated_output_is_partial_not_completed` |
| Result | `cargo test -p davinci-agent watchdog --offline --locked`: exit 0, 18 passed, 0 failed (section 5) | not executed in this session: both tests are `#[ignore]`d without the disposable admitted Node and fixture bundle, and this machine has no `node_modules` for it (section 5) |
| Known difference | DaVinci stops on the second signal, which arrives on the 4th identical call, earlier than Droid's 6. This is deliberate: the plan says not to copy Droid's numbers. | none |

## 3. What may be claimed today

**May be said, with these labels:**

- On the 17-task Stage 0 study (`RUNTIME_OBSERVED`, one machine, one account, one
  date, one repetition): DaVinci used about 2.5x (direct) to 3.3x (script) the model
  requests and 2.8x to 3.4x the total input tokens of Droid on the tasks both pass;
  pass counts were DaVinci 10/17, Droid-direct 12/17, Droid-script 11/17; wall time
  was `NO_CLEAR_GAP` in both cross pairs.
- Startup: `--version` 17.56 ms against 82.78 ms and peak RSS 9.14 MiB against
  105.20 MiB, 10 repetitions each (`RUNTIME_OBSERVED`, `--version` only, not task
  latency and not mid-session RSS).
- The two correctness fixes in section 2 are delivered and their tests pass (S1) or
  exist and are gated on the CI fixture bundle (S2).

**May not be said:**

- That DaVinci has parity with Droid, on behavior or on performance. No matrix row
  reaches `PERFORMANCE_PARITY`, and the run budget cannot produce one.
- That DaVinci is more efficient on tasks. The measured direction on this study is the
  opposite for requests and input.
- That any subscription allowance was saved. Token counts do not prove that, and the
  Stage 0 study recorded no usage windows (deviation 2 in `stage0-report.md`).
- That the Codemode Script-first surface is better, cheaper, or cache-friendlier. Its
  probe has not run.

## 4. Coverage matrix status

`2026-10-31-droid-parity-crosswalk.csv` is the program's coverage matrix. Its
`davinci_baseline_disposition` column used a pre-work vocabulary; this record switches
the rows the merged work actually changes to the spec's terminal labels
(`BEHAVIOR_PARITY`, `JUSTIFIED_DIFFERENCE`) and leaves every other row alone.

| Row | Was | Now | Why |
|---|---|---|---|
| R-15 nudge at three identical calls | `PARTIAL_UNVERIFIED` | `BEHAVIOR_PARITY` | the 3-wide window produces the first signal at the 3rd identical call; `REPEATED_SIGNAL_NUDGE_AT = 1` |
| R-16 stop at six identical calls | `PARTIAL_UNVERIFIED` | `JUSTIFIED_DIFFERENCE` | DaVinci stops on the second signal, at the 4th identical call; the plan says not to copy Droid's 6 |
| R-18 progress-making polls exempt | `OPEN_GAP` | `OPEN_GAP` (note updated) | the exemption is real (identical output digest, or `is_job_poll`), but there is still no 24-iteration pure-poll cap |
| C-42 three identical calls nudge, six stop | `PARTIAL_UNVERIFIED` | `JUSTIFIED_DIFFERENCE` | same as R-15 plus R-16: nudge matches, stop is earlier by design |
| C-43 repeats that return new results can continue | `PARTIAL_UNVERIFIED` | `BEHAVIOR_PARITY` | a read or command whose output digest changes never signals; job polls are exempt |
| C-13 printed output capped, full results logged | `PARTIAL_UNVERIFIED` | `PARTIAL_UNVERIFIED` (note updated) | the cap now keeps head and tail with a marker; the searchable full-result log is still not wired |

Two properties of this table matter more than the row count. First, no row moved to a
passing label on the strength of a source read: each cites a test that exists in the
tree. Second, rows that the merged work does not change were left untouched, including
C-41 and R-17 (three no-output turns), which DaVinci still does not implement.

## 5. Verification run for this record

Run in the worktree on branch `J12003LPZ/droid-gap-stage2`, from `origin/main`
`ecc84853`.

```
cargo test -p davinci-agent watchdog --offline --locked
  exit 0
  test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 2137 filtered out
  (the other test binaries matched no test and reported 0 passed, 0 failed)
```

The 18 include `repeated_unchanged_reads_nudge_then_stop`,
`repeated_command_without_new_output_nudge_then_stop`,
`oscillating_edits_nudge_then_stop`,
`oscillating_edits_observe_path_escalates_to_stop`,
`polling_and_new_evidence_loops_never_signal`,
`test_legitimately_polling_job_output`,
`turn::tests::watchdog_pending_signal_queues_repair_reminder_and_continues` and
`turn::tests::watchdog_escalated_signal_stops_run_without_provider_call`.

```
cargo test -p davinci-coding-agent --test codemode_supervisor_budget --offline --locked
  exit 0
  test result: ok. 0 passed; 0 failed; 2 ignored
  both tests are #[ignore = "requires the disposable admitted Node and fixture bundle"]
```

Building that bundle locally was attempted and failed: `node_modules` is absent under
`crates/davinci-coding-agent/codemode-host`, so `node package-bundle.mjs` stops with
`scandir ... node_modules/@earendil-works/pi-codemode/dist`. Installing it needs the
network. CI builds the bundle and runs both tests with `--ignored`
(`.github/workflows/ci.yml`, "Native Codemode supervisor suite"), so S2's assertion is
covered there, not here.

## 6. What remains, and what each step needs

| Step | Runs or requests | Needs |
|---|---|---|
| W3 probe: `custom_freeform_non_patch` and `programmatic_tool_calling`, each with a byte-identical reuse follow-up | 4 requests | **run 2026-10-31**: custom freeform accepted 200, hosted PTC rejected 400; evidence in `docs/cache/codex-backend-probe.md` |
| W3 step 2: project `codemode` as a custom freeform tool | 0 | delivered behind the freeform flag (`e84830a7`); enabling it still needs the probe's acceptance evidence |
| W3 step 3: Script-first A/B | at most 24 runs | its own preregistration and approval |
| W3 step 4: controlled mode | 0 runs, C08 gate | only if step 3 wins |
| Cache-drop investigation (16 mid-run drops, 32.6% of uncached input) | not estimated | approval; it is the largest untriggered cost in the data |
| Stage 2 gate | 0 | a candidate default change to gate; none exists |

None of the live steps beyond the probe was started. The program's delivered state is: Stage 0
measured, S1 and S2 merged, W1/W2/W4/W5 closed as no measured gap, W3's probe run
(custom freeform accepted, hosted PTC rejected) and its step 2 shipped behind the freeform flag,
with the A/B unrun.
