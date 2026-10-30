# Runtime Integration, Security, MCP, Lifecycle and Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task. All steps use checkboxes.

**Spec:** `docs/superpowers/specs/2026-10-28-claude-runtime-capability-convergence-design.md`. **Master:** `docs/superpowers/plans/2026-10-28-claude-runtime-gap-closure-master.md`.
**Global constraints:** Rust 1.88/Cargo locked; owner approval before implementation and commits; preserve dirty working tree and openai-codex model/effort/OAuth, effect latch, permissions and user constraints; use deterministic offline tests. File paths are repo-relative, review at execution time. A plan item is not a completed change.

**Review focus:** stale verification after edits; restarted tasks losing receipts; permission revocation mid-operation; error after side-effect commit; unrelated tests passing falsely. Every corresponding task must test relevant cases.

**Goal:** Adapt proven lifecycle and integration capabilities with DaVinci's stronger fail-closed boundaries intact. **Architecture:** upgrade existing MCP and permission owners, use authenticated optional host services, centralize event identities and benchmark on comparable local fixtures. **Tech Stack:** `davinci-mcp`, `davinci-agent`, `davinci-server`, `davinci-sys`, `davinci-coding-agent`, existing test/bench packages.

### Task S1 — MCP identity, cancel and safe availability (G-01, G-12, G-14, G-16)
**Files:** `crates/davinci-mcp/src/{lib.rs,stdio.rs}`, `tests/*`; `crates/davinci-agent/src/{mcp.rs,tool_ledger.rs,turn.rs}`. **Interface:** `Error::{Ambiguous,NotDelivered}`, `ToolError::MayHaveRun`, stable logical invocation ID, `EffectStatus::Unknown`.
- [ ] RED: response lost after committed mutation; direct and collateral replay with concurrency; 404 after commit; cancellation after commit; server reset; long session >256 ledger entries; duplicate tool call IDs. Assert actual side-effect count 1, or stop unknown without re-execution.
- [ ] Verify the existing uncommitted G-01 fix first; do not duplicate it. Add stable invocation id inside permitted MCP metadata only when interoperable, never reveal internal identity in user text. Preserve server-level isolation and conservative eviction.
- [ ] Future reconnect may be opt-in **only** after a cooperating server demonstrates dedupe and a proven not-delivered signal; separate transport reconnection from replaying a tool call. Otherwise retain fail-closed behavior.
- [ ] GREEN `cargo test --locked -p davinci-mcp`, affected agent/tool-journal recovery integrations and cancellation fixtures.

### Task S2 — Permission revision, sandbox and hook gating (G-13, G-15, X-11)
**Files:** `crates/davinci-agent/src/{permission.rs,turn.rs,runtime/adapters/tools.rs,events.rs}`, current sandbox owners and tests. **Interface:** preserve one dispatch choke point, policy version and operation-authority scope.
- [ ] RED: change deny policy between prepare and compatibility dispatch; child batch/codemode/MCP denied end-to-end; headless approver missing; symlink escape; forbidden tool dynamically added after catalog refresh.
- [ ] Bind policy revision and owner/cancel token to every dispatch route; remove duplicated `execution_authority(id, journal_configured, journal_configured)` semantics only after inspecting the intended flags. Preserve ReadOnly and no implicit headless approval.
- [ ] Review hook event routing: pre/post tool, batch, failure, compaction, permission and task events. Only add verified missing callbacks with schema and timeouts; no fail-open security gate, no injecting untrusted hook output as authority.
- [ ] GREEN permission, batch, codemode, sandbox and policy-revision suites.

### Task S3 — Authenticate local IPC before peer/IDE expansion (G-09)
**Files:** `crates/davinci-server/src/{lib.rs,unix.rs}`; conditional Windows pipe module only if designed and tested; `crates/davinci-coding-agent/src/main.rs`. **Interface:** token mandatory, constant-time authentication, client identity and workspace binding.
- [ ] RED: empty token refuses listener; wrong token denied; timing-insensitive compare; Unix peer credentials incompatible owner refused; stale socket replacement/TOCTOU; unauthorized cross-session command denied.
- [ ] Preserve private 0700 socket dir and ownership, add peer validation and explicit unsupported Windows state until secure Windows named-pipe transport exists. No unauthenticated fallback.
- [ ] GREEN server integration tests on Unix; Windows platform-specific negative tests.

