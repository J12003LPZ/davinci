# Codex-Efficient Pi Rust Harness Design

Date: 2026-09-03  
Status: Approved design; implementation planning pending written-spec review  
Repository baseline: J12003LPZ/pi-rust at e25bb7c6677de36156b921e61670616efe20ecff  
Reference implementation reviewed: openai/codex main branch  
Primary deployment path: ChatGPT/Codex OAuth subscription backend  
Secondary deployment path: public OpenAI Responses API

## 1. Executive summary

Pi Rust already has a capable provider-neutral agent loop, live streaming, Codex WebSocket continuation, parallel tool scheduling, context pruning, MCP support, subagents, permissions, hooks, and telemetry. The next improvement should not be a rewrite or a fork of Codex. It should be a capability-gated Codex compatibility profile layered over Pi's existing abstractions.

The profile will make Pi speak the Responses protocol losslessly, reuse a persistent WebSocket transport, expose the tool interfaces Codex models are trained to use, preserve prompt-cache stability, and measure every optimization against verified repository tasks.

The implementation has five major systems:

1. CodexCapabilities selects behavior from authenticated backend and model capabilities.
2. ResponsesLedger preserves native Responses items without weakening Pi's generic transcript.
3. CodexTransport owns persistent WebSocket connections, lanes, continuation, recovery, and cancellation.
4. CodexToolSurface provides a small hot tool set plus deferred namespaces.
5. Codex evaluation and telemetry prove task success, latency, turn, token, cache, and reliability improvements.

Correctness is the release gate. Efficiency is measured only among runs that satisfy machine-verifiable task criteria.

## 2. Goals

The design must:

- Optimize ChatGPT/Codex OAuth use first while retaining public API support.
- Preserve Pi's provider-neutral agent loop, scheduler, permissions, extensions, hooks, and TUI.
- Reduce end-to-end task time, model responses, redundant tool calls, and uncached input tokens.
- Preserve all response items required for correct continuation and reasoning.
- Prevent duplicated side effects during retries and transport recovery.
- Make optional tool schemas discoverable instead of permanently consuming context.
- Use prompt caching and context compaction without corrupting session history.
- Produce repeatable Pi-versus-Pi and Pi-versus-Codex CLI benchmarks.
- Allow every major optimization to be independently disabled.

The balanced success scorecard is ordered as follows:

1. Verified task success.
2. End-to-end wall time.
3. Number of model responses.
4. Context and uncached input tokens.
5. Cache reuse.
6. Tool-call count and harness resource use.

## 3. Non-goals

The first implementation will not:

- Replace Pi's agent core with Codex crates.
- Fork the openai/codex repository.
- Make Codex-specific response objects the universal provider abstraction.
- Automatically change the root model or reasoning effort on every turn.
- Adopt server-managed multi-agent execution.
- Require programmatic tool calling.
- Send individual tool results into an actively generating response.
- Remove existing Pi tools needed by non-Codex providers.
- Treat model self-assessment as benchmark success.

Programmatic tool calling, server-managed multi-agent operation, and per-turn model routing may be reconsidered only after measurement identifies them as remaining bottlenecks.

## 4. Current-state assessment

At the reviewed baseline:

- crates/pi-ai/src/stream.rs already builds Responses requests with parallel tool calls, a prompt cache key, low text verbosity for Codex, automatic tool choice, and encrypted reasoning content in include.
- crates/pi-ai/src/codex.rs already maintains per-session and per-account WebSocket state and can continue with previous_response_id and delta input.
- crates/pi-ai/src/stream_decoder.rs decodes function and custom tool calls and reasoning summaries.
- crates/pi-ai/src/lib.rs models tools primarily as function-style name, description, and JSON parameters.
- crates/pi-agent has a parallel scheduler, batch support, context pruning, token governance, permissions, hooks, MCP, plans, and subagents.
- crates/pi-evals contains parity-oriented fixtures but not a sufficiently automated live A/B performance harness.

The remaining high-value gaps are lossless native response preservation, full transport lifecycle management, Codex-shaped tools, deferred schema loading, cache-aware prompt construction, adaptive token accounting, and attributable live measurement.

## 5. Chosen architecture

Three approaches were considered:

