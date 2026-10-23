# Context VM

The [October 2 keep/cut record](readiness/openai-harness-implementation.md)
retains the off default. Compaction/resume fixtures prove boundary behavior,
not a long-task success or efficiency advantage for active mode.

Context VM compiles a bounded working set from authoritative session history.
The default mode remains off; shadow compares projections, active mode uses
the compiled image, and hybrid mode sends the off-mode transcript and compacts
it with the VM ledger. The agent API selects the mode with
`set_context_vm_mode(ContextVmMode::Active)`; `DAVINCI_CONTEXT_VM` and the
`context-vm` setting accept `off`, `shadow`, `active` and `hybrid`.

## Hybrid mode

Hybrid keeps the off-mode request: the append-only transcript with native
items and tool call/result pairs, unpruned, so each request extends the
previous one. It sets no Context VM cache partition. The VM follows every turn
with its deterministic ledger at no provider cost.

Compaction starts when `calibrated_context_tokens()` passes the compaction
threshold: the provider-reported size of the last request (input of every
kind plus the reply) and the four-bytes-a-token estimate of messages added
since, as Codex measures it. A system prompt, tool catalog or other context
outside the messages that changed since the report adds the estimate of its
change; a report from another provider or model is not used. The plain
estimate applies when no report does. A provider that refuses the input as
too long gets one compaction and the same request again per run.

A compaction folds the ledger (one summarizer request over the events no fold
has seen, or the deterministic state when that fails), renders it with every
value's source refs and the superseded values, and keeps it with the most
recent `keep_recent_tokens` of messages verbatim. It is persisted as an
ordinary `compaction` entry, so reload, branching and native replay follow
the off-mode paths. A compaction or branch summary is never read as a user
requirement.

The replacement must estimate under a budget beside the system prompt, tool
schemas and other context outside the messages. The budget is the compaction
threshold for an automatic attempt below the capacity limit. A recovery path
(manual `/compact`, the provider's context-overflow retry, an automatic attempt
whose context has reached the capacity limit) tries the threshold budget first,
with the relaxed assistant-reply rule below. Only when nothing fits under the
threshold does it fall back to the capacity limit, `context_window` minus
`reserve_tokens`: the window is already full, and a replacement that is under
capacity is better than a blocked request. Trying the threshold first matters
because the first cut that fits keeps the most; under the capacity budget alone
a recovery could keep a 12k prompt, land above the threshold, and the next turn
would pay for a second fold.

The overhead taken off the budget is the estimated request overhead (system
prompt, tool schemas, ephemeral context, four bytes a token), never a figure
derived from what the provider counted. A provider that counts the messages
below their byte estimate would otherwise turn that gap into budget, the first
cut would land just under the threshold on the provider scale, and the next turn
would pay for another fold. Before the first fold the overhead also includes the
`retrieve_context` schema, which the first fold adds to every request, unless
the host supplied its own tool overhead (a scalar or an estimator), which
already measures the real tool catalog and is not charged twice.

Because the budget equals the one the hybrid compaction always used, for the
same history the dry run, the assistant-reply rule and the backoff only remove
or defer summarizer requests; none adds one. And because a recovery never uses
the dry run (below), anything the old behavior recovered from, this recovers
from: a session cannot get stuck behind a skip.

The budget can be too small to hold any ledger. With a 14,000-token threshold
and about 13.5k estimated overhead (the live 14k run) it is about 500 tokens,
so hybrid cannot compact at that threshold: the dry run skips for free, every
time. That is a property of the setting, not of the code. Raise the threshold or
shrink the tool catalog.

When the recent messages do not fit beside the ledger, the cut moves to a later
turn boundary (never onto a tool result); when even the newest turn does not
fit, nothing is compacted and the transcript is unchanged.

An automatic attempt below the capacity limit applies one more rule to each
candidate cut: the replaced prefix must contain an assistant reply. Turn 1 is a
user prompt, plus a turn context message on cache-sensitive routes, and nothing
a ledger could stand in for, so replacing it costs a summarizer request for no
gain while the window still has room. A cut that breaks the rule is skipped and
the next later cut is tried; only when every cut breaks it is the attempt
skipped (counter `skippedNoAssistantReply`). Recovery paths do not apply the
rule. With a 90k tool result after a 100k pasted prompt in a nearly full window,
the only cut that fits replaces the prompt, and replacing it with the ledger is
better than a blocked session.

The summarizer request is paid for before the replacement is known to fit, so
an automatic attempt below the capacity limit does not make it blind. A free dry
run comes first: the same fit check, run with the deterministic ledger and
without the parent narrative, which the fold may replace. A recovery (manual
`/compact`, the provider's context-overflow retry, an automatic attempt at or
above the capacity limit) skips the dry run entirely: it pays the fold as it
always did, then fits the result (threshold budget first, capacity budget as the
fallback, assistant-reply rule off). The dry run starts from the ledger the VM holds only
when that ledger continues the current history (the same check `fold_context`
makes); after a branch switch it starts from nothing, so another branch's
ledger cannot block it. It then adds the events no fold has seen, with the
deterministic reducer.

