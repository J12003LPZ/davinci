# Request, recovery and evidence map

Paths below are relative to the repository root. Function names are the stable
source references; benchmark event lines are in [trajectories](trajectories.md).

## Request path

1. `crates/davinci-coding-agent/src/main.rs::complete_prompt_with_host`
   obtains the actual agent projection, developer instructions and authorized
   tools. `live_cache_key` binds the resolved schema, stable instructions and
   permission surface, separately from the transport session.
2. `crates/davinci-agent/src/lib.rs::messages_for_provider`,
   `provider_system_prompt`, and `provider_tool_specs` select the projection.
   Full/Lean exposure and permission changes can legitimately change schemas.
3. `crates/davinci-ai/src/responses_tools.rs::resolve_responses_tools`
   produces the actual ordered contract. Tool sorting already exists.
4. `crates/davinci-ai/src/stream.rs::request_body_with` selects the real provider
   serializer and compatible native resume processing.
5. `native_responses_replay_skip` rejects mismatched projection prefixes or
   stable contracts. `responses_ledger.rs::NativeResponsesTurn::from_prepared`
   retains the prepared request input and acknowledged native output.
6. Streaming adjustment precedes `PreparedProviderRequest::new`.
   `responses_request.rs` freezes the body and builds content-free segment
   manifests. Compression and the authorized transport follow.
7. `provider_observation.rs::Attempt` captures actual send attempts and their
   own raw usage/completeness. A later attempt cannot complete an earlier
   attempt's missing evidence.

Do not conflate cache partition, prompt fingerprint, conversation lineage,
transport lease and request correlation ID. Sessionless execution already
owns an isolated ephemeral transport identity; rebuilding that is unnecessary.

The existing logical wire dump occurs before final streaming preparation.
It is not evidence of final HTTP bytes or headers. Native output, segment
digests and raw JSON byte prefixes are not measurements of model-token prefixes.

## Hybrid contract and its boundary

Off and Hybrid share the legacy provider projection before pruning/compaction.
Neither has Active-mode VM cache affinity. Full serialized fixtures now cover
system/developer instructions, effective sorted schemas, cache identity,
high effort, reasoning-bearing native items, complete call/result pairs,
failed results, repeated reads, requirement correction and verification.

`context_vm_hybrid.rs::complete_prepared_requests_match_through_unpruned_hybrid_history`
checks six real agent-loop completion callbacks. No normalization removes IDs,
native output, authority or meaningful state. It also checks history prefixes,
zero compaction, no pruning, no summarizer calls and no retries.

`complete_requests_preserve_selected_native_branch_across_reload` loads
identical seed sessions in separate temporary storage. Fixed entry IDs and
timestamps remain intact. Real prepared native tapes and selected-ancestry
resume records survive reload, while abandoned native output stays absent.

Important qualification: Off intentionally prunes eligible historical results
at its threshold; Hybrid deliberately does not. Therefore **pre-compaction
alone does not guarantee equality**. The equality fixture asserts the shared
pre-pruning boundary. Do not erase the pruning distinction through
normalization or label it an accidental mismatch.

Existing repeated-compaction, source-retrieval, overflow, reload, fit-budget
and replay tests remain the post-boundary oracle. Summary equality after
compaction is neither expected nor required.

## Stream generation versus operation execution

`stream_decoder.rs::ResponsesDecoder::finish` distinguishes a missing response
boundary from a normally terminated unfinished call. Provisional calls are
presentation only and are discarded before an executable completion returns.

`stream.rs::read_provider_stream` retains typed I/O causes until arbitration.
Malformed framing, invalid UTF-8 and size violations veto the decoder's
retryable missing-terminal classification. A Responses sentinel without its
terminal response is invalid, not a valid completion. Other providers retain
their own sentinel semantics.

`retry.rs::ResponseStreamFailure` is host-generated completion metadata:

| Evidence | Generation retry policy |
|---|---|
| Missing terminal boundary, otherwise valid stream | Existing bounded completion retry, no executable calls |
| Supported typed connection interruption, including wrapped ureq idle timeout | Same bounded generation retry |
| Terminal provider event with a recognized transient message | Existing bounded provider retry compatibility, without executable calls |
| Invalid stream, terminal unfinished call, explicit terminal rejection | No retry through the new stream classification |
| Unknown or unrecognized classification | Fail closed |
| Cancellation, quota/context/policy denial | Existing veto takes precedence |

