# Recovery incident and deterministic safety evidence

## Historical incident

Artifact: `luna-high-recovery-02/vm-off-cm-off+codex/davinci/t6-bookings-r0.stdout.jsonl`
under the local benchmark root.

| Request / observation | Input / cached / output | Boundary |
|---|---:|---|
| 1, L28 | 10558 / 8704 / 107 | Discovery L31–41 |
| 2, L69 | 10981 / 9728 / 28 | Necessary source read L72–74 |
| 3, L133 | 11150 / 8704 / 109 | Applied source edit/transaction L136–138 |
| 4, L159 | 11272 / 8704 / 256 | Valid post-edit read and failed assertion L162–168 |
| 5, L357 | unknown | Failed HTTP attempt, status 200, no usage/completeness |

The fourth check's expectation was wrong after booking an interval. Its failure
does not show the source fix failed. The fifth generation emitted a presentation
`toolcall_end` at L353, then “Stream ended before the tool call was complete”
at L354. No execution start follows. L359–360 contain no executable call;
L361 says no retry; L362 records zero retries and one unknown-usage attempt.

Known first-four consumption: input 43961, cached 35840, uncached 8121,
output 500. Fifth-attempt consumption is additional **unknown**, not zero.
The saved grader passed eight tests, but process exit 1 correctly prevents
overall benchmark success.

Raw provider frames are absent. The saved presentation events cannot establish
physical interruption versus malformed/incomplete provider termination.
Historical causality therefore remains unresolved even though a matching
decoder/classifier defect is reproduced offline.

## Proven corrections

1. The decoder previously recognized generic missing-terminal EOF as retryable,
   but unfinished-tool EOF as terminal. The existing EOF fixture's new
   retryability assertion failed before correction.
2. The reader previously hid malformed preterminal framing behind the
   decoder's generic retryable EOF. A loopback regression failed before
   correction. Typed read causes and framing errors now override that
   classification; executable calls/native continuation are discarded.

The existing completion retry budget/delays and operation engine remain in
place. No streaming tool execution or new recovery loop was introduced.

## Cold-review compatibility corrections

The first available cold review rejected two regressions introduced by the
initial typed classification. Both were reproduced with failing tests before
correction:

- `terminal_provider_errors_preserve_bounded_transient_retry_policy` exercises
  actual Responses `error` and `response.failed` frames. Terminal attempt
  boundaries do not eliminate existing bounded server/rate-limit retries.
  Typed quota, authorization, invalid-request and policy rejection codes veto
  even a misleading transient message. No callable completion is retryable.
- `wrapped_ureq_idle_timeout_is_an_interruption_not_arbitrary_other` identifies
  the pinned ureq 3 timeout inside `io::ErrorKind::Other` through a typed
  downcast, not its text. Arbitrary `Other` remains unknown.
- `idle_timeout_recovery_is_independent_of_first_sse_frame` verifies actual
  loopback idle timeout before and after a complete initial SSE frame.

`terminal_rejection_codes_override_misleading_transient_messages` additionally
checks direct/nested conflicting codes and executable-call vetoes.

The next cold review found that the zero-valid-frame path still returned
untyped framing errors. A malformed first SSE frame at parser column 503 could
therefore look like HTTP 503 to legacy retries. The loopback regression
`malformed_first_sse_frame_never_retries_parser_locations_as_status_codes`
failed before correction, and now checks columns 503 and 429. The typed result
is retained before as well as after the first valid frame.

A neighboring malformed non-SSE JSON body of 503 bytes also triggered the
legacy status regex; a separate cross-provider regression failed before the
parser began preserving `InvalidStream`. Finally, contradictory wrapped
timeouts with `InvalidData` or `PermissionDenied` retain their authoritative
kind, rather than overriding it with timeout metadata. Those negative cases
also failed before the classifier was restricted to ureq's actual `Other`
wrapper. Ordinary wrapped idle timeouts remain retryable in both frame states.

The third review found that the shared terminal finalizer did not apply the
same rejection-code veto as dedicated `response.failed`/`error` events. Both
native `response.done` and whole JSON regressions failed before the finalizer
was corrected. The matrix now covers all five supported terminal event forms,
plus whole-body JSON.

