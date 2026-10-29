# 00 — Research import and evidence classification

Status: audit input. No DaVinci code changed by this document.
Audit date: 2026-10-27.

## 1. Verified identity of the research corpus

Read directly from `C:\Users\sergi\Desktop\lean-claude\re-claude-code\` (not from
chat summaries), and recounted with the repository's own CSV.

| Property | Verified value |
|---|---|
| Target | Claude Code CLI 2.1.310, Windows x64 |
| Executable SHA-256 | `F9A3920634AE3B961EE92C2EB58D0051F5F8852B120983D0A4F57B6A5EFEBA61` |
| Evidence rows | 201 (`Import-Csv evidence-register.csv` → 201 objects) |
| CSV physical lines | 202 (header + 201) |
| CSV columns | 10 |
| Markdown reports | 46 files, `01`–`44` plus diagrams and fixtures |
| Static offsets | Refer to `%TEMP%\opencode\re\cc\textblob.js`, UTF-8 view of the embedded bundle |
| Executable size | 296,791,712 bytes (PE32+), ~53 MB embedded JavaScript |

Evidence classes in the register are the research's own:
`PROVEN_DYNAMIC`, `PROVEN_STATIC`, `PROVEN_STATIC + PROVEN_DYNAMIC`,
`DOCUMENTED`, `INFERRED`, `NOT ESTABLISHED`, `UNKNOWN`.

This audit does not upgrade any of them. A `PROVEN_STATIC` finding describes code
that exists; it does not prove the code runs on every path. A `DOCUMENTED`
finding is a vendor statement. `INFERRED`, `NOT ESTABLISHED` and `UNKNOWN` are
never used here as implementation justification.

## 2. Classification used below

| Class | Meaning in this audit |
|---|---|
| **A** | Enough evidence to state a DaVinci requirement or write a regression test now |
| **B** | Mechanism understood; applicability to DaVinci must be measured or code-audited first |
| **C** | Important unknown blocks a sound decision; research or DaVinci evidence is missing |
| **D** | Provider-, platform-, or product-specific; no engineering justification to adopt |
| **E** | Verified failure or dangerous design; DaVinci must actively prevent it (negative test) |

Class **E** is not "do the opposite of a feature". It is a specific defect with a
fixture, and the requirement is the invariant it violates.

## 3. Findings that drive requirements (class A)

All rows are version 2.1.310 / `F9A39206…EBA61`. "Fixture" names the research
artefact under `re-claude-code/_artifacts/`; "D" is the DaVinci implication.

### 3.1 MCP request identity, replay and reporting

| ID | Claim | Class | Failure mode / fixture | DaVinci implication |
|---|---|---|---|---|
| F-20-01 | Automatic replay is capped at one retry (two attempts), no backoff | A | `e2s4/s4k`, `s4o` | A transport retry budget must be explicit and per-call, not per-attempt-loop |
| F-20-02 | A replay re-sends the same `toolUseId` with a **new** JSON-RPC id after re-dialling | A | `e2s4/s4o` frames | Invocation identity and transport request id are separate; DaVinci must keep both and log both |
| F-20-04 | Raw HTTP faults (ECONNRESET, 500, truncated body, timeout) are never retried | A | `e2s4/s4i`, `s4j`, `s4l`, `s4m` | A raw transport fault is an *unknown* outcome, not a safe retry |
| F-20-05 | HTTP 404 is classified `session_expired_404`, `mayHaveRun=false`, replayed once | A | `e2s4/s4k`, `s4o` | Never derive "did not run" from a status code; require evidence |
| F-20-07 | JSON-RPC `-32000 Connection closed` on HTTP returns an explicit may-have-run warning for a plain tool | A | `e2s4/s4q` | The honest unknown-outcome message is the reference behaviour |
| F-20-10 | A model-initiated second call after a crash differs by new `toolUseId`, new args, fresh handshake | A | `e2s4/s4g` | Recovery must not manufacture a new invocation id, or the two cases become indistinguishable |
| F-31-01 | A session-recovery failure closes the client shared by every pending call | A | `31` §3 | Shared-connection teardown is the collateral-replay root cause |
| F-31-03 | A peer call is re-executed although it received no retryable error of its own | A | `e2s5/a1,a2,a3,a5,b3,b4,c1,c2,g1,g2,o2` | DaVinci must not translate "peer failed" into "this call failed" |
| F-31-05 | A peer response already received is not replayed; a pending one is discarded and re-executed | A | `e2s5/o1,o2,o3` | Teardown must not discard a completed result |
| F-31-06 | Raw ECONNRESET does not tear down the shared client; the peer is unaffected | A | `e2s5/a4` | Teardown scope must be narrower than the connection object |
| F-31-07 | An HTTP 404 on a session-bearing transport replays even a plain non-idempotent tool | A | `e2s5/f2`, `b1` | The 404 path is the highest-risk direct replay |
| F-31-10 | One 404 recovery creates two client generations, one orphaned | A | `e2s5/a2,b1,f2` | Reconnect must not leak a live connection generation |
| F-42-04 | Duplicate client generations occur per recovery, keyed by `(name, config)` | A | `_artifacts/e2s6` | Connection registry must have one live owner per server |
| F-42-06 | The transport-level bound is at-most-twice per call, not at-most-once | A | `31`, `42` | Any DaVinci doc must state the actual bound |
| F-06-06 | Every `tools/call` carries `_meta` `claudecode/toolUseId` and a progress token | A | `06` | A stable per-invocation id is sent to the server; useful, not a guarantee |
| F-06-10 | An MCP crash after init is not retried within the crashing call | A | `06` | Matches the at-most-once stdio result; DaVinci must report unknown, not failure |
| F-06-08 | MCP has 5 background reconnect attempts and 3 initial-transient attempts | A | `06` | Connection attempts must be bounded and classified separately from call retries |

### 3.2 Tool execution and scheduling

| ID | Claim | Class | DaVinci implication |
|---|---|---|---|
| F-10-01 | Tools default to `isConcurrencySafe=false`, `isReadOnly=false` | A | The safe default is "serialize"; DaVinci's classifier must also default closed |
| F-10-02 | Calls are partitioned into consecutive safe runs with unsafe tools as barriers | A | Ordering barriers must be preserved across a batch |
| F-10-03 | At most 10 calls run concurrently per safe run | B | The *mechanism* (per-run cap) is A; the *number* 10 is not transferable |
| F-10-04 | Parallel results are returned in one user message | A | Result publication must keep one result per `tool_use` id, in order |
| F-17-01 | The concurrency limit is per safe run, not global | A | A global semaphore changes barrier semantics |
| F-17-05 | A 403 is not retried and surfaces as authentication failure | A | Auth failure must not be retried into a lockout |
| F-17-06 | A truncated stream triggers a non-streaming retry with halved `max_tokens` | B | Applicability depends on DaVinci's provider decoding; must not silently change the user's budget |

### 3.3 Permissions

| ID | Claim | Class | DaVinci implication |
|---|---|---|---|
| F-37-01 | A deny rule removes the tool from the request schema and the call is rejected as unavailable | A | Denial must be enforced at execution, not only by hiding the schema |
| F-37-02 | Deny beats allow, even a command-line `--allowedTools` | A | Precedence must be deterministic and tested |
| F-37-03 | A `PreToolUse` hook exit 2 blocks the call | A | Hooks are an enforcement point; a hook error must fail closed |
| F-11-02 | `dangerousRemoval` is `bypassImmune` | A | Some operations cannot be bypassed by any mode |
| F-11-03 | Headless `-p` executes a benign Bash call with no approval under default/plan | E | Approval-free headless execution is a posture, not a bug; DaVinci must document its own posture rather than copy this |

### 3.4 Context, cache and persistence

| ID | Claim | Class | DaVinci implication |
|---|---|---|---|
| F-07-01 | Context is assembled in tiers with volatile content outside system | A | Volatile turn state must not rewrite the cached prefix |
| F-08-03 | `DeferredToolPlaceholder` keeps the tool array shape stable | A | Removing a tool mid-session must not renumber the prefix |
| F-08-04 | Tool order is deterministic across sessions | A | Registry order must be sorted/stable, not hash-map order |
| F-09-04 | Journal append is serialised and guarded by a generation counter | A | Concurrent appends must be ordered and detectable |
| F-41-01 | The 5,836 B system prompt is byte-identical across turns | A | Prefix identity is measurable offline |
| F-41-03 | Session JSONL persists per-request cache boundaries, digests and content-addressed blobs | A | Request-shape evidence is durable and auditable |
| F-35-03 | Compaction decisions use `POST /count_tokens` on the incremental payload | B | DaVinci already uses calibrated provider counts; do not add a second authority |

### 3.5 Orchestration

| ID | Claim | Class | DaVinci implication |
|---|---|---|---|
| F-03-01 | The agent tool launches asynchronously by default and returns a handle | B | DaVinci already has background workers; the handle contract is worth comparing |
| F-03-07 | Spawn depth default is 3 and the cap was reached by an executed ladder | A | A depth cap must be enforced and observable |
| F-32-03 | At the depth cap the Agent tool is removed from the request surface | A | Removing the tool is stronger than rejecting the call; test the surface |
| F-32-04 | Concurrent subagent default is 20 | B | The number is not transferable; the *presence* of an admission gate is A |
| F-32-06 | Completion arrives as a user-role `[Subagent hand-back]` message | B | Delivery must be a durable message, not an in-memory callback |
| F-33-01 | The team roster is flat: teammates cannot spawn teammates | B | Compare with DaVinci's nesting policy |
| F-34-05 | Workflow journal keys are `v2:<sha256>` with launched/started/result lines | A | Resume must key on a content digest, not a mutable index |
| F-34-08 | Three identifier namespaces exist; only `wf_…` is accepted by `resumeFromRunId` | A | Resume ids must be unambiguous and validated |

### 3.6 Verified failures that must be prevented (class E)

| ID | Verified behaviour | Why it must not be copied |
|---|---|---|
| F-20-03 | stdio crash after commit and stdio crash before commit produce the *same* `Connection closed` error | Unknown outcome mislabeled as definite failure |
| F-20-06 | A successful 404 replay is reported to the model as success | A duplicate mutation is invisible |
| F-20-08 | `idempotentHint`/`readOnlyHint` cause replay even when `mayHaveRun=true` | A server annotation converts at-most-once into at-least-once |
| F-20-09 | Annotations do not help on stdio because stdio never classifies the error | Annotation-based safety is transport-dependent |
| F-31-04 | The collateral error is admitted only by `isConcurrencySafe()` — the same predicate that permitted overlap | Concurrency safety is not retry safety |
| F-31-11 | Retry exhaustion and failed replay are reported as failures although two mutations committed | False negative after a real side effect |
| F-32-08 | `tasks/<agentId>.output` is advertised as the full transcript but was 0 bytes in 6/6 agents | Never publish a result path that is not written |
| F-40-05…07 | Coding-task success, pass@k, verification rate, false-claim rate and token cost per task are **not established** | No parity or efficiency claim may cite them |

## 4. MCP "at-most-once": required reconciliation

The report-20 table is a per-call statement. Report 31 shows the client-level
behaviour. Six different events are routinely conflated; DaVinci must model them
separately.

| Event | Definition | Claude Code 2.1.310 evidence | Safety question DaVinci must answer |
|---|---|---|---|
| **Direct replay** | The failing call is sent again by the client | HTTP 404 → `mayHaveRun=false` → replayed once (F-20-05, F-31-07). Raw faults → not replayed (F-20-04). stdio → no retry site (F-20-03) | Did *this* invocation already run? |
| **Collateral replay** | A call that had no error of its own is re-sent because a peer tore down the shared client | Proven in 11 cases (F-31-03) | Is teardown scoped to the failing request, or to the connection? |
| **Transport reconnection** | A new connection/session is established; no call is resent by itself | 404 recovery creates two generations (F-31-10, F-42-04) | Does reconnect retain or lose pending results? |
| **Model-initiated second call** | The model asks again in a later turn | New `toolUseId`, new args, fresh handshake (F-20-10) | Is it distinguishable from a replay in logs? |
| **Idempotent execution** | The server executes twice but the effect is the same | Not tested server-side; `idempotentHint` is only a hint (F-20-08) | Who is allowed to assert idempotency — the server, the tool author, or the client? |
| **Unknown outcome** | The client cannot tell whether the effect committed | stdio crash/timeout/truncation all report failure (F-20-03); only the `oM` HTTP path says "may have run" (F-20-07) | Is unknown a first-class state, or an error string? |

**Conclusion.** "At-most-once" is false as a client-wide property. The only
defensible statements are:
1. transport-level attempts are bounded at two per call (F-42-06);
2. a call whose response was received is not replayed (F-31-05);
3. everything else is conditional on the client's own classification of
   "did it run", which is derived from the error shape, not from a server
   acknowledgement.

DaVinci must therefore not adopt a universal at-most-once guarantee. It must
adopt: a stable invocation id, an explicit unknown state, no automatic replay of
a mutation, and a teardown scope that cannot invalidate a peer's result.

## 5. Class B/C/D register (grouped)

Class B and C findings are not requirements yet. They are listed so nothing is
silently dropped; each is resolved in `01-CLAUDE-DAVINCI-CAPABILITY-MATRIX.md`
against DaVinci code.

| Group | Finding IDs | Class | Why not A |
|---|---|---|---|
| Executable layout, version, signing, Bun chunks | F-01-01…06 | D | Properties of that binary, not behaviours |
| Agent-loop prompt geometry and billing headers | F-02-01…09 | B/D | Provider-specific prompt assembly; DaVinci has its own |
| Subagent prompts, tool counts, billing | F-03-03, F-03-05, F-32-10 | D | Claude-specific prompt documents |
| Subagent async envelope and metrics | F-03-01, F-03-02, F-32-05, F-32-09 | B | Compare with DaVinci worker protocol |
| Teams | F-04-01…03, F-33-01…04 | B | Flat roster is a product choice |
| Team IPC | F-33-05 | C | Auth/delivery/claim atomicity unknown |
| Workflow constants and scheduling | F-05-01…05, F-34-01…09 | B | DaVinci has its own scheduler; compare semantics |
| Workflow semaphore internals | F-34-03 | C | Code lives in absent Bun chunks |
| MCP handshake/ids/idle/reconnect | F-06-01…05, F-06-07, F-06-09, F-06-11 | B | DaVinci MCP client is independently implemented |
| MCP cancellation | F-42-01…03, F-42-05, F-42-07 | B/C | Partly UNKNOWN; see 02 |
| Context tiers and auto-compact settings | F-07-02…05, F-35-01…05 | B | DaVinci has a different, measured compaction design |
| Compaction summary request | F-35-05 | C | Research did not establish the request construction |
| Prompt-cache breakpoints and TTL | F-08-01, F-08-02, F-08-05, F-17-09 | B/D | Anthropic-specific wire fields |
| Session JSONL format and supervisor | F-09-01…03, F-12-01…03, F-36-01…08 | B/D | DaVinci uses JSONL + SQLite; compare guarantees, not format |
| Session recovery | F-36-08 | C | Torn-tail/corrupt/orphan paths not established in Claude Code (DaVinci has its own tests) |
| Tool catalogue size / bare mode | F-10-05, F-10-06 | D | Catalogue policy |
| Permission modes | F-11-01, F-11-04, F-37-04…06 | B/C | Mode names differ; interactive approvals unverified |
| Extensions/hooks/telemetry | F-13-01…03 | D | Product surface |
| IDE integration | F-14-01…04, F-38-01…12 | D/C | Different product; some security facts still informative |
| Observability, absolute performance | F-15-01, F-15-02, F-16-01…04, F-39-01…05 | B/D | Different binary; only method transfers |
| Provider retry algorithms | F-17-02…04, F-17-07, F-17-08, F-17-10, F-17-11 | B/C | Applies to the model request path, not tools |
| Effectiveness and quota | F-40-01…07, F-41-02, F-41-04…07 | B/C | **No outcome or savings claim exists** |
| Surface subtraction | F-43-01…03 | B | A reusable idea, not yet a requirement |

## 6. What the corpus explicitly does not establish

These are load-bearing gaps. They are copied into `08-CODING-SUCCESS-AND-SUBSCRIPTION-METRICS.md`
and must not be quietly resolved by assumption.

- Coding-task success, pass@k, verification rate, false-claim rate: **not established** (F-40-05).
- Any client-vs-client or arm-vs-arm effectiveness comparison: **not established** (F-40-06).
- Token/quota cost of a completed coding task: **not established** (F-40-07).
- Provider cache hits and billed-token savings: **not established** (F-41-06).
- Subscription quota savings: **not established** (F-41-07).
- Interactive approval, approval caching/expiry, sandbox enforcement: **unknown** (F-37-06).
- stdio concurrency, timeout during an in-flight POST, session-key collision precedence: **unknown** (F-42-07).
- Cancellation racing an MCP teardown: **not run** (`31` §7 group D).

## 7. Reproduction pointers

Raw artefacts remain in place; this audit did not modify them.

| Need | Path |
|---|---|
| Replay matrix, 21 cases | `re-claude-code/_artifacts/e2s4/` (`summary.csv`, `ledger.jsonl`, `frames.jsonl`) |
| Concurrent recovery, 22 cases | `re-claude-code/_artifacts/e2s5/` |
| Cancellation / stream-json | `re-claude-code/_artifacts/e2s6/` |
| Fixture inventory and safety envelope | `re-claude-code/experimental-fixtures.md` |
| Reconstructed algorithms A1–A… | `re-claude-code/17-reconstructed-algorithms.md`, `43-additional-reconstructed-algorithms.md` |
| Per-subsystem classifications and corrections C-1…C-11 | `re-claude-code/44-COMPREHENSIVE-FINAL-REPORT.md` |

## 8. Limitations of this document

- The classification is an engineering judgement over the register, not a new
  experiment. No Claude Code process was executed for this audit.
- Class A means "enough evidence to write a requirement or a test". It does not
  mean the mechanism is correct or desirable; several A findings are class E
  negatives.
- Where DaVinci already has a different mechanism, the finding is classified B
  even when the research evidence is strong, because the comparison has not
  been made yet. `01` records that comparison.