If that replacement cannot fit, there is no earlier turn to replace
(`skippedNoFit`), or the assistant-reply rule leaves no cut
(`skippedNoAssistantReply`, with its own message), the compaction returns not
compacted with a summary that says which, and no summarizer request is sent.

The dry run is not a strict bound, in either direction. The folded ledger is
usually at least as large as the deterministic one (the proposal is merged into
it), so a pass can still fail the real fit after a paid fold; the backoff below
limits that to one paid request per growth step. But the fold can also be smaller:
it replaces the parent narrative with the proposal's (up to 1,024 bytes), drains
retired entries over 32, and a long history is folded in chunks whose goals are
capped again on each pass. The dry run drops the parent narrative to allow for
the first; it does not model the other two. A false skip below capacity only
defers compaction to the capacity limit, where the attempt is a recovery, pays
the fold, and fits it. If the ledger state cannot be loaded for the dry run, the
compaction goes ahead as before.

A compaction that reached the fold and was not applied (the replacement did
not fit afterwards, the fold failed, or the checkpoint could not be saved)
starts a backoff, if it was an automatic attempt below the capacity limit
(`context_window` minus `reserve_tokens`). So does an applied replacement that
is no smaller than the history, which still counts as applied. The automatic
trigger then waits until the calibrated context has grown by `keep_recent_tokens`
past the size at that attempt. The size recorded is the `tokens` value the automatic trigger passed in for that
attempt (the calibrated count), so the later comparison uses one scale. A
context smaller than the recorded size by more than `keep_recent_tokens` means
the history was replaced, and clears the backoff. A smaller drop does not: a
size recorded from the byte estimate (no provider count yet, after a resume or
a model switch) can read about 3k lower once the provider counts it. An applied
compaction and a session load, branch switch or rewind (`restore_branch_state`)
clear it too, and so does a change of the window or of any compaction setting
(`keep_recent_tokens`, `reserve_tokens`, the threshold): the recorded size says
nothing about the new limits. The dry run still applies to an automatic attempt
below capacity that the backoff lets through.

The backoff only ever defers an automatic attempt below the capacity limit. From
the capacity limit up an attempt is a recovery, like manual `/compact` and the
provider's context-overflow retry: it is never deferred, never records a
backoff, skips the dry run, budgets as described above, and is retried every
turn, exactly as before the backoff existed. A request above `window - reserve`
is the one that is about to stop fitting, so a recovery must not wait there. A
backoff that held through the reserve band would, with the default settings
(`keep_recent_tokens` 20,000, `reserve_tokens` 16,384), keep sending requests
above the capacity limit for many turns after a fold that could not apply (a
first prompt of 42k tokens in a 60k window: compaction at turn 1 as before, not
at turn 11).

Active mode is not changed by any of this. It folds under its own policy and
rewrites the compiled image when it does, which breaks the cached prefix.

Without a session, where events are numbered by position, the VM is rebased
onto the compacted transcript before that transcript replaces the live one;
a rebase that fails leaves the transcript as it was. The rebase keeps the
exact text of every `transient:` source the ledger or its episodes still cite,
so `retrieve_context` with those refs keeps working after any number of
compactions. With a session, cited `session:` sources are read from the JSONL,
which compaction does not shorten.

The compaction request is not a prefix-sharing request: the host summarizer
is a separate completion, and the one recorded ChatGPT-route probe of a
prefix-sharing compaction shape reported no cache reads. It sends the
excerpted events since the last fold instead of the whole transcript.

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

## Broker context authority

Broker items (`ContextItem`) carry a typed `authority`. Only host code that
builds the item sets `ContextAuthority::Mandatory`; `is_mandatory()` ignores
the source name and every field of the free-form `provenance` JSON, so a
source named `x-mandatory-notes` or one that writes `"mandatory": true` is
still optional and is dropped under budget like any other. A
`provenance_kind` of `mandatory_policy` in that JSON is likewise shown as
`repository_fact` unless the item is mandatory. Items deserialized without
the field are optional. No production source assigns mandatory authority
today.

`wrap_untrusted_data` writes `<context_data source="...">`. The source is
attribute-escaped (`&`, `"`, `<`, `>`) and any `<context_data` or
`</context_data` in the content, in any case, has its `<` written as `&lt;`,
so user text, tool output and broker content cannot close or open a wrapper.
Other content is unchanged.

`ContextOverlay` is applied by `build_context_with_overlay`: tombstoned and
excluded items are dropped, pinned items gain priority, and an item whose id
has a memory correction shows the corrected text as
`[user correction of <source>]` plus the text, with tokens re-estimated and
the ledger reason `user_corrected`. Ids are `provenance.id` or the item hash.
The agent holds the overlay (`set_context_overlay`, `context_overlay`); the
active image is compiled with it, and its revision and contents are part of
the prepared-image revision and the manifest's overlay revision. The coding
agent has no persisted overlay source yet, so nothing sets it in production.

