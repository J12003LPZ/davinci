# Local resource lifecycle

DaVinci bounds retained rendering/output metadata and releases idle extension and language-server processes. These changes preserve model/effort selection, execution permissions, verification, complete session history and isolated Graph worker processes. They do not establish improved coding success or subscription savings.

## Configuration

Merge selected settings into your user `settings.json` (normally `~/.davinci/agent/settings.json`); restart the CLI after changing resource settings. Defaults stay at four ordinary subagents, sixteen workflow agents and three Graph workers. No resource reservation budget is enabled by default.

```json
{
  "subagentMaxConcurrentTasks": 4,
  "workflowMaxConcurrentAgents": 4,
  "extensionRuntime": {
    "maxPersistentSessions": 3,
    "idleTimeoutMs": 300000,
    "loadCacheMaxEntries": 128,
    "loadCacheMaxBytes": 8388608
  },
  "cache": {
    "enabled": true,
    "memoryEnabled": true,
    "persistentEnabled": true,
    "maxEntries": 1024,
    "memoryMaxBytes": 33554432,
    "persistentMaxBytes": 268435456,
    "maxObjectBytes": 8388608
  }
}
```

The example is an optional smaller resource profile, not a new default or a measured optimum. Fewer concurrent workers can reduce throughput; smaller caches can repeat computation. Extension defaults are eight hot processes, five minutes idle, 128 memo entries and 8 MiB accounted heap. Active extension requests are protected from expiry. An expired extension restarts and reloads on its next event; extension module globals do not survive. Set `extensionRuntime.idleTimeoutMs` to zero to preserve persistent module state indefinitely. Memo limits of zero disable memoization. Sessions must be 1–8, timeout at most one day, memo at most 4096 entries/64 MiB.

For an explicit shared worker reservation budget, add both positive fields:

```json
{
  "workerResources": {
    "memoryBudgetBytes": 1073741824,
    "estimatedWorkerBytes": 268435456
  }
}
```

Reservations are estimates supplied by the user, **not an OS RSS cap or automatic memory-pressure detector**. Parent runtime clones share the owner across ordinary/background/teammate, workflow and Graph workers; leases release on exit. Saturated lead launches wait with cancellation checks. Nested launches that still hold a parent reservation fail promptly when capacity is exhausted, preventing self-wait; background launches regain normal waiting once the parent lease expires. Graph admission observes absolute run deadlines. Estimates larger than the budget are rejected. `agent_status` exposes the budget, estimated reservation, reserved bytes and waiting workers. Ordinary subagent concurrency accepts 1–8; the separate eight-task argument limit remains.

For Graph, merge into project `.davinci/graph.json`:

```json
{"budgets":{"maxParallelWorkers":2}}
```

Persistent teammates retain their existing 30-minute idle default. An optional `DAVINCI_TEAMMATE_IDLE_TIMEOUT_MS=600000` expires them after ten minutes, trading continuity for less idle context. Language servers now enforce their existing five-minute idle timeout without waiting for another request. Their existing trusted `languageIntelligence.idleTimeoutMs` parser range remains 30000–900000 ms; active requests and caller/workspace authorization remain separate. See [language intelligence](../language-intelligence.md) and [agent teams](../agent-teams.md).

## Runtime bounds and recovery

- Graph output uses a 64-frame queue, 16 KiB frames and 256 KiB line assembly. Larger lines keep exact original bytes in private spool artifacts. Graph protocol stdout that required spooling is incomplete and cannot produce worker success. Slow consumers apply backpressure; cancellation is checked between lines. Post-exit inherited pipes have bounded idle/drain handling.
- The transcript retains row-height metadata and at most one cached viewport (128 blocks, 4096 rows, 8 MiB accounted styled data). Complete source history remains in the model/session. Explicit large copy selections allocate their requested result. Markdown parsing and first layout/resize still process source synchronously; CPU latency on enormous histories remains a limitation.
- Extension memo retention accounts strings/containers and conservative map-node overhead; it is not a precise allocator/RSS measurement. Idle process cleanup drops children outside pool locks.
- In-memory cache eviction uses an ordered access index instead of scanning every entry. Cache and disk limits retain their previous defaults. Token Governor maintenance uses a cross-process lock and successful hourly marker; failed scans remain retryable.
- Authenticated Graph worker roles avoid opening optional learning/memory/visual services absent from their authorized tool surface. Authorized skill/memory tools retain real stores. Immutable model metadata shares an Arc snapshot; trust, credentials and execution-time permissions are not cached as grants.

