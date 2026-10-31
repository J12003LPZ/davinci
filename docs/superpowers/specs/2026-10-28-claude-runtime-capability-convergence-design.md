# DaVinci runtime capability convergence — design specification

**Status:** Proposed for owner review; this document and its companion plans authorize no implementation, commits, model calls, installation, binary patching, or deployment.
**Date:** 2026-10-28.
**Reference:** `C:\Users\sergi\Desktop\lean-claude\re-claude-code\` (Claude Code CLI 2.1.310 static analysis; 2.1.312 completion experiments). **Target:** `C:\Users\sergi\Desktop\davinci\` (local branch `J12003LPZ/cc-tui-parity`, HEAD `6912f940`, dirty working tree); remote `main` at `c78605dd` as of this review.
**Evidence:** Claude evidence register 305 distinct findings / 306 CSV lines, including 104 findings from reports 45–55. The older DaVinci 52-row matrix and 18-gap audit predates local G-02/G-06/G-07 work and is NOT a live implementation status.

## Intent, success and anti-goals

Build an **original Rust implementation of each justified, evidenced Claude Code runtime capability that DaVinci lacks or implements weakly**. Reproduce observable behavioral contracts, not minified proprietary source, tool descriptions, full prompts, or undocumented mistakes. The intended outcome is greater capability and measured reliability without losing DaVinci's stronger journal/permission/cancellation guarantees or degrading Codex subscription efficiency.

**Acceptance definition:** Every *applicable* Claude runtime finding maps to one of `implemented+tested`, `different but valid`, `already implemented`, `blocked on evidence`, `experimental opt-in`, or `explicitly rejected (reason)`. Every gap that is approved for implementation has a Rust owner, failing regression, independent test, rollback, and success metric. 'Exactly the same' means compatible behavior under the same scenario, NOT implementation-language/API syntax/constant matching or unmeasured task quality.

**Non-goals:** copying proprietary Claude Code code; rebranding DaVinci as Claude; changing Codex auth/provider/selected model or reasoning effort; copying unsafe MCP replay, misleading success statuses, stale workflow cache, fail-open Stop hooks or arbitrary concurrency numbers; changing the terminal visual UI (covered by a separate parity plan); manufacturing subscription savings or coding-success claims. Further reverse engineering or live-provider comparison is separately authorized, not assumed.

## Real-state guardrail

The check-out is dirty and not the `main` branch. The most recent audit claims 18 G-gaps and one fix; current uncommitted additions include `crates/davinci-agent/src/subagent_receipt.rs`, `completion_status_tests.rs`, `crates/davinci-evals/src/behavior/{validity,oracle}.rs`, `verified-core.json` and six fixture repositories. The local `implementation/validity-report.json` states verified-core 6/6 valid scenarios; old core-200: 0 valid, 90 invalid, 110 structural-only. Those values are *validation classification*, **not AI task success**. Rebase source mappings at execution time and test these changes before declaring a gap open or closed. Preserve uncommitted edits. Do not create another parallel verification subsystem.

## Approaches and decision

1. **Literal transplant / full parity:** translate the private JS bundle and copy model prompts and quirks. Rejected: proprietary source, brittle against CLI versions, and introduces proven hazards.
2. **Rust behavioral convergence (chosen proposal):** use Claude's verified external contract as a reference, extend DaVinci's existing owners, preserve stronger local invariants, and evaluate before changing defaults. Small independently verifiable PR-sized increments.
3. **Minimal P0/P1 hardening only:** safer but does not deliver the requested full capability inventory. Used only if an item is unverified or clearly harmful.

The plan follows option 2; owner approval is required before execution, not for writing a proposal.

## Architectural invariants

**I-01** Never automatically replay a possibly-committed tool or MCP mutation. A 404, disconnect, timeout or cancellation does not prove it did not run.
**I-02** Durable operation intent precedes side effect; unknown outcome and model-visible recovery survive restart. Preserve the existing SQLite effect latch, JSONL session store, and tool ledger.
**I-03** All dispatch paths must re-check applicable deny rules, approval scope, cancellation, owner/lease and policy generation at the execution boundary.
**I-04** User constraints, trusted sources and unresolved tool pairs survive compaction, session fork/resume and worker handoff; no unsafe cross-context authority.
**I-05** Agent completion, host verification and task correctness are separate fields. Worker claims cannot become verified solely by exit 0 or final prose.
**I-06** Background work has durable identity, explicit ownership, bounded resource admission and restart-recoverable completion evidence.
**I-07** Model/provider/effort are user-controlled. `openai-codex` subscription/OAuth remains native. No API-key fallback, hidden paid calls, speculative fan-out or mandatory model review.
**I-08** Tools, skills, hooks and IDE peers remain authorization-scoped. Discovery never grants execution privileges. Dynamic content cannot silently mutate a stable privileged prefix.
**I-09** Provider cache observations, request bytes, token counts, subscription quota and coding success are separate measures. Missing usage is unknown, never zero.
**I-10** No changes to existing source, external services or installed binaries as part of this planning pass; no commits/pushes without approval.

## Runtime capability families

| Family | Claude reference | DaVinci baseline / target |
|---|---|---|
| Core loop and tool batching | 02, 10, 17, 43 | Existing Rust loop; preserve result pairing/order; parity only where failing tests demonstrate a gap |
| Dynamic tool search | 08, 13, 41 | Full default; Lean exists; inspect tool discovery before introducing provider-compatible deferred schema append |
| Skills/plugins | 13, 15 | Existing skills and extension host; defer disk body reads, preserve scope/permission, add bounded lifecycle hooks only where useful |
| Delegation/fork/depth | 03, 32, 49 | Depth 1 and fresh worker; add *optional* safe context fork/deeper worker after measured justification, never auto-fan-out |
| Teams / mailbox | 04, 33, 49, 50 | Durable task board/results; investigate inter-process comms and owner identities; no unverified team's success claim |
| Workflow/journal/cache | 05, 34, 50 | Existing phases/artifacts; make spec/launch/status/notices durable; invalidate stale output on repo changes |
| Sessions/daemon | 09, 12, 36 | Existing JSONL + operation SQLite; implement opt-in attach/logs/stop/list/resume UX without reducing durability |
| Context/compaction | 07, 35, 41 | Off default, Hybrid/Context VM option; preserve requirements in default path; run *offline fault cases* before defaults change |
| Cache/subscription | 08, 16, 39, 41 | Cached usage parser exists; measured efficiency requires approved backend route; use stable-prefix & schema experiments offline |
| MCP | 06, 20, 31, 42 | Existing conservative unknown-outcome G-01; add server-identity safe reconnect only with dedupe/idempotency proof |
| Permissions/sandbox | 11, 37, 51 | Existing deny-first and journal policy latch; close compatibility route + IPC auth + Windows/Unix controls |
| IDE/LSP | 14, 38 | DaVinci extension/LSP existing; optional IDE peer bridge requires native authenticated client & TTY validation |
| Observability | 15, 39, 52 | Record schema-tagged lifecycle events and background continuity; no raw secret or full prompt logging by default |
| Completion/verification | 40, 45–55 | G-02/G-06/G-07 underway; do not copy Claude's known lack of correctness gate |
| Performance/resources | 16, 39 | Measure DaVinci baseline; bound worker queues, lazy startup and worktree cleanup; use matched host/protocol benchmarks |
| Unknowns | 18, 38, 42, 44 §24, 55 §55.6 | Explicitly mark test-blocked claims; never implement speculative parity as proven |

## Known stronger DaVinci design choices — retain

- G-01 ambiguous MCP outcome and no automatic replay; Claude's direct/collateral replay is a verified negative example.
- Durable SQLite effect latch, unique operation identity, conservative replay reconciliation.
- Deny precedence and no implicit permission approval in headless contexts.
- JSONL append durability, torn-tail handling, atomic compaction commit, cancellation tree and task-board leases.
- Context VM source-grounding, explicit completion generation and planned host-owned subagent receipts.
- Content-addressed workflow artifacts and rerun authority, rather than trusting Claude's script/prompt-only cache key.

## Evidence and decision labels

`PROVEN_STATIC`, `PROVEN_DYNAMIC`, `DOCUMENTED`, `INFERRED`, `UNKNOWN`, `NOT ESTABLISHED` describe **Claude evidence**. `IMPLEMENTED_AND_TESTED`, `EXISTS_UNVERIFIED`, `WEAK`, `MISSING`, `DIFFERENT_SAFE`, `EXPERIMENTAL`, `REJECTED_UNSAFE`, `BLOCKED` describe **DaVinci disposition**. Never convert one taxonomy into the other. Maintain a traceability row for each F-ID, and group IDs only after each has a disposition; higher-level report counts are not feature counts.

## Cross-cutting acceptance gates

- **Offline correctness:** test goes RED against the observed weakness and GREEN against the new implementation; full relevant regression suite, fmt, Clippy, no unexpected workspace failures.
- **Faults:** crash/panic/timeout/cancel/slow worker/corrupt journal/denied permission/concurrent mutation, including real filesystem consequences, tested per applicable owner.
- **Performance:** cold/warm startup, first request, peak working set, handle/thread count, idle RSS, agent admission, I/O counts, binary and disk artifacts; paired on the same host/config, n>=5 for exploratory performance, do not use Claude's historical numbers as a benchmark of DaVinci.
- **Capability:** headless and interactive where supported; Windows and Linux conditional tests and explicit unsupported status, never fake cross-platform support.
- **Model outcome:** a 6-valid-scenario oracle suite proves fixture discrimination only. Live model comparisons require consent, fixed model+effort, paired scenarios, pre-registered budget and no API fallback.
- **Risk:** keep defaults unchanged until compatibility, latency, correctness and quota gates pass; feature-flag opt-in for nested/fork, session daemon, interactive IDE and new reconnect.

## Review focus (additional cases the plans must test)

1. A schema/catalog/skill revision mid-conversation must not invalidate old tool IDs or grant newly authorized execution from stale UI/context.
2. Compaction during cancellation or a partially published tool result must preserve tool pairing, latest user constraint and the original recoverable bytes.
3. Background session exits between durable outcome and notifying a parent: resumed status must show the outcome exactly once.
4. A worker writes after a prior passing verification: host receipt must become stale/unverified, including cross-thread/worktree changes.
5. A simultaneous denied action and queue admission must not launch a tool/worker after the permission or cancellation revision changes.

## Workstreams, ordering and handoff

Read `docs/superpowers/plans/2026-10-28-claude-runtime-gap-closure-master.md` first, followed by the independently executable workstreams:
`2026-10-28-claude-runtime-verification.md`,
`2026-10-28-claude-runtime-orchestration.md`,
`2026-10-28-claude-runtime-context-tools.md`,
`2026-10-28-claude-runtime-integration-safety.md`.
All are plans, **not work completed**. The older plan under `2026-09-20-claude-code-terminal-parity-implementation.md` concerns UI; do not duplicate it here.

**Source limitations:** 2.1.310 JS carve omits lazy modules; 2.1.312 completion dynamic runs are model-scripted; Claude compaction did not fire; team IPC, IDE interactive handshake, native hook hosts, cross-chunk completion predicate and provider quota behavior remain uncertain. The spec makes no claims beyond those boundaries.

**Owner review request:** Approve the behavioral-adaptation approach and workstream scope before any implementation. A project-level design spec and plan are provided together because the user explicitly requested the full plan in this session.