In active mode an ephemeral message with non-text parts (images) is not
flattened into a text broker item. It is placed in the provider input before
the latest user message, as legacy does, its parts intact, charged at the
image ceiling and honouring `block_images`. Text-only ephemeral messages stay
broker items marked `tool_evidence`, the only kind that does not grant user
or policy authority to extension output.

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

Messages the harness writes in the user role (background-job notices,
capability and completion reminders, any message with a `customType`) are
`Custom` events, never `User`: job notices carry tool-evidence provenance and
reminders agent-inference provenance (a `HarnessReminder` event that still
compiles in the user turn, unwrapped, so turns keep alternating), so they cannot become goals, mandatory
image entries or user-budget excerpts. A message marked with
`davinciRealUserOrigin` stays a user message whatever else it carries. The
compaction and branch-summary serializers label the same messages
(`[Background job notice]`, `[Harness reminder]`,
`[Harness context (<customType>)]`) so only a person's text reads `[User]`.

Images a user attaches leave a durable reference where the bytes cannot stay:
`[image <mime> sha256:<16 hex>... - recover with retrieve_context <source_ref>]`.
The reference is in the fold prompt records and in the deterministic goal
(an image-only message is a goal made of its reference). `retrieve_context`
on that `sourceRef` returns the images as image content of the tool result,
on the first unfiltered page only, at most 4 images and 4 MiB of base64 per
call. With a bound session they are read from the JSONL and checked against
the event's content hash. Sessionless, the VM keeps up to 8 MiB of image data
in memory (carried across a rebase for cited sources); past that the result
says the images are not recoverable and returns the text only.

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

With a summarizer, a fold runs on the turn events leave the image without a
fold seeing them (`evicted_unfolded_tokens`): the model would otherwise keep
only their deterministic state, and the window advance that dropped them has
already changed the provider prefix. Automatic delta-depth and delta-token
folds wait for that and are skipped without a summarizer: before it the model
sees every event, and a deterministic fold only rotates the cache epoch.
Window pressure, the hard admission limit, phase boundaries and manual folds
are not held back.

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

The `--mode json` `harness_stats` record carries a top-level `contextVm` block
so a run shows whether the VM was configured and used. It is `{"mode": "off"}`
while the VM is off. Otherwise it holds `mode` (`shadow`, `active` or `hybrid`),
the counters above in camelCase plus `folds`, `tokensBeforeFold`,
`tokensAfterFold` and `pinnedPageLoads`, and `retrievalOffered`, which is true
once a fold made `retrieve_context` available. `folds: 0` in a long session
means the VM never needed to fold; compactions stay under `runtime.compactions`.

`tokensBeforeFold` and `tokensAfterFold` sum, over all folds, the estimated size
of the previous checkpoint ledger and of the new one. They measure the ledger,
not the prompt: the ledger grows as events are folded in, so `tokensAfterFold`
above `tokensBeforeFold` is normal and says nothing about whether a prompt
shrank. For the prompt, read `hybridCompaction`.

`contextVm.hybridCompaction` is present in `hybrid` mode only. In `shadow` and
`active` it is absent. It counts compactions:

| Field | Meaning |
| --- | --- |
| `attempts` | Every hybrid compaction entry: automatic, manual and overflow recovery |
| `applied` | Compactions that replaced the history |
| `skippedNoFit` | Dry run (automatic attempts below capacity only) said the replacement cannot fit or there was no earlier turn; no summarizer request |
| `skippedNoAssistantReply` | Automatic attempt below capacity where no cut had an assistant reply in the replaced prefix; no summarizer request |
| `skippedBackoff` | Automatic attempts deferred by the backoff; no summarizer request |
| `notAppliedAfterFold` | At least one summarizer request was sent and the history was left unchanged |
| `notAppliedOther` | Not applied without a summarizer request: no summarizer, no chunk fitting its window, a failed fold, a failed session write or rebase |
| `contextTokensBefore`, `contextTokensAfter` | Sums over applied compactions of the message estimate before and after |

`attempts` is the sum of the six outcome counters (`applied`, `skippedNoFit`,
`skippedNoAssistantReply`, `skippedBackoff`, `notAppliedAfterFold`, `notAppliedOther`). The token sums use
the message estimate only, so they leave out the system prompt and tool
schemas. A skipped or deferred result reports the same message estimate in
`tokens_before` and `tokens_after`.

The counters belong to the agent, not to the runtime: they survive a session
load, a branch switch and a rewind, and cover the whole agent lifetime. A run
that loads a session starts from zero only if the agent is new. Logs written
before these counters existed lack the object; read that as unknown, not zero.

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