Adjacent negative probes also failed before correction: invalid-prompt/image
policy codes, a declared content-filter reason masked by server-error text,
cancelled/unknown/nonterminal status masked by transient text, and boolean,
numeric or object status fields treated as absent. A malformed status formerly
returned a callable completion and usable native output. Those declarations now
fail closed, and both native-output constructors reject malformed status types.
Existing absent/null legacy status handling remains covered and unchanged.

## Fourth cold review: carrier and precedence gaps

The fourth review rejected G3 (7/10): `response.incomplete` overwrote declared
status vetoes, bare JSON errors lost structured rejection codes, and WebSocket
terminal errors returned only raw text. Three isolated regression runs failed
with exit 101 before correction.

The decoder now preserves cancellation/unknown/malformed status when
normalizing incomplete events. Bare native JSON error envelopes retain direct
and nested code evidence through the shared decoder. WebSocket `error` and
`response.failed` frames also use that decoder and return typed failures, not
raw retry text. Failed responses cannot seed continuation state or reuse their
live socket. The offline matrix checks quota, context, invalid/policy, transient
compatibility, and discarded calls after provisional output. Fresh review is
still required; aggregate passes are not acceptance.

## Fifth review: control compatibility and output-field bypass

The fifth review rejected G3 (7/10). Ordinary typed WebSocket error handling
bypassed existing missing-previous-response and connection-limit recovery.
JSON errors with null or malformed output could still lose rejection evidence;
wrapped envelopes could even return a successful raw-text completion.
Both isolated regressions failed before correction, exit 101.

Recognized structured transport control codes now reach the existing bounded
controller without inventing a stream start. Its loopback test observes two
physical requests, full input after missing-continuation invalidation, and SSE
fallback on pre-generation exhaustion. Acknowledged operation output stays in
the full input; recovery produces no callable completion. Existing counted
operation tests remain the separate proof against effect replay.

Native JSON errors no longer depend on output presence or shape to enter the
shared classifier. Direct and wrapped envelopes cover absent, null, boolean,
string, and array output and reject native-output admission.
The corrected AI unit suite has 426 passes, zero failures, one existing ignore.

This review disclosed exposure to earlier summaries in the documentation.
Its findings were reproduced, but it cannot count as isolated acceptance.
The next safety reviewer receives source/tests and the frozen rubric only.

## Sixth isolated review: error status, malformed WS, terminal cancellation

The isolated source-only reviewer rejected G3 (7/10). The `error` event arm
ignored declared cancellation/unknown/malformed status; malformed WebSocket
JSON could return parser `column 503` through raw retry text; and cancellation
from terminal tool callbacks could retain calls (SSE/JSON) or return success
and seed continuation (WebSocket). The reviewer source-traced these gaps and
ran three existing focused tests, not the new probes.

Four parent regressions then failed before correction, exit 101. The decoder
now applies the status-aware classifier to `error` events. WebSocket read
errors distinguish protocol invalidity from legacy transport failures;
JSON/UTF-8/protocol failures return stripped typed `InvalidStream` messages.
An expanded neighboring-frame probe also failed before its correction:
binary/reserved opcode, RSV bits and invalid control framing cannot be ignored
and subsequently mistaken for retryable EOF.

The reader rechecks cancellation after terminal callback delivery, strips
calls, wire kinds and native message metadata, and returns no native output.
WebSocket aborted returns cannot store continuation or reuse the live socket.
SSE and nonstreaming JSON have the same terminal-callback regression.
The AI unit suite now has 430 passes, zero failures, one existing ignore.
Final affected validation and fresh isolated acceptance remain required.

## Seventh isolated review: envelope shape and reader kinds

The seventh source-only review rejected G3 (7/10). A non-object SSE payload
could be ignored before a callable terminal response, or become retryable
missing-terminal EOF. WebSocket reader `InvalidData` and unknown errors lost
their kinds and entered raw transient text/fallback. The reviewer executed four
existing focused tests; its new probes were source-derived, not executed.

Two parent probes failed before correction, exit 101. Responses event envelopes
now require an object with a nonempty string type; malformed envelopes terminate
with `InvalidStream` before later terminal output can authorize calls or native
history. Scalars, arrays, null, missing type and malformed type are covered.

WebSocket reader invalid/unknown kinds now return typed failed messages both
before and after response start, so neither generation retry nor SSE fallback
can erase the veto. Supported interruption kinds and configured socket idle
timeouts retain the existing bounded policy. Reader fault injection exercises
the actual frame-reader path; it does not claim a real TLS peer was probed.
The corrected AI suite has 432 passes, zero failures, one existing ignore.

