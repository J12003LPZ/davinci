# Validation record

Date: 2026-10-12. Rust 1.88.0, exact locked dependencies, offline.
All commands ran from the task checkout through `rtk proxy`.
Clippy passed with warnings denied. Independent cold review remains pending at
this checkpoint.

## Before/after regression evidence

| Correction | Original-source result | Corrected focused result |
|---|---|---|
| Unfinished-call EOF retryability | Existing decoder fixture with added retry assertion fails | Decoder/unit suite passes |
| Masked malformed preterminal stream | Loopback no-retry assertion fails | Malformed/invalid-data stream fixtures pass |
| Cache report privacy | Three synthetic-secret regressions fail | Ten report tests pass |
| Environment assignment discovery bypass | Two regressions fail; completion evidence incorrectly becomes Verified | Both regressions plus runner/zero-test evaluation pass |

Commands actually executed for those failures:

```powershell
rtk proxy python -m unittest scripts.bench.tests.test_cache_report
rtk proxy cargo +1.88.0 test -p davinci-ai --lib stream_decoder::tests::eof_before_response_completed_is_an_error_that_keeps_text --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib stream::tests::malformed_preterminal_responses_stream_is_not_retryable_eof --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-agent --lib environment_prefixed_pytest --offline --locked
```

## Initial completed affected suites

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai -p davinci-agent -p davinci-coding-agent --offline --locked --quiet
```

**4,857 passed, zero failed, 56 ignored across 172 target reports.**
The completed terminal log contains all target summaries and final doc tests.
Ignore counts are existing tests, not silently removed failures.
This is the pre-cold-review checkpoint. The final corrected suite is rerunning;
this count does not yet certify the two subsequent compatibility corrections.

This includes actual affected crate suites and the planned cache manifest,
diagnostic/accounting, Hybrid/replay, Lean surface, operation journal,
property/crash/idempotency/transaction/recovery, runtime inspector and
end-to-end recovery targets. No protocol/session storage representation changed;
native-output admission now rejects malformed declared status types. Agent and
coding-agent consumers/reload fixtures are in the full affected lane. Separate
full-workspace testing was not needed.

Before source correction, the AI baseline and agent baseline also passed.
The new negative regressions were then run against original behavior before
each correction, rather than assuming the aggregate baseline detects the gap.

Additional focused runs passed:

- Full AI unit suite after scoped sentinel fix.
- Hybrid: 13 tests, including full prepared bytes and valid selected native
  resume tapes across reload.
- Operation publication: 18 tests, including the eleven-fault coverage map in
  [recovery](recovery.md).
- Real-serializer cache manifest: three tests.
- Discovery bypass and assignment evaluation: three tests.

## Reporting and document checks

```powershell
rtk proxy python -B -m unittest discover -s scripts/bench/tests -p "test_*.py" -q
rtk proxy cargo +1.88.0 fmt --all -- --check
rtk proxy git diff --check
```

Python: **188 tests, zero failures, five skips**. Formatting and diff whitespace
checks pass. Git warns about its configured LF→CRLF conversion on some Rust
files; that is not a test failure.

Document pass 1: source/fixture inspection and independent diagnosis references.
Pass 2: deterministic arithmetic, all 12 candidate field sets/decision enums,
and relative-link existence checked. D4 per-request sums match
191572 / 130560 / 2326. Pass 3: sanitized reporting actually runs on the
existing captured t4 sequence, preserving all ten append-only rows and the
stable transition with zero cache reuse. No new raw capture was created.

New instructions/examples are offline invocations already executed above.
No newly added external link is required to reproduce local checks.

## Clippy and existing offline evaluation

```powershell
rtk proxy cargo +1.88.0 clippy -p davinci-ai -p davinci-agent -p davinci-coding-agent --all-targets --offline --locked -- -D warnings
rtk proxy cargo +1.88.0 test -p davinci-agent --test context_vm_efficiency --offline --locked -- --ignored --nocapture
```

Clippy completed successfully. The explicitly ignored offline structural
Context VM report passed as one separate test, covering ten scripted cases.
At 300 turns, Hybrid reports two compactions and 883/883 requirement checks
with and without a summarizer. At 60 turns it reports no compaction and
163/163 checks. The intentionally empty legacy summary loses requirements
in Off at 300 turns; this is existing fixture behavior, not a new regression.

This report measures serialized input bytes and synthetic usage, excludes
instructions/schemas, and does not assert a live performance threshold.
Its passing test means the report completed, not that every mode achieves
equivalent quality. It cannot establish token cache reuse, speed or subscription
savings. The new full-request parity fixtures provide the separate integrity
gate.

## Faults encountered while building the fixtures

These are resolved, not remaining product failures:

- An initial Responses-sentinel guard also rejected valid Chat Completions.
  Two existing tests caught it. The guard is now Responses-only.
- A usage fixture supplied the wrong cache-write key. Its completeness
  assertion caught it; the fixture now uses the existing supported key.
- A counted extension initially used the legacy compatibility ledger instead
  of an explicitly registered journal capability. The journal-count assertion
  caught the fixture error; actual counted effects now have journal authority.
- An uncertainty fixture initially expected zero generation calls. Existing
  behavior allows generation but blocks unsafe dispatch. The test and report
  now state that boundary accurately.

## Skips and authorization boundaries

The default Rust lane leaves paid/live, explicit performance, platform/tool
prerequisite and manual acceptance tests ignored. The existing offline
Context VM structural report was separately executed without providers.
Remaining live feature/competitor evals are specifications, not measured
performance acceptance.

Source ignore reasons were inspected, not guessed:

- AI: explicit release-mode telemetry overhead and large-call performance
  budgets.
- Agent: cache acceptance, Context VM latency/structural measurements,
  transaction/process/operation-capacity baselines and isolated verification
  timing. Linux bubblewrap is platform-gated out of this Windows build.
- Coding agent: admitted Node/browser/sandbox packages, provisioned language
  servers/SARIF Python, explicit offline timing/cache/repository-impact evals,
  fresh CLI/graph-worker executable fixtures, live subscription/reviewer
  probes and plugin networking. Subprocess fixtures marked ignored are invoked
  by their ordinary parent tests, so ignore counts are not necessarily wholly
  unexecuted behavior.

The five Python skips on Windows are the fork/setsid descendant fixture, the
actual unsupported-native-supervision environment fixture (Windows supports
native supervision), Linux waitpid/reaping, Linux crash/reaping, and POSIX
permission-bit checks. Windows Job Object coverage runs instead of the Linux
descendant fixture. No required new regression was ignored or skipped.

No live provider probes, subscription-consuming benchmarks, release install,
installed-binary replacement, deployment, commits, pushes or merges occurred.
All saved benchmark evidence remained read-only. The user-modified `AGENTS.md`
remains out of scope. No running benchmark campaign was managed.

The first independent review failed before producing a verdict because its
model provider was unavailable. A fresh cold read-only review was launched;
the failed review is not counted as a safety pass.

The available cold review **rejected** the initial correction for lost
transient-terminal retries and wrapped idle-timeout retries. Three new
decoder/classifier/loopback regressions each failed (exit 101) before correction.
After correcting the production paths, 413 AI unit tests passed, zero failed,
one ignored. An additional denial-code/callable-result regression and final
affected-suite/Clippy reruns are now scheduled. A new independent verdict is
required; the first rejection is retained, not replaced by self-approval.

That subsequent checkpoint completed: **4,861 passed, zero failed, 56 ignored,
172 target reports**, plus strict Clippy/formatting/diff checks. The second
review then rejected the remaining zero-frame framing bypass. This count is
another intermediate checkpoint, not final acceptance.

The supplied first-frame parser-location regression failed before correction.
Parent also reproduced a neighboring malformed JSON-body size/status confusion
and contradictory typed-timeout/kind priority, each with a failing assertion
and exit 101. After correction, **416 AI unit tests passed, zero failed, one
ignored**. A final full lane and fresh review must certify this last source
state before `CORRECTION_VERIFIED`.

That checkpoint completed: **4,863 passed, zero failed, 56 ignored, 172 target
reports**, with strict Clippy/formatting/diff checks. A third review rejected
structured rejection-code loss in the shared finalizer.

Native-done and JSON quota-veto regressions failed before its correction.
Additional probes failed for invalid/policy aliases, masked content filtering,
cancelled/unknown terminal state, callable malformed status, and both native
output constructors accepting malformed status types. Each got an isolated
negative run before correction. **421 AI unit tests pass, zero fail, one
ignored** at the new source checkpoint. Final affected-suite/Clippy and fresh
cold acceptance are pending.

The subsequent checkpoint reports **4,868 passed, zero failed, 56 ignored,
172 target reports**, and completed strict Clippy. The fourth review rejected
three remaining structured-error carrier/precedence gaps against G3 (7/10).
This is a pre-round-4 checkpoint, not final acceptance.

Each new carrier regression failed before correction, exit 101:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib incomplete_event_preserves_declared_status_vetoes --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib bare_json_error_preserves_structured_rejection --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib websocket_terminal_errors_preserve_structured_rejection --offline --locked
```