1. A provider-specific Codex compatibility profile.
2. A Responses-native rewrite of the entire core.
3. Reuse or fork of Codex crates.

The selected approach is the compatibility profile. It offers most of the model-alignment benefit while keeping Pi differentiated and provider-neutral. A core rewrite has a larger regression surface. Direct Codex crate reuse creates tighter coupling to an internal architecture that Pi does not control.

The dependency direction is:

- The generic Pi agent loop asks the provider for capabilities.
- Codex capabilities activate a native ledger, transport, request builder, and tool adapters.
- Provider-specific components implement interfaces owned by Pi.
- Permissions, scheduling, hooks, session storage, extensions, and UI consume stable Pi events rather than Codex wire objects.

No generic component may branch on a model name when a declared capability can answer the same question.

## 6. Core components

### 6.1 CodexCapabilities

CodexCapabilities is an immutable snapshot for one request lineage. It is derived from the authenticated backend, advertised model metadata, configuration, and conservative probes.

It describes support for:

- Responses API item types.
- WebSocket transport.
- Incremental continuation.
- generate:false prewarming.
- Stream multiplexing.
- Turn-state headers.
- Encrypted reasoning content.
- Assistant phases.
- Custom grammar tools.
- Tool namespaces.
- Native or emulated tool search.
- Explicit cache breakpoints.
- Server-side compaction.
- Service tier selection.
- Zero-data-retention-compatible behavior where applicable.

Unknown capabilities default to disabled. A probe failure must not silently enable an experimental behavior. Capability discovery first uses a versioned backend/model table. An optional probe must be non-generating, side-effect free, cached for the authenticated account, and incapable of delaying the first user turn by more than the configured transport timeout.

The capability snapshot is persisted with the session. A change that affects request shape starts a new Responses lineage.

### 6.2 ResponsesLedger

ResponsesLedger stores the exact ordered sequence of native Responses input and output items required to continue a session. It exists beside Pi's generic ChatMessage history.

The ledger must preserve:

- User and assistant messages.
- Assistant phase.
- Function and custom tool calls.
- Tool results and call identifiers.
- Reasoning summaries.
- Opaque or encrypted reasoning items.
- Unknown future item types as raw typed payloads.
- Response IDs and lineage boundaries.
- Compaction checkpoints.
- Partial output and cancellation state.

The generic transcript remains optimized for UI, provider switching, search, export, and compatibility. The native ledger is authoritative for Codex continuation.

Legacy sessions without a complete ledger can be loaded, but they must start a new lineage with a full normalized replay. Pi must not manufacture continuation from incomplete history.

### 6.3 CodexTransport

CodexTransport owns an account-level connection pool rather than placing socket lifecycle logic inside the agent loop.

It provides:

- Persistent WebSocket connections.
- Multiple stream lanes identified by stream ID.
- Prewarmed requests using generate:false when supported.
- Continuation using previous_response_id plus delta input.
- Coordinated OAuth refresh.
- Backpressure and bounded queues.
- Explicit cancellation.
- Idle and maximum-age rotation.
- Connection health and latency metrics.
- SSE fallback only before response output has begun.

The first version waits for all parallel tool calls from one response, executes them concurrently, and sends their results in a single continuation. Injecting results individually into a still-active response is deferred until a capability probe and benchmark demonstrate safety and benefit.

### 6.4 CodexToolSurface

CodexToolSurface translates stable Pi tools into provider-native definitions. It does not own the tool implementations or bypass permissions.

The immediate hot set is:

| Tool | Purpose |
|---|---|
| exec_command | Start a bounded command using the platform-appropriate shell |
| write_stdin | Continue an interactive command session |
| apply_patch | Apply a grammar-constrained workspace patch |
| read | Read bounded file ranges |
| grep | Search contents, preferring ripgrep |
| find | Discover files, preferring rg --files |
| ls | Inspect a directory |
| update_plan | Adapt Pi plan state to the Codex-recognized interface |
| agent | Run a bounded Pi subagent task |
| tool_search | Discover deferred namespaces and definitions |

For the Codex profile, bash, powershell, write, edit, and batch are not advertised by default. Their implementations remain available as compatibility aliases or deferred tools. All aliases map to the same ToolClass, permission policy, hooks, timeout, and audit behavior.

