# TypeSafe / Jev decision intelligence

TypeSafe / Jev is an optional, provider-neutral decision layer for fast,
structured routing observations. It is not a coding completion model and it is
not an authority for permissions, security, transactions, required
verification, or workspace boundaries.

The v1 implementation is Phase 1 shadow mode: enabled requests are sent,
strictly validated, audited with metadata, and measured, but their answers do
not change the deterministic coding path. The feature is disabled by default.

## Settings and activation

The user-owned setting is shown under `AGENT BEHAVIOR` as:

`TypeSafe / Jev decision intelligence off`

Its values are `off` and `on`. A missing or malformed setting is treated as
disabled. The project settings merge may disable the feature, but it can never
enable it when the user setting is absent or off:

```text
effective_enabled = user_enabled && !project_disabled
```

Turning the setting off immediately stops new decision requests. It does not
delete the stored TypeSafe credential. The existing deterministic routing,
verification, and prompt behavior remain the baseline in either setting.

## Credential lifecycle

Credential resolution is deliberately separate from ordinary model selection:

1. `TYPESAFE_API_KEY` is preferred when present.
2. Otherwise the `typesafe` entry in `auth.json` is used.
3. An empty or invalid environment override is an explicit failure; it does
   not silently fall back to the stored key.

Login, model discovery, and TypeSafe use the same credential store. An explicit
`DAVINCI_CODING_AGENT_DIR` (or legacy `PI_CODING_AGENT_DIR`) selects its
`auth.json`. Otherwise, DaVinci uses `~/.davinci/agent/auth.json` when present,
retains an existing `~/.pi/agent/auth.json` when the new store is absent, and
creates the new store for fresh installations. Creating a settings or model
cache directory therefore does not hide existing logins.

The settings flow accepts paste, masks the candidate, rejects voice input while
the secret overlay has focus, and caps input at 8192 UTF-8 bytes. Surrounding
whitespace, an optional copied `Bearer` prefix, and a complete copied
`Authorization: Bearer ...` header are normalized before validation and storage.
Enter first performs a real credential validation request. The key is persisted
and the setting is enabled only after validation succeeds. If
settings persistence fails after a new key was stored, the previous credential
is restored (or the new entry is removed) and the feature remains off. Esc and
Ctrl+C cancel.

To replace an existing stored key from the DaVinci terminal, open `/settings`,
select **TypeSafe / Jev API key**, and press Enter. The replacement uses the
same masked, validate-before-save flow. Ctrl+V pastes text directly into the
masked field; typed uppercase letters and underscores are preserved.
A cancelled or rejected replacement
leaves the previous credential active. When `TYPESAFE_API_KEY` is present, the
environment controls the current credential; DaVinci explains that it must be
changed or unset outside the settings sheet and then restarted.

The validation request is a bounded probe to:

```text
POST https://api.typesafe.ai/v1/systemone
Authorization: Bearer <candidate>
Content-Type: application/json
```

It uses model `jev-latest`, asks one fixed Noul and one fixed Score about a
constant probe string, and requires the full response to pass the runtime
response validator described below. No task text is sent. No `/model`
discovery request is used.

## Request privacy contract

The request state is schema version 2 and contains only:

- a redacted, Unicode-capped current task;
- derived task signals;
- language and framework signals;
- recent file-kind categories;
- typed workspace state: clean, dirty, or unknown; and
- boolean availability flags for supported capabilities.

The serialized request is capped at 12 KiB and the task is capped at 4096
Unicode scalar values. DaVinci does not automatically read or send repository
source files, raw tool output, prior transcripts, system prompts, or memory and
skill bodies to Jev. The **current user task is sent** after secret/path
redaction. Fenced code blocks and recognized diffs/stack dumps are removed.
Unrecognized source pasted as ordinary prose can remain; redaction is not a
guarantee that arbitrary private text will be detected.

Workspace dirtiness remains unknown without an authoritative Git observation.
Edited UI paths do not imply a dirty worktree. Capability flags use exact native
tool registrations. When engineering tools have already computed a shared
snapshot, the shadow worker validates its root and file/directory stamps before
reusing language and dependency signals. Missing, invalidated, or busy snapshots
supply no additional facts. No workspace scan runs on user submission.

Provider responses are capped at 32 KiB and accepted only when their answer
set exactly matches the requested questions.

## Provider and failure behavior

Normal decision requests have an 800 ms first-attempt target and a 1500 ms
absolute caller deadline across attempts. The first attempt receives at most
800 ms; one retry is permitted only for a retryable error and within the
remaining hard budget. Shadow requests use one attempt. Credential validation
uses a separate 5-second probe.

Shadow preparation and inference run on a background worker concurrently with
the normal coding turn. There is one shadow slot and no queue; a busy slot drops
the sample. TypeSafe owns a persistent HTTP agent and connection pool.

The deadline covers the caller even if synchronous provider code or OS DNS
resolution does not return. Such a call can retain one worker until it exits;
the busy guard then makes later samples fall back instead of accumulating
threads. It does not delay the main coding provider or apply a late answer.