After the bounded carrier corrections, **424 AI unit tests passed, zero failed,
one ignored**. The next full affected lane and independent review will certify
the corrected checkpoint. No live measurements are included.

That checkpoint completed with **4,871 passed, zero failed, 56 ignored across
172 target reports**, plus strict Clippy. The fifth review then rejected
bounded WebSocket control recovery and JSON output-shape bypasses.

Both new regressions failed before correction with exit 101:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib json_error_output_shape_cannot_bypass_structured_rejection --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib localhost_structured_control_errors_keep_bounded_recovery --offline --locked
```

The initial fixture compile attempt lacked two imports and was corrected before
the actual failing assertions above. After correction, a fixture expected a raw
error on exhaustion; source inspection confirmed existing pre-generation SSE
fallback instead. That assertion was corrected without changing production
policy. **426 AI unit tests passed, zero failed, one ignored**.

The fifth reviewer saw prior summaries in the documentation, so its rejection
is retained but not counted as an isolated verdict. A fresh source-only safety
review and final affected-suite/Clippy lane are running.

On resumption, the shared checkout unexpectedly pointed to `main` at the same
base commit. Work stopped before writes. The user explicitly authorized switching
back to `harness-gap-diagnosis` and confirmed no other session used the checkout.
All uncommitted changes were preserved; no commit or push occurred.

The round-5 full lane completed: **4,873 passed, zero failed, 56 ignored,
172 target reports**, strict Clippy/format/diff checks, with its final success
sentinel recorded. The isolated sixth review rejected three remaining G3
paths (7/10). These counts do not certify the subsequent source.

Four new focused probes failed before correction (exit 101):

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib error_events_preserve_declared_status_vetoes --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib websocket_malformed_frames_are_typed_nonretryable_failures --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib websocket_cancellation_from_terminal_callback_strips_calls --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib cancellation_from_terminal_callback_strips_calls_and_native_output --offline --locked
```