## 7. Normal request lifecycle

A normal turn follows this order:

1. Resolve model, authentication, configuration, and CodexCapabilities.
2. Compute the stable-prefix and request-shape hashes.
3. Reuse or create the appropriate transport connection and stream lane.
4. For a new or invalidated request shape, send at most one generate:false prewarm only while other user-visible work is already in progress, such as permission resolution or tool execution. Never delay a ready user delta to prewarm, and never repeat an identical prewarm.
5. Send the user delta using the current previous_response_id.
6. Stream native response items into ResponsesLedger and generic UI events.
7. Accumulate tool calls and validate call identifiers.
8. Execute independent tools concurrently through Pi's scheduler and permissions.
9. Record each tool result in the exactly-once call ledger.
10. Send all completed results in one continuation.
11. Repeat until the model emits a final-answer phase or a terminal error.
12. Persist usage, trace data, response ID, capability snapshot, and lineage state.

A turn is not complete merely because text was emitted. Completion requires a terminal provider event or an explicit cancellation/error state recorded by Pi.

## 8. Request-shape invalidation

The request-shape hash covers:

- Model.
- Reasoning effort.
- Stable instructions.
- Immediate tool definitions and schema versions.
- Permissions that change advertised capabilities.
- Cache mode.
- Provider feature flags.
- Authentication/backend family.
- Compaction lineage.

A change to any of these starts a new lineage or performs a full replay according to capability policy. Cosmetic UI changes do not invalidate provider state.

## 9. Recovery and exactly-once rules

Recovery behavior is explicit:

- previous_response_not_found: perform one full lossless replay and establish a new lineage.
- WebSocket failure before output: reconnect and retry once under the same call ledger.
- WebSocket failure after partial output: retain the partial response and report recovery state; do not blindly regenerate.
- SSE fallback: allowed only before output begins.
- OAuth expiration: pause new sends, perform one coordinated refresh, then resume queued work.
- 429 or retryable 5xx before accepted tool calls: honor Retry-After when present and add bounded jitter.
- Error after a side-effecting tool result: never re-execute solely because the provider response was lost.
- Unknown response item: persist it losslessly and surface a compatibility trace rather than dropping it.
- Cancellation: record a terminal cancellation item and prevent queued tool calls from starting.
- Transport rotation: drain active lanes before replacement.

The tool-call ledger is keyed by session, lineage, and provider call ID. It stores normalized arguments, permission decision, execution state, result digest, and side-effect classification.

## 10. Typed tool definitions

Pi's function-only tool specification becomes a provider-neutral tagged definition:

- Function tool.
- Custom/freeform tool.
- Namespace.
- Tool-search tool.

Every definition supports:

- Deterministic canonical serialization.
- Strict input validation.
- Deferred-loading metadata.
- Namespace membership.
- Required capabilities.
- Permission classification.
- Timeout and output limits.
- Versioned schemas.

Tools are emitted in stable canonical order. A schema or ordering change updates the request-shape hash.

### 10.1 apply_patch

apply_patch uses a Codex-recognized custom grammar and remains permission-gated.

The implementation must:

1. Parse the entire patch.
2. Resolve every path against the workspace root.
3. Reject traversal and unauthorized paths.
4. Validate all hunks against current contents.
5. Obtain required permissions.
6. Build a recovery journal and stage creates and updates in temporary files.
7. Commit the patch as one logical invocation.
8. If any mutation fails, restore every previously changed path from the journal before returning failure.
9. On startup, detect and resolve an incomplete journal before accepting another patch.
10. Record the normalized patch digest and affected paths.
11. Return a compact deterministic result.

Any adapted code or grammar from openai/codex must preserve applicable MIT attribution.

## 11. Deferred tools and namespaces

Optional tools are not placed in the stable hot set. Deferred areas include:

- Web access.
- Memory.
- Graph operations.
- Security analysis.
- Jobs.
- Extensions.
- Provider-specific optional tools.
- MCP servers.

Each MCP server receives a descriptive namespace. A namespace should normally expose fewer than ten closely related functions; larger servers are divided by domain.

When native tool search is unavailable, Pi advertises an emulated tool_search function. Its result causes selected definitions to be appended on the next request. Discovered tools are added after the stable cache boundary so the prefix remains reusable.