## Eighth isolated review: decoder authority and unknown-only streams

The replacement source-only reviewer rejected G3 (7/10). WebSocket kept
reading after decoder envelope rejection, allowing later disconnect/timeout or
control recovery to erase invalidity. Unknown-only SSE received `MissingTerminal`
retry authority despite no recognized Responses event.

Both parent probes failed before correction, exit 101. WS now stops on
`decoder.is_done()`, including rejected envelopes, before further frame reads
or control decisions. Responses decoding records recognized event evidence.
Unknown-only observed streams fail closed while extension events remain
ignorable around recognized Responses events.

Neighboring WS unknown-only EOF/close and SSE unknown-only timeout probes also
failed before correction. Those paths retain `Unknown`, not transient retry
authority. Existing no-frame connection fallback and recognized interruption
compatibility remain. The final AI unit checkpoint is 436 passes, zero failures,
one existing ignore; final affected tests and fresh isolated review are pending.

## Ninth isolated review: declared payload types

The ninth source-only reviewer rejected G3 (8/10). Recognized event names with
explicitly malformed item/delta payloads could authorize missing-terminal
recovery; a boolean terminal response could panic during status normalization.
Both parent probes failed before correction, exit 101.

Selected declared fields used for item/call assembly, event indexes, deltas,
argument/input completion, response output and error decoding are now
shape-checked before recognized-event authority. The native constructors reuse
the same guards, so malformed declared fields cannot seed replay output.
Unknown opaque fields/items and missing legacy fields retain compatibility.

Neighboring malformed-error and WS control-payload probes also failed before
their guards. Control recovery now validates the complete declared event
payload before acting on its code. The corrected AI checkpoint has 439 passes,
zero failures, one existing ignore. Final affected tests and isolated verdict
are pending.

## Tenth isolated review: status, declared rejection and parser arbitration

The tenth source-only reviewer rejected G3 (7/10): malformed status on a
preterminal response could establish recovery evidence; completed status could
mask declared content filtering and admit calls/native state; and malformed
Responses output could fall into Chat Completions tool parsing. All three parent
probes failed before correction, exit 101.

Shared declared-status/rejection vetoes now run before event authority, including
preterminal events, and native admission applies them across the captured
attempt. Explicit error/failed/incomplete events cannot be promoted into
successful native output by a later terminal record. Malformed declared
Responses fields select the Responses validator rather than a foreign tool
parser. A neighboring declared-rejection/legacy-choice probe also failed before
its guard.

The existing well-shaped Bedrock fixture parser and content-filter error wording
remain compatible; their existing tests caught initial regressions and pass
without relaxed assertions. Corrected AI units: 443 passes, zero failures,
one existing ignore. Final affected validation and independent verdict pending.

## Round-11 review: native continuation and fixture cancellation

The eleventh isolated reviewer rejected G3 (7/10): malformed
`incomplete_details.reason` could admit tool/native output; an incomplete
Length-only WebSocket response could seed previous-response continuation; and
fixture WebSocket results could retain calls after callback cancellation.
Three regressions failed before correction, including a valid function-call
fixture that reproduced the cancellation result.

The decoder and native constructors validate declared incomplete details and
tool-item status before admission. Live WebSocket reuse and continuation
storage now require admitted `NativeResponsesOutput`; incomplete output cannot
extend a native prefix. Fixture processing receives the abort callback and
strips calls/native state after cancellation, including cancellation before
transport or fixture handling. A direct fixture caller was updated to pass no
abort callback; its assertion was not weakened.

The initial broad test lane exhausted Windows memory during parallel Rust
compilation (`STATUS_STACK_BUFFER_OVERRUN` and allocation failures). That is an
environmental validation failure, not a passing checkpoint. The lane was
restarted with `-j 1`; its final result is recorded in [validation](validation.md).

## Round-12/13 review: terminal envelope status authority

A fresh source-only review found that a malformed top-level terminal `status`
could be ignored while nested `response.status` was `completed`. An added
regression first exposed a second issue: native output admission accepted that
same envelope, and the decoder classified the malformed declaration as
`Unknown` instead of `InvalidStream`. Both paths now veto malformed envelope
status before tool dispatch or native continuation admission.