Native terminal event forms, bare JSON error envelopes, and structured
WebSocket terminal errors now use the shared status/code classifier.
Incomplete-event normalization preserves declared cancellation, unknown, and
malformed status vetoes. A declared content filter or invalid/policy code
cannot be masked by transient error text. Malformed status types cannot become
callable completions or native replay output; existing absent/null legacy
status handling is retained.

The marker is not taken from provider text and is not operation authority.
Legacy unstructured transport errors retain the old classifier; this change
does not pretend they have typed provenance. Structured WebSocket provider
errors return typed failed messages, discard calls, close the socket, and
cannot seed cached continuation. Two recognized structured control codes
retain the existing bounded transport controller: missing previous response
clears continuation and retries full input once; connection limit before
generation retries once. Exhaustion before generation falls back to SSE.
Structured rejection/status vetoes remain stronger than control routing.
Historical raw-error attribution remains
unresolved.

WebSocket malformed JSON/UTF-8 and protocol-frame failures carry typed
`InvalidStream`, not parser-number retry text. Unstructured transport failures
retain their existing policy. All live carrier readers recheck cancellation
after terminal callbacks and strip calls, wire kinds and native metadata
before returning an aborted completion; native output and WS continuation are
not admitted from that completion.

Responses stream envelopes must be objects with a nonempty string event type.
Non-object or malformed-type payloads cannot be ignored before callable terminal
output. WebSocket reader invalid/unknown kinds remain typed vetoes before and
after response start; supported interruptions retain bounded compatibility.
Decoder rejection is terminal for WS reading even if the wire event name was
invalid. Observed unknown-only event corpora do not establish retry authority,
including on a later timeout or disconnect; ignorable extensions around
recognized Responses events retain compatibility.
Declared item/delta/call-completion/index/response-output/error field shapes are
checked before recognized-event evidence. The decoder, WS control recovery and
native-output admission reuse these guards. Missing legacy fields remain
accepted; opaque future fields/items are not reinterpreted as known call data.
Declared status/rejection vetoes apply before preterminal recovery evidence and
before completed output admission. Content filtering cannot be masked by
`completed`. Malformed declared Responses fields and explicit vetoes select the
Responses validator, never a callable legacy-parser fallback. The existing
well-shaped Bedrock fixture representation remains supported.

Terminal envelope-level status, when present, must match the event's terminal
state (`completed`/`done` or `incomplete`). Malformed, null, or contradictory
envelope status is `InvalidStream`, even when nested response status claims
completion; it cannot expose tool calls or seed native continuation. Nested
response status retains its established compatibility rule: absent/null is
treated as omitted, while malformed non-null or non-completed values cannot
admit terminal native output.

`crates/davinci-agent/src/turn.rs::complete_with_retry` retries only generation
before dispatch. Failed attempts do not add executable calls or native resume
state. Acknowledged results persist before the next request.

Operation admission, claims, effect latches, result publication, source-bound
checkpoints and permission revisions remain authoritative. Generation may
occur while an uncertain operation exists, but an unsafe fresh-ID mutation
still cannot dispatch. Do not promise semantic deduplication across arbitrary
new call IDs.

## Verification and accounting

`command_receipt.rs` captures process streams; rendered tool text cannot grant
verification. `runtime/evidence_store.rs::ExecutionReceipt::is_verified_check`
requires supported nonempty discovery when the command is a test runner.

`verification/discovery.rs::runner_words` now reuses the shell classifier's
literal environment-assignment predicate. Decorating a test command no longer
bypasses discovery requirements. Unsupported pytest output remains unknown.
Zero-test, unrelated, stale and nonterminal evidence remains rejected.

`Verification Planner` remains the existing source-bound planner, not a second
completion engine. The follow-up limitation in the saved Python traces is
unsupported discovery and conservative coverage recognition, not a proven
stale-result or Governor projection defect.

The benchmark parser intentionally separates model-issued calls from automatic
verification. For t1, 13 model-issued calls plus one automatic check equals
14 executed leaf operations. State which metric is being compared.

## Safe diagnostics

The Rust manifest already provides safe hashes, sizes and typed segment tags.
Its new real-serializer regression checks deterministic metadata, secret
safety, absent fields and unchanged prepared request bytes.

`scripts/bench/cache_report.py` remains an offline read-only reader. It now
hashes routing keys and differing items, omits paths and untrusted labels,
and fails malformed captures without printing their content/path. It does not
enable raw tracing, persist new captures, add backend parameters or change
runtime authority. Normalized usage sidecars do not prove completeness, so
the report explicitly labels completeness unknown.