## Reproduce local measurements

Use Rust 1.88.0, Node and Python 3.11+. All workloads are deterministic local fixtures; the CLI comparison passes `--offline` explicitly. No provider calls are made. Preserve a baseline source archive and release executable before building the candidate separately.

```sh
python3 scripts/bench/resource_lifecycle.py prepare /absolute/baseline-source /tmp/baseline-fixture
python3 scripts/bench/resource_lifecycle.py prepare "$PWD" /tmp/candidate-fixture --candidate
cargo build --release --offline --manifest-path /tmp/baseline-fixture/Cargo.toml --target-dir /tmp/resource-target -j 2
cp /tmp/resource-target/release/davinci-resource-fixture /tmp/baseline-fixture-bin
cargo build --release --offline --features candidate --manifest-path /tmp/candidate-fixture/Cargo.toml --target-dir /tmp/resource-target -j 2
cp /tmp/resource-target/release/davinci-resource-fixture /tmp/candidate-fixture-bin
python3 scripts/bench/resource_lifecycle.py compare --baseline /tmp/baseline-fixture-bin --candidate /tmp/candidate-fixture-bin --samples 5 --output /tmp/components.json
cargo build --release -p davinci-coding-agent --bin davinci --locked --target-dir /tmp/resource-target -j 2
python3 scripts/bench/resource_lifecycle.py compare --kind cli --baseline /absolute/baseline-davinci --candidate /tmp/resource-target/release/davinci --samples 5 --output /tmp/cli.json
python3 -m unittest discover -s scripts/bench/tests -p test_resource_lifecycle.py
```

`prepare` copies the actual production Graph process and JS host modules into an external fixture crate. Only its local execution-boundary hooks are stubbed; this benchmark cannot validate permissions. Product permission tests do that. Both component variants use the same optimized profile without LTO; production CLI variants use the repository release profile. Compare variants within each profile.

Fixtures cover a 50,000-entry transcript (cold layout plus five warm viewport calls), 12,000 cache churn operations, a 32 MiB newline-free child stream with exact byte recovery, classifier host startup, and real Node extension/LSP active/idle processes. Active fixtures keep a handler/request pending beyond the accelerated idle timeout and assert it completes in the same process. Idle fixtures accelerate the configured timeout to one second and observe after 2.3 seconds; the LSP fixture directly sets its internal config below the user parser minimum. This is not a real TypeScript index or extension workload. Active-request preservation is also covered by subprocess regression tests; it is not inferred from idle RSS.

The Linux sampler sums observed process-tree RSS every 2 ms, with PID birth identities. Shared pages count once per process and short peaks can be missed; this is not unique physical memory. Unsupported counters are unknown. Five independent samples support medians/ranges, not credible p95 claims. Build/source identities and raw samples accompany the recorded results. No live coding quality, platform-wide speedup, subscription usage or sustained production workload claim follows from these fixtures.



## Recorded component results

Linux x86_64, Rust 1.88.0, Node 24.19.0, Python 3.12.14. Baseline source `d44028fb`; candidate code `8fd946d6`. Five runs per variant. All original 32 MiB output bytes were recovered, and transcript row counts/checksums matched. [Raw samples, source digests, binary hashes and validation](resource-lifecycle-results.json) identify the exact measured artifacts.

RSS below is the median sampled process-tree peak, followed by the largest observed peak in parentheses, in MiB. Cold time is median [minimum–maximum], in milliseconds; transcript time includes model/history construction plus first layout, and host time includes temporary-directory setup.

