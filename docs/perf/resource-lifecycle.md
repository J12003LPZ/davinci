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

Reservations are estimates supplied by the user, **not an OS RSS cap or automatic memory-pressure detector**. Parent runtime clones share the owner across ordinary/background/teammate, workflow and Graph workers; leases release on exit. Saturated launches wait with cancellation checks. Estimates larger than the budget are rejected. `agent_status` exposes the budget, estimated reservation, reserved bytes and waiting workers. Ordinary subagent concurrency accepts 1–8; the separate eight-task argument limit remains.

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

Fixtures cover a 50,000-entry transcript (cold layout plus five warm viewport calls), 12,000 cache churn operations, a 32 MiB newline-free child stream with exact byte recovery, classifier host startup, and real Node extension/LSP idle processes. Idle fixtures accelerate the configured timeout to one second and observe after 2.3 seconds; the LSP fixture directly sets its internal config below the user parser minimum. This is not a real TypeScript index or extension workload. Active-request preservation is covered by subprocess regression tests, not inferred from idle RSS.

The Linux sampler sums observed process-tree RSS every 2 ms, with PID birth identities. Shared pages count once per process and short peaks can be missed; this is not unique physical memory. Unsupported counters are unknown. Five independent samples support medians/ranges, not credible p95 claims. Build/source identities and raw samples accompany the recorded results. No live coding quality, platform-wide speedup, subscription usage or sustained production workload claim follows from these fixtures.

Measured results and validation are recorded below after the final source build.