Expanded invalid-frame neighbors failed a second isolated WS probe before
their bounded frame-validation correction. The final AI unit checkpoint is
**430 passed, zero failed, one ignored**. The full affected lane and a new
isolated source-only review are running. No live measurement occurred.

The round-6 lane reports **4,877 passed, zero failed, 56 ignored across
172 target reports**, with completed strict Clippy. The seventh isolated review
rejected non-object SSE payloads and erased WS reader kinds (7/10).

Both new probes failed before correction, exit 101:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib websocket_reader_kind_vetoes_survive_before_and_after_start --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib nonobject_responses_sse_never_authorizes_terminal_calls_or_retry --offline --locked
```

The expanded corrected matrix includes invalid/unknown reader kinds before and
after stream start, supported timeout/reset compatibility, malformed event
envelopes, and scalar payloads with/without callable terminal output.
**432 AI unit tests passed, zero failed, one ignored**. Final affected
validation and isolated review are running; no live TLS/provider probe was
performed.

## Completed round-7 offline source checkpoint

The final round-7 lane completed with **4,879 passed, zero failed, 56 ignored
across 172 target reports**. Strict Clippy with warnings denied, formatting and
diff whitespace checks passed. The log ends with
`FINAL_OFFLINE_GATES_PASSED`, not merely a progress estimate.

Terminal log:
`C:\Users\sergi\AppData\Local\Temp\droid-bg-1791816626586.out`.
Independent source-only review `ed67664e-455e-4a00-a53d-9b87a7527c46`
failed with `model_provider_unavailable`, producing no verdict. Replacement
review `19b66144-6dfb-44c1-a1b4-bfd3c06f39b4` rejected two remaining
decoder-authority gaps (7/10). This checkpoint is not final acceptance.

Retained review/reproduction records live under
`C:\Users\sergi\AppData\Local\Temp\harness-gap-execution\critique\round-1.md`
through `round-8.md`. They retain rejection findings and distinguish
source-traced reviewer hypotheses from actually executed parent probes.
The fourth record was reconstructed from the resumed summary and says so;
the fifth disclosed prior-summary exposure. Neither limitation is hidden.

Python reporting was separately rerun: **188 tests OK, five existing Windows
skips**. Document structure checks cover all 12 candidate records, 13 consistent
fields, decision enums and 17 relative links. Scripted D4 request sums and
uncached counts match the report. No live efficiency acceptance is claimed.

## Round-8 regressions and pending acceptance

The parent reproduced WebSocket reading past decoder rejection and unknown-only
SSE EOF classification before correction. Neighboring WS unknown-only
disconnect and SSE timeout-precedence probes also failed before their fixes.
All four isolated runs exited 101:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib websocket_decoder_rejection_precedes_later_transport_or_control --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib unknown_only_responses_events_do_not_establish_retry_authority --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib websocket_unknown_only_events_cannot_authorize_disconnect_retry --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib unknown_only_sse_timeout_cannot_grant_interruption_retry --offline --locked
```