| Local fixture | Baseline RSS | Candidate RSS | Baseline cold ms | Candidate cold ms |
| --- | ---: | ---: | ---: | ---: |
| 50,000-entry transcript | 40.87 (40.93) | 16.08 (16.14) | 197.30 [133.72–284.65] | 108.46 [105.28–129.61] |
| 12,000 cache inserts/churn | 4.49 (4.68) | 4.87 (4.93) | 250.54 [181.06–253.10] | 59.24 [52.19–97.79] |
| 32 MiB newline-free output | 59.55 (66.73) | 7.34 (7.41) | 90.50 [77.92–117.17] | 69.47 [60.85–96.38] |
| Full baseline / classifier worker host | 5.79 (6.06) | 5.41 (5.45) | 21.44 [21.33–22.01] | 21.34 [21.09–21.81] |

The ordered cache index uses slightly more RSS in this fixture while reducing churn time. Classifier bootstrap avoided learning-store creation in all five candidate runs (baseline created it in all five); its cold time was essentially unchanged with empty stores. These are local observations, not universal speedups.

Baseline warm viewport visits: median 0.235 ms, range 0.174–0.529 ms (five visits within each of five runs). Candidate warm viewport visits: median 0.225 ms, range 0.200–0.874 ms (five visits within each of five runs). The ranges overlap; this does not establish a warm-latency improvement.

Both JS and LSP baseline children remained resident after the 2.3-second idle observation (roughly 27–29 MiB per Node process); all candidate idle children had exited. Both variants completed every active handler/request in the same process beyond the accelerated one-second timeout. Peak RSS of these lifecycle fixtures stayed similar because each first starts a real server/runner. The improvement is release after idle, not a lower active-process peak.

Sampling requested a 2 ms pause between snapshots; actual cadence also includes `/proc` enumeration and sampler overhead. Observations are descriptive: baseline runs precede candidate runs, OS caches are not reset, shared pages are counted per process, and short peaks can be missed. Raw observation counts and all samples are retained. The sampler itself is outside the measured tree but competes for CPU. No p95, live coding-quality or subscription-saving claim is made.

Output spools are recovery artifacts, outside the persistent-cache byte budget. Capture failures remain explicit; preserving evidence can consume disk space. Cold resize still performs synchronous source/height work. Native Windows/macOS behavior has not been executed in this Linux environment.

## Validation and review

The full workspace run before the final review corrections passed 7,095 tests with zero failures and 63 ignored tests. After correcting the review findings, 3 admission, 9 selected resource/host/settings (1 existing ignored), and 61 transcript tests passed; workspace compilation and formatting passed. No additional broad test run was made after the user requested only necessary tests. Three sampler parser tests passed. The [independent review and lead resolutions](resource-lifecycle-review.md) record the two Important findings and the additional inherited-owner correction.

The final native release build passed using the repository release profile and locked dependencies. Five explicitly offline CLI invocations per variant produced the expected stub, with zero provider calls. Baseline binary SHA-256 remained `cfcf8e4272ccd6ab581f99da90623fdefb1876c7954de5d72ea6ecf027406b7e`; candidate is `58cec03675154de588c572f93647df3f25c63be3e8dc8fb168e55a21e611b778`.

| Offline CLI invocation | Baseline | Candidate |
| --- | ---: | ---: |
| Harness wall time median [min–max], ms | 108.21 [93.45–111.27] | 92.80 [91.30–111.78] |
| Sampled peak-tree RSS median (max), MiB | 30.77 (32.07) | 26.29 (27.91) |

Wall time includes process startup, one offline stub turn, sampling and output capture. Five runs with overlapping timing ranges do not establish a general startup speedup. This is separate from the component profile and does not simulate an interactive session or live coding task. The installed baseline was preserved; the candidate executable is a separate build artifact.

## PR review: bounded output artifacts

Each child retains at most 64 oversized-output files and 32 MiB of spooled bytes. A single line may spool up to 16 MiB; exceeding that limit returns an explicit capture error. Eviction deletes the file before dropping its metadata. Temporary-file ownership covers callback failures, and supervisor errors delete previously retained files. Successful outcomes transfer the surviving artifact references to their caller. These are per-child bounds, not a global disk-retention policy.

Native validation: 19 focused process/output tests passed (one existing ignored fixture), including 100 oversized lines, byte-budget eviction, failed delivery, and a real child that fails after producing one retained spool. Locked package compilation, formatting, and diff checks passed. Earlier benchmark results above predate these limits and do not measure this revision.
