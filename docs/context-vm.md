# Context VM

The [October 2 keep/cut record](readiness/openai-harness-implementation.md)
retains the off default. Compaction/resume fixtures prove boundary behavior,
not a long-task success or efficiency advantage for active mode.

Context VM compiles a bounded working set from authoritative session history.
The default mode remains off; shadow compares projections and active mode uses
the compiled image. The agent API selects the mode with
`set_context_vm_mode(ContextVmMode::Active)`.

## Provider admission and protocol

`ProviderContextBudget` in `davinci-agent/src/provider_budget.rs` is the
single budget authority. It subtracts the full system prompt, serialized tool
schemas, reasoning/output reserves and a framing margin from the model window.
The resulting working-set budget is passed to the compiler. Native and JS
provider dispatch also receive the matching output limit.

Token accounting uses conservative UTF-8/serialized-byte ceilings with framing
allowances. These are admission estimates, not tokenizer measurements. They
can reject context that a model-specific tokenizer would fit. Fold decisions,
`/context` and `estimated_context_tokens()` use the four-bytes-a-token
heuristic of the off mode and the compaction threshold instead; measured
against the threshold, the ceiling fired folds at about a quarter of the
configured fill. A request whose required context fails admission always
gets one fold before it is blocked.

The checkpoint, mandatory policy/broker context, latest complete user request,
newest event and complete live tool exchange are non-droppable. Optional
history must fit around them. If required state cannot fit after a fold, the
active run loop returns a budget error before calling the provider. A remaining
page-storage or compilation failure also blocks dispatch. Authoritative
messages are retained. The nonfallible `messages_for_provider()` accessor
keeps its legacy inspection fallback; the active run loop gates dispatch first.

Historical tool results become wrapped custom data context. Only a complete
current assistant/tool suffix preserves provider tool-call/result messages,
with nonempty, unique, matching IDs. Malformed or partial exchanges become
evidence instead. OpenAI Responses wire tests cover both paths.

## Semantic state and materialization

A fold uses the existing configured `Summarizer`, the structured
`CheckpointProposal` prompt/parser and provenance validator. Active goals,
constraints, strategies, blockers, decisions, files and verification evidence
support explicit resolve/supersede/reject transitions. A transition names its
previous value and newer supporting source; corrections cannot silently
replace unrelated state. Recent retirement evidence is bounded to 32 entries.

Omitted slots preserve parent values. User constraints and goals cannot be
created from assistant speculation. Policy transitions require policy evidence;
user corrections require user evidence. Each accepted value has valid source
references, bounded length and provenance. Invalid additions/transitions are
ignored. Full source history remains retrievable.

If no summarizer is configured, its request cannot fit, or its response is
invalid/unavailable, folding retains conservative deterministic excerpts. This
fallback does not infer completion and can grow until admission fails. Real
semantic compression requires a functioning configured summarizer; it is not
claimed from the deterministic fallback or fixture evals.

The deterministic state is the requirement ledger of last resort. A user
message over 512 bytes keeps its opening and every clause that reads as an
instruction (never, must, only, instead, correction, ...), word for word, up
to 1 KiB. Tool evidence keeps its head and its tail, where test runners print
the verdict. The goal list keeps the first request and the newest half; older
goals that state an instruction outlast routine ones. Sources stay
retrievable.

A fold sends the summarizer the parent state and only the events after
`folded_through_seq`, the newest event an earlier fold saw. Validation still
sees every event, so accepted values may cite older sources. A fresh or
rebuilt root starts at zero and sends everything.

New updates materialize one current checkpoint rather than chaining full-state
copies. Legacy delta objects remain readable; subsequent updates collapse them.
Branch changes rebuild state from the selected authoritative history. Cache
pages are derived artifacts, not the source of truth.

## Cache-stable image

The hot window keeps its first event while everything from it still fits
`hot_event_tokens`, so consecutive requests share every earlier message. When
it overflows, or when the compile budget rather than the window bounds the
image, it restarts at half of what fit. A window that slid one event per turn
rewrote the provider input from its first message on every request.

Content that changes on routine turns comes after the hot events: the delta
and optional broker items not marked stable. The delta renders as a
`state_update` with only the values the model cannot see elsewhere: not in the
checkpoint at the head of the image and not fully backed by hot events it
carries verbatim. It is omitted when empty. Admission still reserves the whole
delta page, which stays retrievable by id.

Automatic delta-depth and delta-token folds wait until events have left the
hot window without a fold seeing them (`evicted_unfolded_tokens`), and are
skipped without a summarizer: before that the model sees every event, and a
deterministic fold only rotates the cache epoch. Window pressure, the hard
admission limit, phase boundaries and manual folds are not held back.

## Prepared images and storage

`PreparedContextImage` holds an immutable `Arc<ContextImage>` or its
preparation error. Token estimation, provider messages, manifests and cache
affinity reuse it when inputs match. Messages/session branch, system prompt,
broker overlays, Living Plan, tool/permission surface, model/budget and VM root
participate in revision invalidation.

Public mutable agent fields remain compatible, so revision checks still hash
the current inputs. Warm access is O(history bytes), not O(1); it avoids repeat
branch projection, page loading, image construction and large image clones.
The owned compatibility accessor can still clone the image.

For a bound JSONL session, VM event records retain metadata and hashes, not a
second copy of every event body. Source retrieval streams the authoritative
session file and checks session identity, entry ID and projected-content hash.
Transient/unbound sessions retain bodies so retrieval still works. Agent
messages, session entries and page caches have their own memory costs; removing
VM body duplication does not make total process memory constant.

## Metrics

| Counter | Meaning |
| --- | --- |
| `page_lookup_hits/misses` | Actual derived-page lookup results |
| `rebuild_attempts/successes/failures` | Reconstruction from authoritative events |
| `semantic_page_faults` | Valid explicit pageable-context recovery requests |
| `retrieval_hits/misses` | Success/failure of those retrievals |
| `images_compiled` | New compiled images, excluding warm prepared-image reuse |

`context_recovery_rate()` is retrieval hits divided by semantic page faults
(zero when no requests occurred). Legacy page-fault counters follow semantic
retrieval only. A successful rebuild after a missing page remains a lookup miss.

## Shared engineering facts

`EngineeringSnapshots` is owned by the native extension host. Test Impact
and Change Impact reuse repository indexing and workspace metadata; Verification
Planner and Build Intelligence reuse available metadata without initiating a
snapshot scan.

Native mutations and new turns invalidate the snapshot. Reuse checks workspace
identity, known file/directory stamps and existing read permissions; indexed
consumers also drain repository observations. Missing or stale data falls back
to the consumer's normal checks. Stamp checks are not cryptographic detection
of an externally modified file whose size and timestamp were preserved.

Git dirty state is typed `Clean/Dirty/Unknown`; this snapshot currently
reports `Unknown` without an authoritative Git observation. Missing
build/LSP/transaction revisions remain absent. The snapshot shares facts already
produced by these consumers; it does not claim every subsystem has been
consolidated into one scan.

See [measurements and validation](context-vm-measurements.md) for the
deterministic eval corpus, release benchmarks and remaining live-provider gates.
