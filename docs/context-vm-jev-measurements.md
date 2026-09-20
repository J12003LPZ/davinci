# Context VM / Jev hardening measurements

Measured on this Windows host in an optimized release build, September 19, 2026.
Baseline source: main 1106b1be2bffa1fb7de6e8c0708cc59d1ca2e576.
These are deterministic local fixtures. No live TypeSafe/API calls, paid model
evals, cross-platform CI run or terminal transport measurements are implied.

## Context preparation: 16 paired samples per history size

Each turn contains a user request, about 8.4 KB of tool evidence and an assistant
message. The authoritative history is JSONL-backed. A cold call invalidates the
prepared image; a warm call must reuse the same Arc without another compilation.

| Turns | First cold ms | Recompile p50 / p95 / p99 ms | Reuse p50 / p95 / p99 ms | Provider JSON bytes: history / image |
| --- | ---: | --- | --- | --- |
| 100 | 7.894 | 7.049 / 7.894 / 7.894 | 1.700 / 1.806 / 1.806 | 935,465 / 106,482 |
| 300 | 20.690 | 19.995 / 21.843 / 21.843 | 4.862 / 6.992 / 6.992 | 2,806,865 / 148,700 |

| Turns | Source body bytes | VM retained old-body bytes | Avoided former two body copies | Conservative token ceiling: history / image |
| --- | ---: | ---: | ---: | ---: |
| 100 | 846,414 | 0 | 1,692,828 | 948,789 / 105,434 |
| 300 | 2,539,614 | 0 | 5,079,228 | 2,846,789 / 144,052 |

Old-copy bytes are a logical counterfactual (two copies of visible history),
not a before/after process-RSS measurement. Agent messages, session entries and
pages still consume memory. Token ceilings use the same conservative accounting
basis on both sides; they are not billed/tokenizer counts. Warm revision hashing
still scans history bytes. Sixteen samples provide only coarse tail estimates.

## Graph: 32 samples per size

The synthetic DAG has sequential, seven-node-back and half-index dependencies.
Rendering, navigation and hit testing use graph functions directly; the key
measurement bypasses the full application event dispatcher. Quantiles
below are milliseconds except hit testing, which is in microseconds.

| Nodes | Layout p50 / p95 / p99 ms | Render p50 / p95 / p99 ms | Simulated key-to-frame p50 / p95 / p99 ms | Hit p50 / p95 / p99 us | Owned layout bytes |
| --- | --- | --- | --- | --- | ---: |
| 25 | 0.017 / 0.027 / 0.078 | 0.798 / 0.876 / 1.059 | 0.923 / 0.957 / 0.971 | 0 / 0 / 0 | 5,690 |
| 100 | 0.103 / 0.148 / 0.182 | 0.945 / 1.056 / 1.172 | 1.673 / 1.842 / 1.899 | 0 / 0 / 0 | 26,840 |
| 500 | 0.738 / 0.874 / 0.894 | 1.910 / 2.015 / 2.022 | 5.137 / 5.529 / 5.937 | 0 / 0 / 0 | 139,640 |
| 1000 | 1.475 / 1.632 / 2.037 | 3.100 / 3.393 / 3.928 | 9.495 / 10.177 / 10.580 | 1 / 1 / 1 | 280,640 |

No layout cache was added: the 1,000-node headless key-to-frame p95 was 10.177 ms
and layout p95 was 1.632 ms on this fixture. Adding invalidation state is not
justified by these measurements alone. This is not a universal frame budget:
terminal transport, slower hardware and other DAG shapes can cost more. Memory
is an owned-layout estimate, not total allocations or RSS. Zero-microsecond
hit-test samples mean below the measurement resolution.

## TypeSafe connection reuse

Thirty-two requests per mode use a loopback HTTP fixture.

| Mode | TCP connections | p50 / p95 / p99 ms |
| --- | ---: | --- |
| New client per request | 32 | 1.565 / 1.645 / 1.910 |
| Persistent client | 1 | 0.189 / 0.250 / 0.320 |

The persistent client reused one connection for all requests. The fixture uses
plain HTTP and a one-millisecond server accept poll, which contributes to cold
latency. These numbers demonstrate connection reuse, not production TypeSafe
latency, TLS cost or a user-visible TTFT improvement. Separate deterministic
tests cover nonblocking shadow admission, one-job capacity, stale completions,
body limits and caller deadlines when a provider ignores its timeout.

## Reproduction

Run the following from the repository root with RTK installed. These are the
commands used for this run. Offline mode needs cached Cargo dependencies. These three ignored tests emit JSON and assert their invariants.

```sh
rtk proxy cargo test -p davinci-agent --release --test context_vm_performance --offline --locked -- --ignored --nocapture
rtk proxy cargo test -p davinci-coding-agent --release --lib cold_and_warm_http_performance --offline --locked -- --ignored --nocapture
rtk proxy cargo test -p davinci-tui --release --lib graph_dag_performance --offline --locked -- --ignored --nocapture
```

## Correctness and delivery verification

The workspace test run covered 88 suites: 4,727 passed, one failed and 38 were
ignored. The sole failure was an ample-budget cache-affinity fixture whose
1,000-token ceiling no longer admitted its optional broker item under the
stricter accounting. The fixture now uses 10,000 tokens and explicitly checks
that the broker item is present; its tight-budget checks remain. All four tests
in that target passed on the focused rerun. No production code changed after the
workspace run. The later permission-mode cache regression also passed.

Workspace all-target check, Clippy with warnings denied, formatting, 37 Node
tests and all three release benchmarks passed. This is a full run plus focused
reruns, not a claim that the original workspace command was green. A zero-match
Jev invocation was rejected as evidence and rerun against the library target.

Independent code/security reviewers passed their reviewed scopes. The performance
reviewer checked all three measurement tables against recorded output and
confirmed their stated limitations.
Coverage tooling was unavailable, so no percentage is claimed. Live-provider
wire acceptance, real model fold quality and cross-platform CI remain unverified.

The release build passed. The executable resolved by the normal davinci command
was updated at C:\Users\sergi\.cargo\bin\davinci.exe after checking that no
DaVinci process was running. Installed and built SHA-256 hashes match:
27064BA8C92E3FC058C72D467A79B7D18CB4BC352B1AC1DEB7F9EE1CCAEB6C42.
Version 1.0.70 and help/usage smoke checks both exited successfully. The previous
binary is preserved at C:\Users\sergi\.cargo\bin\davinci.exe.before-context-hardening-20260919-01a0bc03.bak.
No user sessions, configuration or credentials were changed by delivery.

Self-review: 9/10; happy with the scoped implementation. The remaining point is
the unmeasured live-provider/model and cross-platform behavior described above.
No commit or push was performed. Start davinci normally to use the updated binary.

The mixed-role Context VM corpus has nine scenarios, including two adversarial
histories with corrections, rejected hypotheses, failed/passing verification,
resident/pageable evidence and forged provenance. Proposals are fixtures tested
at the actual runtime validation boundary; this does not measure real LLM fold
quality. Fake-summarizer integration separately verifies the prompt/parser path.

Local raw logs/JSON and independent review verdicts are retained under
`C:\Users\sergi\AppData\Local\Temp\davinci-repo-audit-01a0bc03`.
The implementation contract and caveats are in [Context VM](context-vm.md) and
[Jev decision intelligence](decision-intelligence.md).
