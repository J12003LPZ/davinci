# Claude-Derived DaVinci Improvements — Superpowers Implementation Program

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This document is a **planning deliverable**, not authorization to modify source, start paid inference, commit, push, or deploy.

**Goal:** Improve independently verified coding-task completion and runtime correctness of DaVinci by testing 15 Claude-Code-inspired hypotheses against mechanisms DaVinci already implements, making only evidence-justified changes.

**Architecture:** Retain DaVinci's Rust turn loop, durable operation journal, Context VM, verification receipts, capability scheduler, progressive MCP disclosure, and evaluation harness. Audit direct production paths and end-to-end recovery behavior first; add failing regression tests, make minimal repairs, and evaluate efficacy via controlled paired runs. Avoid parallel replacements for existing subsystems.

**Tech Stack:** Repository-pinned Rust **1.88.0**, current Cargo.lock, existing SQLite/JSONL/session infrastructure, existing local/loopback fixtures, `davinci-agent`, `davinci-ai`, `davinci-evals`, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md` (existing repo design), the attached *Claude Code CLI — Runtime & Workflow Reconstruction* (19 Oct 2026), and this program's explicit evidence/acceptance contracts. Existing related plans: `docs/superpowers/plans/2026-09-14-davinci-harness-optimization-master.md`, `docs/superpowers/plans/2026-10-01-openai-harness-production-plan.md`. This is an **audit/delta extension**, not an instruction to reimplement those plans.

**Source snapshot:** `main@cd4e0e1560e920a1026507358a6e9c1cffc5b96f` (GitHub commit, 18 Oct 2026 UTC). Re-check source/test locations at execution time because the branch will move.

**Status:** Proposed; **no source edits, builds, tests, live model calls or commits were performed** while authoring this plan. GitHub file contents and file-tree/test inventory were inspected; assertions about code presence do not imply tests were run today.

## Global constraints

- Honor repo `AGENTS.md`: preserve the user's primary `openai-codex` ChatGPT subscription workflow. **No API-only fallback, new paid API requirement, or silent model/effort change.** Preserve other existing providers' compatibility.
- Honor existing safety/authority contracts, sandbox/permissions, append-only durable evidence, user instructions, worktree and cancellation boundaries. Do not downgrade guarantees to copy Claude Code behavior.
- **Do not commit, push, merge, deploy, change credentials, or modify the installed DaVinci without separate user authorization.** During execution, use a disposable isolated worktree and submit reviewable diffs; any commit step is conditional on authorization.
- Keep pinned toolchain and dependencies; add no dependency without a demonstrated need. No new scheduler/context/ledger framework unless current owner cannot meet a pinned invariant.
- Offline fixture and loopback tests by default. **No authenticated model runs or subscription allowance consumption without explicit scoped approval.**
- Never equate a normal CLI terminal result with verified task success. Unknown operation outcomes, missing usage, unsupported observations and inconclusive verification stay *unknown*.
- Features already covered by strong production-path tests are **closed as parity**, not reimplemented. Keep the work small; every proposed product change needs a red regression or measured quality/latency win.
- Evaluate coding outcomes, not similarity of architecture or claimed pass rate. Same-model comparisons are required for a harness-causality conclusion; mixed-model competitor runs must be labeled product-level comparisons.

## Review focus — high-risk failure inputs that must be pinned by tests

1. Kill a started shell side effect between effect execution and durable publication: resume must not blindly replay; outcome unknown until independent reconciliation (**R1**).
2. A stream ends after partial tool JSON or a tool result is absent after compaction/resume: no malformed provider history and no unintended operation (**R2**).
3. A slow summarizer finishes after a newer user turn: old summary cannot replace newer authority or mutate its generation (**R3**).
4. Two concurrent owners request the same branch/worktree while another contains uncommitted changes: only one lease; dirty changes preserved (**R4**).
5. A task's last code mutation is newer than its last passing check; a masked, collect-only or irrelevant check is not proof of completion (**B1**).

## Existing capabilities — audit before building

The repository already has substantial, relevant code/tests:

- **Recovery:** `src/runtime/operations/{recovery.rs,retry_safety.rs,store.rs}`, `tests/{operation_crash,operation_recovery,operation_idempotency,operation_ownership,operation_publication}.rs`.
- **Provider streaming:** `tests/ws3_stream_cut.rs` already documents retry for incomplete tool streams without dispatching partial calls.
- **Completion:** `src/{completion.rs,turn.rs,verification.rs}`, `tests/{inline_verification,completion_requirements,harness_requirement_completion}.rs`.
- **Debugging:** `src/prompt/capabilities/debugging.rs` already demands reproduce-first and regression tests; behavior fixtures already include hard debugging tasks.
- **Context:** `src/compaction.rs`, `src/runtime/context_vm/`, `tests/context_vm_{integrity,durability,replay,prefix_stability}.rs`.
- **Tools/agents:** `src/scheduler.rs` has parallel/serial lanes default width 8; `src/runtime/worktree.rs` has worktree leases; `tests/mcp_progressive_disclosure.rs` demonstrates deferred schema exposure.
- **Benchmarking:** `crates/davinci-evals` and `.github/workflows/competitor-differential.yml` already exist. Do not create a redundant evaluator.

## Workstreams and dependency order

| Gate | Plan | Deliverable | Hypotheses covered |
|---|---|---|---|
| G0 — before all changes | [04 — Measurement, task M1](04-measurement-and-rollout.md) | Frozen baseline SHA, coverage/gap register, known failures, executable test profile | #14, #15 |
| G1 — correctness | [01 — Runtime Integrity](01-runtime-integrity.md) | Real-path crash, pairing, compaction, worktree race guarantees | #1, #2, #3, #10 |
| G2 — solve quality | [02 — Coding Success](02-coding-success.md) | Completion proof, debugging behavior, continuity, repair loop | #4, #5, #7, #8 |
| G3 — selective optimization | [03 — Orchestration & Efficiency](03-orchestration-efficiency.md) | Per-call safety evaluation, delegation, cache, discovery, pressure | #6, #9, #11, #12, #13 |
| G4 — decision / release | [04 — Measurement & Rollout](04-measurement-and-rollout.md), M2–M4 | Structured telemetry, paired eval report, accept/revert decision | #14, #15 |

**Parallelism:** Workstreams G1 and G2 may proceed in separate isolated worktrees after G0; within a workstream, honor order. G3 changes must not start before their corresponding G1 invariants are pinned. G4 instrumentation begins with G0 and extends as needed.

## Original 15-item coverage matrix

| # | Original idea | Planned disposition | Owner |
|---|---|---|---|
| 1 | Crash-safe tool execution | Test actual CLI/Agent shell lifecycle, not just journal unit tests; repair if red | R1 |
| 2 | Transactional compaction | Prove fresh revision/generation at publication | R3 |
| 3 | Tool-result integrity | Missing/duplicate/orphan IDs across cut/resume/compact | R2 |
| 4 | Evidence-based completion | Fresh verification of latest edits; no false verified-success | B1 |
| 5 | Better debugging workflow | Already present as prompt; measure task behavior, change only if failing | B2 |
| 6 | Adaptive scheduling | Test argument-specific safe conflicts vs width-8 control | O1 |
| 7 | Better context retention | Test exact constraints, file paths and evidence survives fold/resume | B3 |
| 8 | Automatic failure recovery | Test bounded correction after failed reproducer/check | B4 |
| 9 | Smarter subagent delegation | Test bounded ROI, output provenance and owner fences | O2 |
| 10 | Safer worktree coordination | Atomic lease race + dirty-worktree preservation | R4 |
| 11 | Prompt-cache optimization | Stable prefix, exact usage/accounting; no fabricated savings | O3 |
| 12 | Dynamic tool discovery | Already present; test activation correctness and minimal visible schema | O4 |
| 13 | Context-pressure management | Provider-specific budget, large result and compaction fallback | O5 |
| 14 | Runtime observability | Typed request/tool/continuity/verification records | M2 |
| 15 | Coding-success benchmark suite | Extend existing evals, not another framework | M3–M4 |

## Definition of done

- Each candidate gap has an audit entry: `PRESENT_AND_PROVEN`, `PRESENT_BUT_UNPROVEN`, `MISSING`, or `NOT_WORTH_CHANGE`, with source path, test artifact, and reason.
- Every selected code fix has a failing test reproducing the bug **before** the fix and an unchanged focused green test afterward. Existing green behavior is not rewritten.
- Baseline and candidate use pinned source/model/tool/config identities. All failures (including infra failures) and missing measurements appear in reports.
- No regression in authorization, recovery safety, source-order semantics, active context constraints, unknown state handling or protected provider routes.
- No claim that DaVinci surpasses or equals Claude Code without matched independent task results. Shipping is gated on local tests + explicit authorization for release changes.

## Handoff

Read `AGENTS.md`, the two existing plans named above and the four subplans. Execute **M1 first**, then one independently testable task at a time. Treat all source paths in these files as planning anchors, not a substitute for checking the current default branch. If a test already demonstrates the invariant, record the evidence and close the task with no code change. Do not commit without separate authorization.