The provider maps failures to these health states:

- `Disabled`
- `Ready`
- `CredentialInvalid`
- `RateLimited`
- `Overloaded`
- `Unavailable`
- `SchemaMismatch`

Failure never fails the coding turn; the deterministic path continues. A 401
marks the current runtime generation credential-invalid and prevents repeated
calls until an explicit enable or credential replacement resets that
generation. 429, 529, and network failures use bounded cooldowns. User-facing
notices are emitted only on health transitions and never include credentials,
provider payloads, or raw errors containing secret material.

The runtime uses request generations. Disabling, enabling, or replacing a
provider increments the generation, and a response from an older generation is
discarded as stale.

## Response and routing policy

The request and response shapes follow the published System One API
(`https://docs.typesafe.ai/api.md`). Question ids are never sent to the model,
so every question states its full judgment and names the state fields it reads.
Noul questions describe their `true` and `false` outcomes in `criteria`, Choice
questions map 2 to 255 option ids to descriptions, and Score questions carry an
ordered array of 2 to 10 level descriptions. `DecisionRequest::validate_size`
rejects any other shape before a network call.

A response has exactly the documented top-level fields `model` (the concrete
model, such as `jev-1.13.0`), `answers`, and `usage` (`input_tokens`,
`output_tokens`). The validator accepts only the requested answer types:

- Noul: a finite probability value;
- Choice: a declared option, a finite probability distribution, and confidence;
- Score: a level position in `0..=levels - 1` (not a probability), an optional
  `legend` object, a distribution keyed by exactly the requested level
  numbers, and confidence. Telemetry records the position divided by
  `levels - 1`.

Undocumented fields, missing answers, unknown choices, non-finite values,
out-of-range probabilities or scores, malformed distributions, and oversized
responses are rejected. Credential validation asks one Noul and one Score and
runs the reply through the same validator, so a response shape the runtime
would reject fails at key entry rather than in every later request.

Deterministic requirements remain authoritative:

```text
FinalRequirements = DeterministicMinimum UNION AcceptedJevExtras
```

Jev answers can only add an approved optional capability when the provider is
available, the action is authorized, no deterministic veto applies, and the
configured confidence threshold is met. Jev cannot remove or weaken a
deterministic requirement. Verification, test, change, security, transaction,
permission, and workspace-boundary decisions remain deterministic. Choice and
score answers are telemetry-only in Phase 1.

The requested questions are bounded to the supported routing set:

`browser_relevant`, `git_history_relevant`,
`package_intelligence_relevant`, `test_impact_relevant`,
`change_impact_relevant`, `verification_planner_relevant`,
`verification_scope`, and `regression_risk`.

Confidence bands are high at `>= .85`, medium at `>= .60`, and abstain below
`.60`. Noul answers use the separate bands: strong yes `>= .90`, actionable
yes `>= .85`, uncertain `>= .70`, abstain `>= .30`, possible no `>= .10`, and
strong no below `.10`. Negative answers never remove deterministic work.

## Audit and telemetry

The bounded audit record contains only request ID, provider, model, decision
class, question IDs, a hash of the redacted state, state byte size, latency,
outcome, applied actions, and disagreement metadata. It never stores the task,
full request, full response, credential, authorization material, or raw tool
output.

Telemetry records request/success/failure counts, soft-deadline misses, timeouts, explicit HTTP
401/422/429/529 counts, schema failures, fallbacks, additions, disagreements,
reported provider token usage, and latency. It also keeps a small bounded
rolling set of per-answer metadata:
answer type, value or choice, confidence, probability margin, and whether the
answer was shadow-only or behavior-affecting. It contains no task text or
credential material.

There is no durable decision cache. Only bounded in-process state is retained
for health, audit, and telemetry.

## Rollout gate

Phase 1 is shadow-only and records no behavior changes. A future guarded
additive phase requires the offline decision-intelligence evaluation and a
review showing:

- no deterministic regressions;
- zero removal of deterministic requirements;
- complete fallback coverage for provider failure fixtures;
- no credential or private-input leakage;
- measured latency and usage within budget; and
- measured useful-addition, false-addition, missed-addition, and disagreement
  rates.

Until that gate is explicitly met, the runtime must not silently switch to
guarded behavior.

The repository evaluation fixture covers the ten required scenario classes,
including off/default behavior, privacy, deterministic fallback, provider
failures, stale results, shadow invariance, mandatory verification, and the
measurement fields needed for a guarded decision.

Live TypeSafe API validation is intentionally not part of the offline test
pass; it requires a user-provided credential and external service access.
The ignored live acceptance test uses the stored credential (never printed)
and the production provider, probe, request builder, and parser:

```sh
cargo test -p davinci-coding-agent --test typesafe_live -- --ignored --nocapture --test-threads=1
```

First run, 2026-09-24, `jev-latest` resolving to `jev-1.13.0`: the probe and
all five routing requests parsed. Latency was 132 to 305 ms, under the
800 ms soft budget. Each request was about 5.1 KB and about 1,500 input and
185 output tokens; the question text, not the task, dominates the size.
