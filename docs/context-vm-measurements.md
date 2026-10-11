# Context VM and graph hardening measurements

## October 10, 2026: hybrid mode

Same scripted session and fixture summarizer as below, run to 300 turns so
the off and hybrid modes compact more than once (128k window, threshold at
the window less the 16k reserve). Replies report the request at four bytes a
token as their provider count. Structural and offline, as below.

| Mode, 300 turns | Compactions | Summarizer calls / bytes | Non-prefix bytes | Prefix reuse | Requirement checks kept |
| --- | ---: | ---: | ---: | ---: | ---: |
| Off | 2 | 4 / 516,278 | 1,222,076 | 98.4% | 394/883 |
| Active (fold policy below) | 25 | 25 / 919,565 | 8,791,022 | 61.2% | 883/883 |
| Hybrid | 2 | 2 / 246,468 | 1,242,851 | 98.3% | 883/883 |
| Hybrid, no summarizer | 2 | 0 / 0 | 1,242,851 | 98.3% | 883/883 |

At 60 turns, before any compaction, hybrid requests are byte-identical to
off. Hybrid's extra 1.7% of non-prefix bytes is the larger ledger summary
after each compaction. Off's requirement loss starts at its first compaction
because the fixture summary is generic: it shows that off depends entirely on
the summarizer, not how a real model summarizes. Hybrid's ledger keeps the
requirements with a summarizer that adds nothing, or none.

Found on the way: a fold in a session wrote a `context_checkpoint` entry type
that the session codec rejects, so a session with a fold could not be
reopened. Fold records are now `custom` entries with that `customType`; the
bare type is still read in memory.

## October 10, 2026: fold policy and cache-stable image

Baseline: `6e5e81c` (main). Codex source read at `openai/codex@de8fab6`.
Offline and structural only: no provider was called, so no cache hit, billed
token, latency or task-success claim is made here.

### Codex and DaVinci compaction, from source

| | Codex (`codex-rs/core`) | DaVinci active VM (baseline) |
| --- | --- | --- |
| Model-visible history | Append-only items, reasoning items included, until compaction | Checkpoint + delta pages + a sliding hot window of recent events; older assistant/tool events rendered as wrapped text |
| Trigger | Provider-reported `last_token_usage.total_tokens` plus a bytes/4 estimate of newer items, against `auto_compact_token_limit` and the effective window (`session/context_window.rs`) | Byte ceiling (about a token per byte) against an 80% pressure trigger, plus delta-depth/delta-token maintenance once the ceiling passed 50% |
| Local compaction | Same conversation prefix plus the compaction prompt; new history is initial context, up to 20k tokens of recent user messages verbatim, and the summary (`compact.rs`) | Separate structured-JSON prompt over every event; validated `CheckpointProposal`; deterministic fallback |
| Native/remote compaction | Normal `/responses` stream with a `CompactionTrigger` input item; exactly one opaque `compaction` output item is kept with up to 64k tokens of retained messages (`compact_remote_v2*.rs`) | Not implemented; the recorded ChatGPT-backend probe rejected standalone `/responses/compact`; remote v2 is unprobed |
| Requirement retention | Recent user messages verbatim, newest first | Deterministic goals: 480-byte head of each user message, first + 15 newest |
| Exact recovery | None | `retrieve_context` by page or source ref |

### Defects found

1. Fold pressure was the admission ceiling (about 1 token per byte) compared
   with a threshold the off mode applies to bytes/4. The same 20-turn history
   read 79,332 tokens active and 16,589 off.
2. The hot window slid one event per turn and the delta sat before it, so
   almost no request repeated the previous one's input prefix.
3. A budget-bound image dropped one old event per turn: the same churn.
4. Each fold re-sent every event to the summarizer, in a prompt that shares no
   prefix with ordinary requests.
5. Deterministic goals kept a 480-byte head: a requirement later in a long
   paste was lost once its event left the window. Tool evidence lost its
   trailing test verdict.
6. Without a summarizer, maintenance folds only rotated the cache epoch.

### Scripted session (`tests/context_vm_efficiency.rs`)

60 turns, 128k window, about 3.5 KB of tool output per turn, three
requirements (one inside a 5 KB paste, one correction). Each request goes
through `run_loop` and `messages_for_provider()` and is serialized as OpenAI
Responses input. "Non-prefix bytes" are request bytes after the longest
common prefix with the previous request: an upper bound on what a prefix
cache could not reuse, not a measured cache miss. The summarizer is a fixture
that proposes nothing new, so retention rests on the deterministic state.