### Task S4 — Process and owned resource cleanup (G-10)
**Files:** `crates/davinci-coding-agent/src/shutdown.rs`, `crates/davinci-sys/src/process.rs`, tool/subagent process owners.
- [ ] RED: SIGTERM/SIGHUP during foreground child; Windows terminal job close; blocked stdout reader; daemon or LSP survivor; shutdown while notifying peer.
- [ ] Replace abrupt `process::exit` destructor bypass with bounded explicit cleanup; attach managed Windows foreground jobs to kill-on-close job object without killing unrelated user processes. Record incomplete cleanup as durable diagnostic.
- [ ] GREEN platform-relevant process tests, no lingering owned children; verify existing cancellation tree invariants.

### Task S5 — Versioned hooks/observability contract (X-04, X-06)
**Files:** `crates/davinci-agent/src/events.rs`, coding-agent host event rendering, `crates/davinci-ai/src/provider_observation.rs`, existing bench and session stores.
- [ ] RED: background completion between emit and subscriber attach; duplicate lifecycle event; stale generation; large output/secret redaction; hook failure/timeout; hook loop cap; coherent task usage metadata.
- [ ] Specify versioned typed event payload for init/tool/permission/worker/workflow/compact/model switch and final outcome; journal where correctness requires, forward interactive events where supported. Never treat optional Stop hook as a correctness oracle.
- [ ] GREEN serialisation, restart/duplicate, hook failure and UI host tests. Exclude raw credentials/prompts from default logs.

### Task S6 — Optional IDE + authenticated peer integration (X-05)
**Files:** Existing `crates/davinci-coding-agent/src/native_extensions/language_intelligence/*`, `crates/davinci-server`, TUI host integration only with approved UI contract.
- [ ] Prepare fixture for selection/current file, diagnostics, diff, reconnect, authenticated handshake, stale port and permission refusal; execute real TTY/IDE peer experiment only with approval. Claude's IDE handshake is not dynamically established; do not assert wire parity with unavailable peer handlers.
- [ ] Prefer documented/native LSP+host protocol over cloning internal Claude `ws-ide` messages. Keep IDE connector disabled and no extra startup sockets when not configured.
- [ ] GREEN local mock protocol, rejection tests, cross-platform unsupported behavior; live IDE gate recorded separately.

### Task B1 — Performance/usage baseline and conformance
**Files:** `scripts/bench/*`, `docs/perf/claude-runtime-gap-closure/*`. **Interfaces:** startup first request, wall, RSS, disk, per-worker memory, request count, results/cached usage with unknown values.
- [ ] Write reproducible paired mock turn/worker/workflow test on the same hardware, config, OS; n>=5 and distinguish short `--version` fast path from full turn; no fake model latency.
- [ ] Measure new runtime capabilities one at a time, retain baseline and rollback; source Claude startup 3.46–3.58 s and 300.6–334.3 MB as historical only.
- [ ] Do not ascribe improved verified coding success or quota to bytes/RSS. Add under-load queue, panic/cancel, handle count and binary footprint checks.

### Task B2 — Core loop/scheduler parity audit
**Files:** `crates/davinci-agent/src/{turn.rs,batch.rs,tools.rs}`, `crates/davinci-ai/src/stream_decoder.rs`.
- [ ] Model two tool calls with different completion orders; injected permission deny, cancellation, output caps, invalid tool inputs; assert stable call/result association and sequential mutation barriers.
- [ ] If already equivalent, close with tests. Do not copy Claude's numeric concurrency-10 limit or unsafe retries.

### Task B3 — Session history and interrupt recovery
**Files:** `crates/davinci-session/src/lib.rs`, `crates/davinci-agent/src/turn.rs`, existing journal/recovery modules.
- [ ] RED: torn last JSONL line, orphan result, unresolved tool call, session fork, background resume after process kill, compaction during interrupt and missing journal. Preserve source bytes before repair.
- [ ] Extend only unverified paths; preserve session tree and SQLite operation ledger separately. GREEN affected tests on Windows and Linux.

**Acceptance:** no provider/tool duplication, permission leakage or orphaned owned processes; optional IDE/daemon separate; API usage and performance measured only with valid sources; all remaining unknowns listed, not guessed.
