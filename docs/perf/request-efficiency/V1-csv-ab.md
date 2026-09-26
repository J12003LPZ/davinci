# V1 CSV A/B

The owner revoked the stop-on-hidden-regression rule and authorized continued
diagnostic investigations without repeated approval. Five CSV repetitions per
frozen binary completed with alternating order, identical isolated settings and
fresh task workspaces. No other implementation work overlapped the live timing.

Evidence: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\campaigns\ab-csv-v1-20260926`.
The manifest records hashes, settings and schedule; `analysis.json` contains
complete summaries and paired metrics. The external driver is
`request-efficiency\ab-csv-v1-20260926.py`.

| Metric | B0 | V1 |
|---|---:|---:|
| Hidden passes | 5/5 | 5/5 |
| Median wall seconds | 39.933 | 27.667 |
| p90 wall seconds | 46.381 | 41.919 |
| Total wall seconds | 200.329 | 161.574 |
| Logical requests | 40 | 31 |
| Gate reminders | 5 | 2 |
| Tool calls | 57 | 36 |
| Uncached input tokens | 38,729 | 41,428 |
| Cache ratio | 0.91262 | 0.87757 |
| Output tokens | 5,410 | 4,439 |

B0 SHA-256: `f48278e9e52586d9b75977881205adfdeb1b795190178b7f6eaacb6b2fe41dcc`.
V1 SHA-256: `259dc8391e35d33783d7079e77ca81128f3e2ebb3055570c443c47db54c0a57b`.

The earlier V1 casefolding failure did not reproduce in this sample. This does
not prove absence of a quality effect. Neither model prompts nor graders were
changed, and the original failure remains in the checkpoint report. No run
reported a credential/usage stop, unrelated change or transaction leak.

Decision: continue to a fresh complete screening arm, retain every failure and
assess the whole campaign. Do not infer acceptance or general parity from these
five pairs. The revoked stop rule no longer interrupts screening on model
failures; correctness and evidence requirements remain unchanged.
