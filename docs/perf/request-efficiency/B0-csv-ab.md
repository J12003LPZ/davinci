# B0 CSV same-campaign A/B

Owner-authorized diagnostic: 10 repetitions per immutable binary on `t5-csv`,
with alternating first-arm order. One isolated agent directory, identical
settings, credentials, model catalog, public prompt and frozen task fixtures.
No concurrent development or other live campaign ran during these measurements.

| Arm | Passes | Failed repetitions (zero-based) | Median wall seconds | Total wall seconds |
|---|---:|---|---:|---:|
| baseline | 9/10 | [8] | 39.925 | 391.099 |
| B0 | 10/10 | [] | 41.517 | 414.896 |

| Additional metric, all 10 runs per arm | Baseline | B0 |
|---|---:|---:|
| p90 wall seconds | 47.374 | 45.911 |
| Logical requests | unavailable | 83 |
| Gate reminders | 10 | 10 |
| Tool calls | 113 | 115 |
| Uncached input tokens | 76016 | 71002 |
| Cache ratio | 0.916236 | 0.921990 |
| Output tokens | 10213 | 10781 |

Exact paired metrics and aggregate availability are saved in
`paired-analysis.json` beside the raw results.

The baseline failed all four hidden tests at repetition 8; B0 passed all four
on every repetition. Every process exited zero, including the baseline's
incorrect result. This reproduces the failure class without B0 instrumentation.
Per the owner's decision rule, classify the earlier CSV failure as model
variance and resume the implementation plan. This small sample does not prove
equal failure probabilities or establish a performance improvement.

Binary identities:
- Baseline: `2027a2a55f51391f9d9aee24b52fb5b280cd3f8fda1c12d13ae33b8ddcd9c831`
- B0: `f48278e9e52586d9b75977881205adfdeb1b795190178b7f6eaacb6b2fe41dcc`

Verified all 20 row identities, exact repetition membership, fixture hash,
identical settings/catalog hashes, executable versions and current binary
bytes after the run. Baseline request counters are unavailable; no request
savings claim is made. The original interrupted screen remains incomplete;
a separate fresh 48-row screen subsequently completed (see `B0-lite.md`).
No hidden fixtures, graders or generated task solutions were edited. No
subagents, commits, pushes, merges or installations were used for this A/B.

Evidence: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\campaigns\ab-t5-20260926`.
The directory contains the schedule, manifests, immutable run results, raw
stdout/stderr, generated implementations and `ab-summary.json`. The external
orchestrator is `C:\Users\sergi\davinci-bench-evidence\request-efficiency\ab-t5-20260926.py`.
