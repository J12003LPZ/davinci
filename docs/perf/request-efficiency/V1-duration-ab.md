# V1 duration A/B

The owner authorized five runs per binary after the interrupted V1 screen.
All ten runs completed in one campaign with alternating B0/V1 order, the same
isolated settings, model catalog, effort and frozen duration fixture. Each run
used a fresh workspace. No other implementation or live campaign overlapped.

Evidence: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\campaigns\ab-duration-v1-20260926`.
`ab-manifest.json` records the schedule, configuration and identities;
`analysis.json` contains the paired metrics and grader-only failure rechecks.
The external driver is `request-efficiency\ab-duration-v1-20260926.py`.

| Metric | B0 | V1 |
|---|---:|---:|
| Hidden passes | 2/5 | 3/5 |
| Median wall seconds | 42.193 | 35.860 |
| p90 wall seconds | 50.486 | 59.634 |
| Total wall seconds | 221.299 | 206.703 |
| Logical requests | 44 | 35 |
| Gate reminders | 5 | 3 |
| Tool calls | 43 | 29 |
| Uncached input tokens | 40,561 | 43,237 |
| Cache ratio | 0.91511 | 0.88626 |
| Output tokens | 6,543 | 5,847 |

B0 SHA-256: `f48278e9e52586d9b75977881205adfdeb1b795190178b7f6eaacb6b2fe41dcc`.
V1 SHA-256: `259dc8391e35d33783d7079e77ca81128f3e2ebb3055570c443c47db54c0a57b`.
Both binaries' identity sidecars supplied source and dirty-diff hashes.

B0 repetitions 0, 3 and 4 and V1 repetitions 1 and 4 each passed 16/17 tests.
All five grader-only rechecks reproduced the same trailing-whitespace failure.
No credential/usage stop, unrelated edit or transaction leak was reported.
No benchmark source, hidden test or grader was changed, and no hidden test was
sent to either benchmark model.

Matched median wall ratio (V1/B0) was 0.72413 and request ratio 0.66667.
However, V1 p90 and total uncached input were higher. Five pairs on one small
task do not establish performance superiority or statistical quality parity.

Decision: the initial failure is not unique to V1; the parent reproduces the
same failure more often in this sample. Record shared model variance and resume
the previously authorized screening in a new directory. V1 remains unaccepted
until the screening and checkpoint review finish. This diagnostic is not a
replacement for the same-campaign Codex control or the complete screening arm.
