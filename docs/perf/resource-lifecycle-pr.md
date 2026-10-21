Long conversations retained a full styled transcript, Graph output had unbounded buffering, and idle JS/LSP processes stayed resident. This bounds retained rows/output/memo metadata, adds idle cleanup, shares worker reservations and immutable model metadata, and avoids unused Graph-role stores and repeated Governor scans.

- Preserve existing concurrency/cache defaults, model/effort selection, permission checks, complete history and recoverable output evidence. New concurrency and estimated worker budgets are opt-in.
- Fix review findings: inherited budgets survive worker/turn rebuilding; nested saturated launches fail promptly; Graph admission respects run deadlines; large Shell history streams rows.
- Keep isolated Graph processes. No new product dependencies.

Local release fixtures (five runs per variant, median sampled process-tree peak RSS):

| Fixture | Baseline | Candidate |
| --- | ---: | ---: |
| 50,000-entry transcript | 40.87 MiB | 16.08 MiB |
| 32 MiB newline-free output, all bytes recovered | 59.55 MiB | 7.34 MiB |
| Cache churn | 4.49 MiB / 250.54 ms | 4.87 MiB / 59.24 ms |

Both candidate idle Node fixtures exited after the accelerated timeout; all active JS/LSP requests completed in the same process. Classifier bootstrap avoided learning-store creation; empty-store startup time was essentially unchanged. RSS sampling can miss short peaks and counts shared pages per process. No p95, live coding-quality or subscription-saving claim.

Validation: full workspace before final review fixes: 7,095 passed, 0 failed, 63 ignored. After fixes: 3 admission, 9 selected resource/host/settings (1 ignored), and 61 transcript tests passed; workspace compilation and formatting passed. Three sampler tests passed. Broad tests were not repeated after the user requested only necessary checks.

Compatibility: JS runners now expire after five minutes idle by default; set extensionRuntime.idleTimeoutMs to 0 when module-global continuity is required. Resource reservations estimate usage rather than enforce OS RSS. Cold transcript layout/resize still scans source synchronously. Native Windows/macOS execution is unverified.

Configuration examples, raw samples/build identities, and the independent review with resolutions are in docs/perf/resource-lifecycle.md, docs/perf/resource-lifecycle-results.json, and docs/perf/resource-lifecycle-review.md.

Final native release build passed with locked dependencies. Five offline CLI runs: median sampled peak-tree RSS 30.77→26.29 MiB; median harness wall time 108.21→92.80 ms with overlapping timing ranges. Zero provider calls; installed baseline hash unchanged. Raw binary/source identities are in the results file.