A second source-only review found string status contradictions, such as
top-level `cancelled` with nested `completed`. A further review found explicit
top-level `null` had the same authority gap. Present terminal envelope status
must now be the string expected for that terminal event. Regressions cover
numeric, contradictory-string and null declarations, assert no calls/native
output, and pass in the final AI unit suite. The established direct
non-streaming response compatibility for absent/null nested status was kept;
it was not broadened by this envelope-level correction.

The latest independent source-only reviewer accepted the final status paths
and found no high-confidence tool-dispatch/native-continuation bypass. It ran
no tests; automated validation is recorded separately. Earlier rejections and
the parallel-build failure remain part of the evidence and are not erased by
this acceptance.

## Eleven fault boundaries

All named tests are deterministic and offline. Source paths are relative to
`crates/`; multiple scenarios share the existing facilities.

| Scenario | Regression evidence |
|---|---|
| 1. EOF before completed item | `davinci-ai/src/stream_decoder.rs::preterminal_eof_matrix_drops_all_provisional_calls_before_retry` |
| 2. EOF midway through arguments | Existing `eof_before_response_completed_is_an_error_that_keeps_text` now asserts retryability and discarded calls; `davinci-agent/tests/operation_publication.rs::preterminal_generation_retry_never_executes_partial_mutation` counts one actual effect only after valid regeneration |
| 3. Malformed arguments with normal termination | `malformed_completed_arguments_preserve_validation_without_generation_retry`: preserved failed result, no generation retry, zero actual effects |
| 4. Completed read versus acknowledged read | Decoder matrix discards streamed completed item without terminal response; `acknowledged_read_is_retained_without_reexecution_after_generation_disconnect` counts one actual read in returned/raw failure paths |
| 5. Immediately before mutation dispatch | `cancellation_immediately_before_effect_dispatch_has_zero_actual_effects`; cancellation at tool execution start blocks the physical effect |
| 6. Effect executed before acknowledgment | `uncertain_journal_effect_blocks_dispatch_and_fresh_call_id_bypass`: one real effect, unresolved journal attempt, new-ID dispatch blocked; existing `operation_crash` tests count endpoints across effect-latch/result/publication crash points |
| 7. Acknowledged effect before next response | `acknowledged_effect_is_not_reexecuted_during_returned_or_raw_generation_retry`: one durable physical effect, one persisted result, acknowledged outbox, association retained after reopen |
| 8. Missing final usage | `davinci-ai/src/stream.rs::missing_usage_is_not_backfilled_by_later_complete_generation`: failed and successful no-usage attempts remain unknown after a complete later attempt; missing usage alone does not retry |
| 9. Retry exhaustion | `generation_retry_exhaustion_preserves_terminal_failure_in_both_return_forms`: three calls/two retries, terminal failure retained, zero effects |
| 10. Cancellation during recovery | `cancellation_at_retry_start_never_issues_another_generation_or_effect`: deterministic event-triggered cancellation, one request, no success receipt/effect, both return forms |
| 11. Revocation during recovery | `permission_revocation_during_generation_retry_blocks_fresh_call_ids`: current authority denies new ID, zero effects, no restored session grant, both return forms |

Additional invalid-data fixtures distinguish malformed SSE, invalid UTF-8,
oversized frames, typed supported I/O interruption, unknown I/O kind and
normally terminated unfinished calls. Unknown/malformed classification cannot
fall back to transient-looking text. Cancellation and quota/policy limits
remain higher-priority vetoes.

The mutation tests count persisted physical effects, not only tool events,
attempt records or identical final file contents. The read test counts actual
reader invocation. Journal-backed extension capabilities are explicitly
registered; unknown legacy tools are not silently promoted into journal
operations.

Generation cannot naturally disconnect “during dispatch”: this architecture
finishes generation before dispatch. Scenarios 5 and 6 therefore inject faults
at the actual execution boundary rather than inventing streaming execution.
An uncertain journal operation can allow one generation but still rejects its
unsafe fresh-ID dispatch. Do not claim every uncertainty blocks model calls.

## Remaining limitation

Unstructured completion/WebSocket transport errors do not gain typed provenance.
Their existing bounded policy is tested, not broadened. There is no claim of
universal deduplication across fresh model IDs, or proof that the historical
t6 run would recover. Live reliability, consumption and performance were not
measured after correction.
