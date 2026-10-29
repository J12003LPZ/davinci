# Claude Code-to-DaVinci Runtime Gap Closure — Master Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Achieve behavioral parity for every evidenced, applicable Claude Code CLI runtime capability missing or weak in DaVinci, while retaining stronger DaVinci safety invariants and measuring real regressions before changing defaults.

**Architecture:** Extend existing Rust workspace owners, never reimplement the harness wholesale. Use an evidence-to-owner crosswalk, isolated test-driven workstreams, original implementation of validated behavior, durable generation/identity contracts, opt-in experimental functionality, and an explicit acceptance ledger that includes justified non-adoption.

**Tech Stack:** Rust 1.88.0; Cargo workspace; existing JSONL and SQLite operation journal; native tools/MCP; local PowerShell/Windows and conditional Unix test environments; Python only for offline evidence and benchmark orchestration.

**Spec:** `docs/superpowers/specs/2026-10-28-claude-runtime-capability-convergence-design.md`.

## Global Constraints

- This is a **proposal and implementation backlog**, not evidence that any item is implemented. **No commits, code changes, paid model calls, credentials, auto-update, deployment or installation are authorized by this plan.**
- Claude local research root: `C:\Users\sergi\Desktop\lean-claude\re-claude-code\`; 305 register rows in 306-line CSV on 2026-10-28; static v2.1.310 `F9A392…EBA61`, new completion runs v2.1.312 `609967…C865`. Re-verify exact hashes and register before using offsets.
- Target checkout: `C:\Users\sergi\Desktop\davinci\`; local branch `J12003LPZ/cc-tui-parity` at `6912f940`, **dirty** with G-01 and in-progress G-02/G-06/G-07; upstream main at `c78605dd` at planning time. Never reset/checkout/overwrite another session's working tree. Choose a new worktree at execution time after approval and only when branch ownership allows.
- Current local `AGENTS.md` is `@CLAUDE.md`, whose instructions require minimal, goal-driven, surgical changes. The committed main `AGENTS.md` additionally requires preserving the Codex subscription path. At implementation time read both effective guidance and any nested `AGENTS.md`.
- Preserve `openai-codex` OAuth, user model/effort, permissions, safety-journal effect latch, provider capability gates, tool-result pairing, context provenance and source permissions.
- No proprietary binary/source transplantation, private prompt copying, auth bypass or imitation of Claude's unsafe replay and misleading success statuses. Only independently implement proven observable contracts.
- Pair tests with specific behavior; test RED then GREEN, validate, review diff; commits occur **only after separate owner approval**, despite normal Superpowers plan templates.
- No feature is "done" merely because code compiles. Functional/regression/perf/on-host acceptance must be recorded with commands, result codes, build identities and limits.
- Do not duplicate existing plans: `2026-10-22-codex-efficiency-implementation-plan.md`, `2026-10-23-orchestration-coordination.md`, `2026-10-21-resource-lifecycle.md`, `2026-09-20-claude-code-terminal-parity-implementation.md`. Review their latest actual implementation status first.

## Review Focus

1. Restart immediately after side effect but before tool result/notification: exactly one durable outcome; unknown stays unknown (V2, O3, S3).
2. Permission revision or workspace trust changes while a tool/worker waits: refuse stale authority (S2, O1, T1).
3. Summarization drops an old user restriction while latest code edit remains unverified: fail closed or report explicit unverified without extra automatic model turn (C1, V2).
4. Cached workflow result from another Git revision or untrusted worktree: force re-verification instead of reusing as verified (O3).
5. Interrupt/timeout during code verification or server reconnect: no false positive, no duplicated mutation, and bounded recovery (V2, S1, O2).

## Source inventory, attribution and crosswalk

The Claude reference contains numbered reports `01`–`20`, `31`–`55`, `call-graph.md`, `experimental-fixtures.md`, several Mermaid diagrams, raw capture campaigns and `evidence-register.csv`. No reports `21`–`30` are present; never treat numbering gaps as missing research. `17`, `43`, `54` contain algorithms; `44` and `55` carry the authoritative later corrections, including client completion vs task correctness. Capture a SHA manifest before implementation; do not edit the research corpus.

**Evidence ingestion gate:** Generate and maintain a one-row-per-finding CSV from all 305 F-IDs with fields `finding_id,claude_report,claude_class,claude_build,davinci_owner,disposition,gap_id,plan_task,test_anchor,decision_reason`. Its initial rows may be `REQUIRES_TRIAGE`, and cannot be counted complete. Every later `IMPLEMENTED` requires source/test evidence; each `BLOCKED` needs a named experiment.

## Capability groups → workstream owners

| ID | Gap or comparison decision | Claude evidence | DaVinci owner | Workstream | Priority |
|---|---|---|---|---|---|
| G-01 | Ambiguous MCP outcome | 20, 31, 42 | davinci-mcp; agent/mcp; tool_ledger | S1 / regression preservation | Closed in prior audit, reverify |
| G-02 | Worker verification receipts | 49, 55 | agent/subagent_receipt; subagent; coding-agent/main | V3 | P1, **in progress locally** |
| G-03 | Worktree teardown errors + reaper | 32, 36 | agent/worktree | O4 | P1 |
| G-04 | Constraint survival in default compaction | 07, 35, 41 | agent/compaction; context_vm; turn | C1 | P1 |
| G-05 | Workflow spec/status/notice durability | 05, 34, 50 | agent/runtime/workflow | O3 | P1 |
| G-06 | Valid coding success corpus | 40, 53 | davinci-evals/behavior | V1 | P1, **six valid scenarios now local** |
| G-07 | Explicit, persistent completion verification | 45–55 | agent/lib, turn, events | V2 | P1, **in progress locally** |
| G-08 | Default Full tool surface | 08, 13, 41 | agent/tool_surface; coding-agent/extension_host | C2 | P2 |
| G-09 | Local IPC auth hardening | 14, 38 | davinci-server/lib, unix | S3 | P2 |
| G-10 | Signal/child cleanup | 12, 36 | coding-agent/shutdown; davinci-sys/process | S4 | P2 |
| G-11 | Worker memory admission/timeout | 32, 39 | agent/runtime/worker_admission | O1 | P2 |
| G-12 | Stable MCP invocation identity | 06, 20, 42 | davinci-mcp; agent/mcp | S1 | P2 |
| G-13 | Rebind policy on compatibility dispatch | 11, 37 | agent/permission, turn | S2 | P2 |
| G-14 | Cancel-after-commit MCP test | 42 | davinci-mcp/tests; agent tests | S1 | P2 |
| G-15 | Collapsed authority arguments | n/a (local defect) | agent/turn | S2 | P3 |
| G-16 | Ledger 256-record eviction | 09, 36 | agent/tool_ledger | S1 | P3 |
| G-17 | Eager SKILL.md file reads | 13 | coding-agent/skills + extension_host | C3 | P3 |
| G-18 | Workflow cost cap unsupported | 34, 41 | agent/runtime/workflow/limits, executor | O5 | P3 |
| X-01 | Context-forked/nested agents (optional) | 03, 32 | agent/subagent; coding-agent/main | O2 | P3/experimental |
| X-02 | Background attach/list/logs/restart supervisor | 12, 36 | session; coding-agent; server | O6 | P2/opt-in |
| X-03 | Deterministic schema deferral in conversation | 08, 13 | agent/ToolSurface, tools; ai/responses_tools | C2 | P2 |
| X-04 | Scoped skills/plugin refresh and lifecycle hooks | 13 | coding-agent/skills; extension host; events | C3, S5 | P3 |
| X-05 | IDE authenticated peer + diagnostics | 14, 38 | coding-agent/native_extensions/language_intelligence; server | S6 | P3/blocked on interactive evidence |
| X-06 | Observability/usage/event parity | 15, 39, 41, 52 | agent/events; ai/provider_observation; bench | B1 | P2 |
| X-07 | Tool scheduler concurrency + ordered pairing audit | 10, 17 | agent/turn; batch; tools | B2 | P2 (only if gap demonstrated) |
| X-08 | Session fork/crash recovery and pairing | 09, 36 | davinci-session; agent/turn | B3 | P2 (differential tests) |
| X-09 | Compaction summary pipeline / idle compaction | 07, 35 | agent/compaction; context_vm | C1 | P3/partially established |
| X-10 | Cache/cache-window optimization | 08, 16, 41 | ai/stream; cache_stats; benchmark | C4 | P2/measure first |
| X-11 | Safety of permissions, sandbox, hook events | 11, 13, 37, 51 | agent/permission; sandbox; event hooks | S2, S5 | P2/differential tests |
| X-12 | Startup, RSS, storage, hot sessions | 01, 16, 39 | coding-agent; sys; session; benchmark | B1 | P2/measurable |
| X-13 | Agent-team IPC/peer coordination | 04, 33 | agent/team; mailbox; server | O7 | P3/Claude peer contract partially unknown |
| X-14 | Model-coding success comparison | 40, 53, 55 | davinci-evals, benchmark | V4 | P1 design, execution only when authorized |

`C`, `O`, `S`, `V`, `B` task IDs are defined by the workstream plans. Several X-rows are verification or opt-in exploration, not automatically feature requests.

## Phase plan / order

**Phase 0 — Read, freeze and reconcile (master tasks M0–M3).** Exactly pin all input refs, current checkout, files and tests. No code changes; resolve existing work before proposing duplicates.

**Phase 1 — Truth and measurement (V1–V4).** Valid offline oracles, persistent verification states and host-owned worker receipts; integration pass. Crucial foundation before claiming improvements.

**Phase 2 — Durable orchestration (O1, O3, O4, O5, O7).** Bounded capacity; workflow recovery and notices; worktree integrity; team messaging contracts; budget features.

**Phase 3 — Context/cache/tools (C1–C4).** Requirement-preserving compaction; cache-stable deferred tools; genuinely lazy skill I/O; provider-byte measurements and optional retention changes.

**Phase 4 — Safety, lifecycle and integration (S1–S6).** MCP identity/cancel behavior, policy rebind, IPC trust, owned process reaping, hook lifecycle, IDE bridge only when evidence and interfaces exist.

**Phase 5 — Advanced optional parity (O2, O6, B1–B3).** Bounded nested/fork agent and detached supervisor after resource/durability; event/performance/conformance tests.

**Phase 6 — Scoped authorized model study (V4/C4).** Real paired tasks with same model+effort+task/time and explicit request budget; never fabricate subscription-quota improvement.

## Master tasks

### Task M0: Pin and reconcile current sources (must precede every workstream)

**Files:** Read `AGENTS.md`, `CLAUDE.md`, `rust-toolchain.toml`, `Cargo.lock`; read `docs/audit/claude-code-parity/{00..12}*.md`, `implementation/validity-report.json`, existing overlapping plans. Create at execution time `docs/audit/claude-code-parity/implementation/current-source-baseline.md`.
**Interfaces:** Inputs are two repo SHAs, status, 305 evidence rows, current Rust owners; output a stable manifest and per-feature state. No API call.
- [ ] Record `git status --porcelain=v1`, `git rev-parse HEAD`, `git rev-parse origin/main`, `git worktree list`, `rustc --version`, `cargo --version`.
- [ ] Confirm register rows via `python -c "import csv;..."` (count rows, unique IDs, no empty reproduction). Snapshot manifest hashes with `Get-FileHash` on Windows or `sha256sum` on Linux.
- [ ] Review uncommitted G-01/G-02/G-06/G-07 diff and affected existing tests; do not copy their changes into a new branch unless the owner arranges an isolated snapshot/patch.
- [ ] Verify corpus validity classification (6 meaningful scenarios, 90 invalid + 110 structural-only) against current data; do not count as model success.
- [ ] Produce status matrix `ALREADY_DONE/IN_PROGRESS/MISSING/WEAK/DIFFERENT_SAFE/BLOCKED` with changed paths and source evidence.

### Task M1: Close the complete 305-finding crosswalk

**Files:** Create `docs/audit/claude-code-parity/implementation/claude-finding-crosswalk.csv` and `.../gap-closure-decisions.md`; update only these files once approved.
**Interfaces:** Each CSV F-ID maps to its original subsystem and a DaVinci disposition plus test/plan task; no orphan or duplicate finding.
- [ ] Generate CSV with every finding once, retaining original ID/class/version, marked `REQUIRES_TRIAGE`.
- [ ] For each F-ID inspect its referenced capture or static anchor; decide existing/different/weak/missing/blocked/rejected, link Rust owner and workstream task.
- [ ] Assert CSV row count equals evidence register count; all F-IDs match once; **zero untriaged** at decision freeze.
- [ ] For a finding affecting an unknown Claude path, record `blocked` with exact missing fixture/chunk/authorization instead of guessing or constructing a fictitious interface.
- [ ] Independent review rejects any alleged "Claude improvement" supported only by `DOCUMENTED` or unsupported generalization.

### Task M2: Lock common interface, test and rollback contracts

**Files:** Create `docs/audit/claude-code-parity/implementation/parity-contracts.md`; reference `runtime/operations`, `CompletionStatus`, `VerificationReceipt`, `AgentEvent`, `ToolSurface`, workflow records.
- [ ] Define stable source identity, worker authority, result revision, verification scope, cache identity and request usage without new duplicate stores.
- [ ] Negative tests planned for all invariants I-01–I-10 from spec; map to workstream files.
- [ ] Establish feature flag / configuration rollback for opt-in advanced parity. Avoid new mandatory provider calls.
- [ ] Map platform matrices Windows/Linux, fresh process/resume, headless/interactive, direct/batch/codemode/MCP.

### Task M3: Baseline and release gate

**Files:** `docs/perf/claude-runtime-gap-closure/baseline.md`, existing `scripts/bench` suite. No new benchmark runner unless absent.
- [ ] Offline benchmark a DaVinci short headless turn and warm session; measure to first model-request, wall, RSS, handles, binary size and disk footprint; >=5 comparable replicates.
- [ ] Record Claude's single-host 4.014–4.063 s wall and 300.6–334.3 MB **only as historical context**; no paired superiority claim from unlike host/test shapes.
- [ ] Run `cargo test --locked -p davinci-agent --lib`, `cargo test --locked -p davinci-mcp`, `cargo test --locked -p davinci-evals` and applicable workspace checks. Record unrun tests explicitly.
- [ ] Verify no live model calls; isolate disposable fixtures. Use `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --offline` where environment permits.
- [ ] Define per-wave release gate: correctness no regression, new tests pass, staged fields backward-compatible, no unknown-outcome replay, metrics present or explicitly unavailable.

## Dependencies and sequencing rules

`M0 → M1/M2/M3`; `V1 → V2 → V3 → V4`; `O1 → O2/O5`; `O3 → O6`; `C4 offline baseline → C2 default experiment`; `C1 requirement gate → any changed default`; `S1 idempotency identity → any MCP reconnect`; `S3 auth → S6 IDE/peer transport`. Independent items may run after M0 with separate code owners and worktrees; never parallel-edit common agent/lib, turn or main without owner coordination.

## Completion criteria for this *program* (not for this plan document)

- 100% of 305 current source findings have an explicit defensible disposition; newly appended findings get the same treatment. No "exact parity" completion while evidence remains untriaged.
- Every approved applicable gap is independently tested in DaVinci or explicitly blocked; no dual implementation for an existing owner.
- All known replay, permission, cancellation, result-corruption and verification invariants pass in fault injection.
- Former `G-02/G-06/G-07` local changes have been independently reviewed and integrated or clearly rejected, not rewritten.
- The original core-200 is never included in a success denominator until it has valid oracles. Success-corpus validity != agent performance.
- Default changes require measured net improvement with no correctness regression; subscriptions savings remain unknown unless provider reports it.
- Final end-to-end review checks docs, versioned interfaces, migration, backward-compatible session data, platform behavior, configuration rollback and all relevant offline suites.
- Owner receives separate impact ledger: confirmed behavioral gains, measured time/bytes/RSS, unverified hypotheses, blockers and deferred features.

## Workstream documents

- `2026-10-28-claude-runtime-verification.md` — V1–V4
- `2026-10-28-claude-runtime-orchestration.md` — O1–O7
- `2026-10-28-claude-runtime-context-tools.md` — C1–C4
- `2026-10-28-claude-runtime-integration-safety.md` — S1–S6, B1–B3

**Execution handoff:** This plan is not approved for implementation yet. Review it, resolve whether optional advanced parity features should be enabled by default or stay opt-in, then select native or subagent-driven execution. Preserve the dirty checkout until that decision is made.