## 12. Prompt caching

The request layout is deterministic:

1. Base Codex behavior instructions.
2. Pi permissions and tool strategy.
3. Repository instructions such as AGENTS.md or CLAUDE.md.
4. Immediate tool definitions.
5. One explicit cache breakpoint when supported.
6. Conversation and steering items.
7. Tool results, retrieved memory, and newly discovered definitions.

Configuration:

| Mode | Behavior |
|---|---|
| auto | Use explicit caching only when advertised; otherwise use implicit caching |
| implicit | Send a stable conversation cache key without explicit breakpoints |
| explicit | Require backend support and fail configuration validation if unavailable |
| off | Disable Pi-controlled cache fields |

The prompt cache key is stable within the authenticated conversation. The request-shape hash is tracked separately. Pi records cache-read, cache-write, and uncached-input tokens and warns through telemetry when cache writes are repeatedly not reused.

The initial implementation uses one breakpoint. Additional breakpoints are added only when live measurements show net benefit.

## 13. Context and output economics

Provider-reported token usage is authoritative after each response. A model-aware tokenizer estimates the next request. Character-count estimation is only a fallback.

Preflight accounting reserves:

- Projected input.
- Maximum or configured response output.
- Reasoning allowance.
- Safety margin.

Pressure is applied in this order:

1. Bound output at the producing tool.
2. Store full evidence locally and return a compact model-facing view.
3. Replace old large outputs with stable retrieval references.
4. Prune completed or superseded tool chains from the provider view.
5. Compact when pruning cannot restore safe headroom.
6. Begin a new lineage after compaction.

Pi never prunes the active user request, unresolved tool calls, current constraints, active plan state, the most recent supporting evidence, or opaque items required by continuation.

Server-side compaction is preferred when advertised. The fallback local checkpoint records goals, constraints, decisions, relevant evidence references, modified files, verification status, and unresolved work.

Tool outputs use three layers:

1. Compact model-facing summary.
2. Full evidence stored in the session evidence store.
3. Stable reference for selective retrieval.

Output policies are semantic: searches limit matches per file and globally; reads limit ranges and bytes; commands preserve exit status and relevant leading/trailing diagnostics; tests preserve failed names and messages; agent calls return conclusions and evidence references; MCP payloads are validated and bounded.

## 14. Model and reasoning policy

The root model and reasoning effort remain stable within a lineage. Automatic per-turn switching would invalidate continuation and reduce cache reuse.

Pi may provide explicit quality/latency presets at session creation. Bounded subagents may use a faster model when the account and configuration permit it, but routing decisions are recorded and benchmarked. Raising root reasoning effort starts a new lineage.

## 15. Observability

Every execution emits sanitized JSONL events containing:

- Session, turn, response, stream, call, and agent identifiers.
- Stable-prefix and request-shape hashes.
- Request build and send times.
- First-byte and first-useful-tool times.
- Tool queue, start, and finish times.
- Continuation and response-complete times.
- Compaction boundaries.
- Token usage by category.
- Tool-definition count and serialized size.
- Continuation versus replay.
- Cache mode and observed reuse.
- Retry, fallback, and cancellation events.
- Provider latency versus Pi overhead.
- Harness CPU and peak memory where available.

Raw prompts, authentication data, environment variables, encrypted reasoning, and sensitive tool output are excluded by default. OpenTelemetry export is optional; local JSONL is always supported.

## 16. Benchmark design

Pi uses three layers:

### 16.1 Protocol conformance

Runs on every pull request. It verifies request serialization, event decoding, continuation, custom tools, unknown item preservation, cancellation, and recovery.

### 16.2 Recorded deterministic evaluation

Runs on every pull request with fixed streams and tool fixtures. It catches scheduler, parser, permission, pruning, and output-governor regressions without consuming provider usage.

### 16.3 Live Codex evaluation

Runs explicitly or before release. It compares:

- Pi's generic OpenAI profile.
- Pi's optimized Codex profile.
- Official Codex CLI as an external reference.

Paired runs use the same repository revision, isolated workspace, prompt, model, effort, permissions, network policy, and starting Git state. Cold-cache and warm-cache suites are separate. Execution order is randomized.

