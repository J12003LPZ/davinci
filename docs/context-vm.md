# Context VM

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
can reject context that a model-specific tokenizer would fit.

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

New updates materialize one current checkpoint rather than chaining full-state
copies. Legacy delta objects remain readable; subsequent updates collapse them.
Branch changes rebuild state from the selected authoritative history. Cache
pages are derived artifacts, not the source of truth.

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
snapshot scan. Jev uses already available facts in its background worker.

Native mutations and new turns invalidate the snapshot. Reuse checks workspace
identity, known file/directory stamps and existing read permissions; indexed
consumers also drain repository observations. Missing or stale data falls back
to the consumer's normal checks. Stamp checks are not cryptographic detection
of an externally modified file whose size and timestamp were preserved.

Git dirty state is typed `Clean/Dirty/Unknown`; this snapshot currently
reports `Unknown` without an authoritative Git observation. Missing
build/LSP/transaction revisions remain absent. Capability IDs come from exact
registered tool sets, not name substrings. The snapshot shares facts already
produced by these consumers; it does not claim every subsystem has been
consolidated into one scan.

See [measurements and validation](context-vm-jev-measurements.md) for the
deterministic eval corpus, release benchmarks and remaining live-provider gates.