| Mode | Folds | Summarizer prompt bytes | Request bytes | Non-prefix bytes | Prefix reuse | Requirement checks kept |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Off | 0 | 0 | 6,629,364 | 213,539 | 96.8% | 163/163 |
| Active, baseline | 43 | 3,540,159 | 5,168,814 | 5,098,479 | 1.4% | 110/163 |
| Active, this change | 3 | 148,342 | 3,639,547 | 690,404 | 81.0% | 163/163 |
| Active, baseline, no summarizer | 43 | 0 | 5,168,814 | 5,098,479 | 1.4% | 110/163 |
| Active, this change, no summarizer | 0 | 0 | 3,488,088 | 555,331 | 84.1% | 163/163 |

Active mode still re-sends about 7.6 KB per steady turn against about 3.5 KB
off, because the previous native tool exchange is re-rendered as wrapped
evidence once it leaves the live suffix. Off mode never compacted in this
fixture; a session long enough to compact would change its column. Both
summarizer columns are fixtures, not model fold quality.

Regression tests that fail on the baseline: `tests/context_vm_fold_policy.rs`
(five) and the hot-window and budget-restart tests in
`runtime/context_vm/mod.rs`. Reducer ledger tests cover the excerpt and goal
eviction rules.

### Not changed, and why

- Admission still uses the byte ceiling. DaVinci has no context-overflow
  recovery path, so a calibrated (smaller) admission estimate could dispatch
  an over-window request. Calibrate from provider-reported input tokens only
  together with an overflow retry.
- Prepared-context revisions still hash history on every warm lookup (a few
  times per request; 1.7-4.9 ms at 100-300 turns in the September 19 table).
  Removing them needs proof that nothing mutates the public fields between
  lookups; the measured cost does not justify a new invalidation scheme.
- Remote compaction stays off: the typed remote-v2 shape is unprobed on the
  ChatGPT backend and no authenticated call was authorized.

### Remaining gates

Live matched runs against Codex (same model, effort, fixtures and checks),
provider-reported cached/uncached input, real summarizer fold quality,
latency, and native reasoning continuity in active mode (historical
assistant reasoning is not replayed there). The default stays off.

## September 19, 2026: hardening measurements

Measured on this Windows host in an optimized release build, September 19, 2026.
Baseline source: main 1106b1be2bffa1fb7de6e8c0708cc59d1ca2e576.
These are deterministic local fixtures. No live API calls, paid model
evals, cross-platform CI run or terminal transport measurements are implied.

### Context preparation: 16 paired samples per history size

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

### Graph: 32 samples per size

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

### Reproduction

The October 10 scripted session runs in a debug build with
`cargo test -p davinci-agent --test context_vm_efficiency -- --ignored --nocapture`
(`TRACE_LCP=1` prints per-turn bytes). For the September 19 runs:

Run the following from the repository root with RTK installed. These are the
commands used for this run. Offline mode needs cached Cargo dependencies. These two ignored tests emit JSON and assert their invariants.

```sh
rtk proxy cargo test -p davinci-agent --release --test context_vm_performance --offline --locked -- --ignored --nocapture
rtk proxy cargo test -p davinci-tui --release --lib graph_dag_performance --offline --locked -- --ignored --nocapture
```

### Correctness and delivery verification

The workspace test run covered 88 suites: 4,727 passed, one failed and 38 were
ignored. The sole failure was an ample-budget cache-affinity fixture whose
1,000-token ceiling no longer admitted its optional broker item under the
stricter accounting. The fixture now uses 10,000 tokens and explicitly checks
that the broker item is present; its tight-budget checks remain. All four tests
in that target passed on the focused rerun. No production code changed after the
workspace run. The later permission-mode cache regression also passed.

Workspace all-target check, Clippy with warnings denied, formatting, 37 Node
tests and the Context VM and graph release benchmarks passed. This is a full run
plus focused reruns, not a claim that the original workspace command was green.

Independent code/security reviewers passed their reviewed scopes. The performance
reviewer checked the Context VM and graph measurement tables against recorded
output and confirmed their stated limitations.
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
The implementation contract and caveats are in [Context VM](context-vm.md).