A smoke run executes each task/profile pair once. A release comparison executes every task/profile pair at least three times and reports a paired bootstrap 95 percent confidence interval for median deltas. More repetitions may be configured, but a result with fewer than three release repetitions cannot pass the rollout gate.

Each record includes exact model slug, backend, Pi commit, Codex commit, date, configuration hash, and verifier result. Authentication outages, provider unavailability, and exhausted rate limits are classified as infrastructure failures using predefined rules; they are never silently removed.

### 16.4 Task corpus

The initial release corpus contains 20 to 30 tasks covering:

- Codebase discovery.
- Local bug fixes.
- Cross-file features.
- Compilation errors.
- Test failures.
- Behavior-preserving refactors.
- Documentation-driven changes.
- Interactive commands.
- Parallel search.
- Large-output pressure.
- Permission denial.
- Cancellation.
- Continuation loss.
- WebSocket recovery.

Every task has a machine-verifiable criterion such as tests, builds, expected and forbidden changes, or a purpose-built verifier.

### 16.5 Metrics and release gates

Primary metrics:

1. Verified success rate.
2. End-to-end wall time.
3. Model response count.
4. Tool-call count.
5. Uncached input tokens.
6. Cached-input ratio.
7. Output and reasoning tokens.

Reports show median, p95, confidence intervals where sample size permits, and individual task results.

The optimized profile ships only when:

- Protocol and golden-task tests have no correctness regression.
- Live verified success is at least equal to the generic profile.
- Duplicate side-effect execution is zero.
- Cancellation and recovery suites pass.
- At least two major efficiency dimensions improve without materially worsening another.

For the rollout gate, material worsening means more than a 10 percent increase in the median of another primary efficiency metric or more than a 15 percent increase at p95. Verified success is stricter: the optimized profile must complete at least as many paired verified runs as the generic profile, and it may not introduce a new deterministic task failure.

Initial live-suite targets are:

- 25 percent lower median wall time.
- 30 percent fewer model responses.
- 25 percent fewer uncached input tokens.
- 60 percent cached-input ratio after the first stable turn in warm-cache scenarios.

These are targets to validate, not assumed outcomes.

## 17. Feature flags and attribution

Independent feature flags cover:

- websocket
- lossless_responses
- native_apply_patch
- tool_search
- explicit_caching
- adaptive_context

The evaluator supports ablation runs that disable one feature at a time. Complexity that does not produce measurable benefit is removed or remains experimental.

## 18. Implementation phases

### Phase 0: measurement foundation

Deliver traces, protocol fixtures, a deterministic smoke suite, paired live benchmark tooling, and baseline results.

Exit gate: repeated baseline runs distinguish harness changes from ordinary model noise.

### Phase 1: lossless Responses protocol

Deliver CodexCapabilities, typed request items, ResponsesLedger, assistant phases, opaque item preservation, deterministic shape hashing, and legacy-session lineage migration.

Exit gate: existing SSE tests pass and recorded native streams round-trip without dropped items.

### Phase 2: production WebSocket transport

Deliver account-level connection reuse, stream lanes, prewarming, incremental continuation, coordinated OAuth refresh, cancellation, reconnection, and exactly-once recovery.

Exit gate: fault injection passes for failures before output, after partial output, missing previous responses, retryable HTTP errors, expired authentication, and cancellation.

### Phase 3: Codex-native tool surface

Deliver typed definitions, exec_command, write_stdin, apply_patch, update_plan, native parallel calls, namespaces, and capability-gated tool search.

Exit gate: permission behavior remains equivalent, duplicate mutation is impossible, and smoke evaluations reduce response count without reducing success.

### Phase 4: cache and context controller

Deliver stable-prefix construction, cache modes, explicit breakpoint support, model-aware accounting, evidence-backed pruning, adaptive compaction, and retrieval references.

Exit gate: warm-cache reuse is observed, long sessions avoid overflow, and compaction retains verified success.

### Phase 5: rollout

Progression:

1. Experimental opt-in.
2. Opt-in with verbose metrics.
3. Automatic activation for ChatGPT/Codex OAuth with a rollback flag.
4. Default OAuth profile.
5. Removal of experimental status after two stable releases.

Public API behavior remains capability-driven and does not inherit subscription-only headers.

## 19. Crate boundaries