After correction: **436 AI units passed, zero failed, one ignored**.
Recognized streams retain missing-terminal/interruption policy; extension
events remain ignorable alongside recognized events. Unknown-only observed
events cannot gain retry authority from timeout/disconnect, and decoder
rejection is terminal before subsequent WS reads or control recovery.
The final affected lane and isolated review are running.

## Round-9 declared-payload validation

The completed round-8 lane reports **4,883 passed, zero failed, 56 ignored
across 172 targets**, with completed strict Clippy. The ninth isolated reviewer
rejected malformed declared payloads establishing recognized-event authority
(8/10), and identified an inherited boolean-response normalization panic.

Both new regressions failed before correction, exit 101:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib malformed_known_payloads_never_establish_retry_evidence --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib malformed_terminal_response_returns_invalidity_instead_of_panicking --offline --locked
```

The expanded known-payload matrix then exposed malformed declared error shapes
with transient text; that isolated run failed before its additional guard.
A neighboring malformed WS control-payload probe also failed before correction:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib malformed_control_payload_cannot_bypass_decoder_validation --offline --locked
```

After correction: **439 AI units passed, zero failed, one ignored**.
The decoder, WS control path and native-output constructors share declared
payload guards. Missing legacy fields and opaque future items remain compatible.
The final affected lane and fresh isolated review are running.

## Round-10 status/rejection and parser arbitration

The round-9 lane reports **4,886 passed, zero failed, 56 ignored across
172 targets**, with completed strict Clippy. The tenth isolated reviewer
rejected preterminal malformed status, content-filter rejection masked by
completed status, and malformed Responses output falling through to Chat
Completions call parsing (7/10).

Each new probe failed before correction, exit 101:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib preterminal_declared_status_vetoes_survive_eof_and_terminal_output --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib declared_content_filter_precedes_completed_callable_output --offline --locked
rtk proxy cargo +1.88.0 test -p davinci-ai --lib malformed_responses_fields_cannot_use_chat_completion_tool_parser --offline --locked
```

A neighboring declared-rejection/legacy-choice parser probe also failed before
its arbitration guard:

```powershell
rtk proxy cargo +1.88.0 test -p davinci-ai --lib declared_rejection_blocks_legacy_choices_parser --offline --locked
```

The initial corrected unit run exposed two compatibility regressions: routing
the existing Bedrock-shaped fixture as Responses and changing the content-filter
error wording. Both existing assertions were preserved and production behavior
corrected, not relaxed. Explicit malformed Responses fields and declared vetoes
still prevent fallback into callable legacy parsing.

After correction: **443 AI units passed, zero failed, one ignored**.
Final affected validation and fresh isolated acceptance are running.

## Final continuation-admission and terminal-status corrections

Round 11 exposed incomplete-details admission, Length-only WebSocket
continuation, and fixture cancellation gaps. Subsequent fresh source-only
reviews exposed malformed, contradictory-string, and null top-level terminal
status bypasses in decoder and native-output admission. The corrections and
their regression evidence are retained in [recovery](recovery.md).

The first broad round-11 build attempted parallel Rust compilation and failed
under Windows memory pressure, including allocation failures and
`STATUS_STACK_BUFFER_OVERRUN`. It is not counted as a test result. Re-running
serially succeeded:

```powershell
cargo test -p davinci-ai -p davinci-agent -p davinci-coding-agent --offline --locked -j 1 --quiet
cargo test -p davinci-ai --offline --locked -j 1 --quiet
cargo clippy -p davinci-ai -p davinci-agent -p davinci-coding-agent --all-targets --offline --locked -j 1 -- -D warnings
```

Final three-crate run: **4,894 passed, zero failed, 56 ignored across 172
targets**. The exact final AI crate run passed **447**, zero failed, one
existing ignore. Strict Clippy passed on all affected targets with warnings
denied. The fresh independent source-only reviewer accepted terminal status
arbitration and found no high-confidence tool-dispatch/native-continuation
bypass; it ran no tests, so it supplements rather than replaces the automated
results.

No provider calls, paid evaluations, or efficiency measurements were used.
The run is serial specifically to avoid the observed parallel-build memory
failure. The earlier Python, formatting, and document checks remain recorded
above; final formatting/diff/document integrity checks are repeated after
updating this evidence package.