| Responsibility | Location |
|---|---|
| Capabilities, typed requests, native response items | crates/pi-ai |
| WebSocket transport and recovery | crates/pi-ai/src/codex.rs and focused submodules |
| Agent-visible adapters and permission mapping | crates/pi-agent |
| Context governor and session integration | crates/pi-agent |
| Fixtures, benchmark runner, reports, and gates | crates/pi-evals |

Large files such as stream.rs, stream_decoder.rs, and codex.rs are split by responsibility only as they are touched. Unrelated refactoring is outside scope.

## 20. Migration and rollback

Every persisted session records:

- Session format version.
- Provider profile.
- Capability snapshot.
- Request-shape hash.
- Tool-schema version.
- Model.
- Reasoning effort.
- Lineage boundaries.

A legacy session can always reopen through the generic transcript. It starts a new native lineage rather than attempting unsafe continuation.

Each feature flag is a kill switch. Disabling WebSockets, tool search, explicit caching, or adaptive compaction must leave the session readable and allow a safe new lineage.

## 21. Testing strategy

Testing is layered:

- Unit tests for capability decisions, hashing, canonical tool ordering, parsing, path safety, token budgeting, and retry classification.
- Property tests for stream fragmentation, unknown item round-tripping, patch parsing, and call-ledger idempotency.
- Integration tests for SSE and WebSocket turns, parallel tools, OAuth refresh, cancellation, compaction, and migration.
- Fault-injection tests for disconnects and partial provider state.
- Golden request and event fixtures from sanitized protocol recordings.
- Live paired tasks with machine verifiers.
- Compatibility tests confirming non-Codex providers retain their existing tool and message behavior.

No phase is accepted based only on compilation or model-written summaries.

## 22. Risks and mitigations

| Risk | Mitigation |
|---|---|
| ChatGPT backend behavior changes | Capability snapshots, conservative probes, feature flags, recorded backend metadata |
| Continuation loses required state | Lossless ledger, unknown-item preservation, full-replay recovery |
| Retry duplicates a mutation | Persistent exactly-once call ledger |
| Too many tool definitions consume context | Small hot set, namespaces, deferred loading |
| Cache optimization increases cost | Cache token telemetry, one initial breakpoint, ablation testing |
| Context pruning removes evidence | Full local evidence store, protected active chain, retrieval references |
| Codex-specific changes leak into other providers | Pi-owned interfaces and provider profile adapters |
| Live benchmark noise hides regressions | Paired runs, randomized order, repeated samples, task-level reporting |
| Large refactor becomes unreviewable | Phase gates, focused modules, test-first slices, independent flags |

## 23. References

- Pi Rust repository: https://github.com/J12003LPZ/pi-rust
- OpenAI Codex repository: https://github.com/openai/codex
- Latest model guidance: https://developers.openai.com/api/docs/guides/latest-model
- Responses WebSocket mode: https://developers.openai.com/api/docs/guides/websocket-mode
- Tool search: https://developers.openai.com/api/docs/guides/tools-tool-search
- Reasoning and preserved reasoning items: https://developers.openai.com/api/docs/guides/reasoning
- Prompt caching: https://developers.openai.com/api/docs/guides/prompt-caching
- Apply patch: https://developers.openai.com/api/docs/guides/tools-apply-patch
- Programmatic tool calling: https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling
- Responses multi-agent guidance: https://developers.openai.com/api/docs/guides/responses-multi-agent
- Error codes: https://developers.openai.com/api/docs/guides/error-codes

## 24. Final acceptance criteria

The design is implemented when all of the following are true:

1. OAuth Codex sessions use lossless native Responses continuation.
2. WebSocket recovery cannot duplicate a side-effecting tool.
3. Codex receives the compact native hot tool set and can discover deferred tools.
4. Stable prompt content remains cacheable across tool turns.
5. Long sessions prune or compact without losing active task state.
6. Public API and non-Codex providers continue to work through capability detection.
7. Protocol, deterministic, migration, fault-injection, and live benchmark suites pass.
8. Live results meet correctness gates and demonstrate material efficiency improvement.
9. Every major optimization can be disabled without corrupting sessions.
10. Documentation explains configuration, telemetry, benchmarking, migration, and rollback.
