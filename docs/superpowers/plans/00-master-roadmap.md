# DaVinci — Superpowers Master Implementation Roadmap

**Prepared:** 2026-10-17  
**Status:** Proposed plan only; no user installation, source edit, PR, build or live test has been performed.  
**Repository:** https://github.com/J12003LPZ/davinci  
**Audit reference:** `main` at observed tip `59d55fb7e145c792ae96271b6f32f219dbf56ab2`. Reconfirm tip and regressions before each task; compaction was modified recently.

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` or `superpowers:executing-plans` after the written spec and plan are reviewed. Execute one independently testable task with a failing test, a focused implementation, verification evidence and a commit; do not launch all streams in an unreviewed change.

## Goal

Make DaVinci feel fully automatic after one approved setup: strict two-option trust on first use, healthy local memory + Ollama and governor across Windows restart, an intuitive `/design <brief>` browser artifact studio, optional **working** Qdrant, and context integrity across complete session lifecycle. Restore correctly, report errors honestly, do not lose project data, and keep advanced flags optional.

## User-experience contract

```text
Open DaVinci project
  └─ if necessary: No — Exit  |  Yes, I trust this folder
       └─ load trusted project resources
            └─ reconcile saved setup profile
                 ├─ Ollama healthy? reuse; else approved autostart
                 ├─ embeddinggemma present? reuse; else Download confirmation
                 ├─ local records readable? index/retrieve; else warn + preserve bytes
                 ├─ governor store healthy? verify exact retrieval; else warn
                 ├─ Qdrant selected? probe/optional approved start; else never touch
                 └─ /design requested? infer intent → generate 3 concepts
                                      → browser studio → select/revise
                                      → separate implementation approval
```

On later launches, approved services start without repeat questions. `Disabled` is respected. No Docker or Qdrant is required for local memory. No model/installer/process executes from untrusted project-controlled resources. No model request may silently erase or misattribute context.

## Required decisions (already supplied by the user)

- Defaults: Ollama, EmbeddingGemma, local JSONL memory, autoStart=true, safeAutoRepair=true, Token Governor enabled.
- Optional choice: Qdrant, explained inside `/setup`, with detection and opt-in management; no mandatory Docker. SQLite migration deferred until benchmarked.
- `/design` is natural-language first; no required `--kind` or `--variants`, with 3 visually varied concepts and browser comparison.
- Trust first-run: exactly No/Yes in interactive startup; No exits; Yes is stored. Existing tool-specific permission gates still apply.
- Any action installing binaries, downloading models, escalating privileges, changing OS startup, or creating/managing containers requires explicit one-time approval for the identified scope; safe approved starts/repairs can be automatic afterward.

## Delivery order and dependencies

| Gate | Workstream and plan | Scope | Ship independently when |
|---|---|---|---|
| **0** | [Design specification](../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md) | Confirm desired behavior, data and permissions contract | User reviews, implementation baseline fixed |
| **1** | [01 — Strict project trust](01-strict-project-trust.md) | Before-resource startup gate, denial, host consent separation | No project code executes on untrusted startup |
| **2** | [02 — Setup, local memory, Ollama, governor](02-automatic-local-memory-and-governor.md) | Health, discovery, first-run Install, startup/restart supervisor, durable memory | Ten relaunches require no manual action, outage visible, records intact |
| **3** | [03 — Context integrity/lifecycle](03-context-integrity-lifecycle.md) | Origin, overlays, AGENTS rules, image recall, budgets, compaction, session persistence | Authority/source continuity proven across folds/restarts |
| **4** | [04 — Natural-language Design Studio](04-natural-language-design-studio.md) | `/design <brief>`, 3 variants, browser, errors, selection, review | Actual browser end-to-end verified, not fixture-only |
| **5** | [05 — Optional Qdrant integration](05-optional-qdrant-backend.md) | Remote queries + Docker discovery and ownership + fallback | Works and is measured; absence cannot block default profile |

Workstreams 03 and 04 may be researched in parallel after their permission/interface dependencies are agreed. 05 should not land in the default setup workflow until local memory and trust gates have passed. Do not convert all 40+ tasks into a single huge PR.

## Release train (practical grouping)

**Release A — trust & diagnostics:** trust startup gate, setup/profile read-only status, honest store/provider health. No new installation automation as part of this release.

**Release B — zero-maintenance local memory:** consented Ollama startup/model pull, repair/restart, governor verification, setup wizard and feature opt-outs. Qdrant remains off.

**Release C — context integrity:** compaction, provenance, scoped AGENTS and session/branch recovery fixes validated on current main; behind mode-safe gates where appropriate.

**Release D — Design Studio:** no-flags design, debug receipts, real browser preview, selection, revisions, no auto-apply; retain strict confinement.

**Release E — optional Qdrant:** full remote filtering/index/retrieval, Docker manager consent, fallback, repair, benchmark; no default-enable without measured gate.

## Global implementation instructions

1. Set up an isolated git worktree using `superpowers:using-git-worktrees`; pin the test candidate. Inspect `AGENTS.md`, repo policy, existing CI scripts and model-runtime constraints. Do not modify the user's installed binary, host Docker/Ollama or real credentials during the plan drafting stage.
2. For each bug claim, use `superpowers:systematic-debugging`: reproduce; capture exact error/effective configuration; trace root cause; write a failing test; implement only the measured fix. Some previously identified bugs may be fixed on current main.
3. For every new feature use `superpowers:test-driven-development`; small commits; `superpowers:verification-before-completion` before any success claim. Do not claim a live integration pass from an offline fixture.
4. Prefer a reviewer per task and a whole-branch audit. Preserve provider auth/subscription route, permission mode, sandbox, session journal, memory durability, cross-platform behavior and feature-off parity.
5. Do not silently escalate authority or call downloads/untrusted scripts. Backups, durable disablement, undo/rollback and transparent effective config are release blockers.

## Shared acceptance matrix

| Scenario | Expected outcome | Owner |
|---|---|---|
| New untrusted project, choose No | Exit without project hooks/extensions/MCP; decision remembered | 01 |
| New trusted project, choose Yes | Trust saved, load resources, tool asks remain | 01 |
| Fresh Windows boot, Ollama stopped | Approved auto-start; real embeddings/search healthy or clearly degraded | 02 |
| Model missing | Show Download, one Enter confirmation, progress, verification | 02 |
| Corrupt `.davinci/vector-memory/records.jsonl` | No overwrite/data loss; repair path and error visible | 02 |
| User disabled memory | No startup/probes that launch daemon, no repeated alerts | 02 |
| Governor compresses 900-line output | Exact hidden middle line recovered after restart | 02 |
| 300-turn session with user correction | Latest authorized requirement survives compact/restart/branch | 03 |
| Synthetic capability reminder | Never becomes authoritative user request in fold | 03 |
| Old screenshot across fold | Original visual evidence remains recoverable or is explicitly unavailable | 03 |
| `/design Create a dashboard` | Product intent inferred, three distinct browser concepts, no flags | 04 |
| Broken design runtime or malformed model JSON | Exact actionable error; prior revision never lost | 04 |
| Select Design B and revise | Selection/revision survives restart, no target edit until separate approval | 04 |
| Qdrant absent/unhealthy and not selected | No Docker activity, normal local search works | 05 |
| Qdrant selected but down | Approved owned start attempt, then visible local fallback, no record loss | 05 |

## Verification matrix

- Rust offline: targeted `cargo test -p davinci-agent context_`, `cargo test -p davinci-coding-agent setup_`, `cargo test -p davinci-coding-agent design_`, `cargo test -p davinci-coding-agent qdrant_` (adapt exact filters to registered test names), then `cargo test --workspace` where feasible.
- Static: `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets -- -D warnings` on pinned toolchain. Preserve existing script/CI exclusions only when documented.
- Windows native: ConPTY trust/setup menus, approved Ollama process startup/stop and cold relaunch, non-admin install behavior, Docker Desktop process and engine separation, AppContainer browser capture/interaction and hostile-page network/process denial.
- Other OS: Linux/macOS feature-off behavior and strict native sandbox; no silent unsupported installer.
- Provider: use fake Ollama/Qdrant and recorded Codex fixtures for most tests; run authorized live account checks separately, with budget, fingerprint, screenshots and evidence retention.
- Performance: compare 300-turn context success and cache overhead, setup startup impact, retrieval p95/quality. No regression/success marketing claim without measured results.

## Risk register and mitigation

- **Startup deadlock or surprise execution:** read-only probes, explicit consent, off-thread supervisor, fixed deadlines, strict trust gate.
- **Memory corruption/migration:** local JSONL remains authority, atomic writes, backup before format changes, tested tombstones/legacy fallback; Qdrant rebuildable.
- **Compaction regression:** fail-closed commit validation, maintain original history, source recovery, origin-preserving serialization, 300-turn tests.
- **Design renderer escape:** continue verified pinned runtime, strict browser confinement, one-time pair tokens and host-side authority. Never relax protection to improve success rate.
- **Provider rate-limit/shape failures:** bounded attempts, checkpoint every stage, surfaced exact errors, no unbounded retrial or unauthorized model switch.
- **Project trust/permission bypass:** source trust != execution grant; tests for preloading order, override tampering and stored explicit denials.
- **Docker interference:** only user-approved owned container; never change foreign containers or OS startup unprompted; fallback to local.

## Honest limitations

This is an implementation **proposal**, not evidence the bugs reproduce in the current installed build. The repo's design-artifacts readiness file explicitly reports incomplete live certification; the Qdrant projection decision explicitly chose local as production. Current environment has not accessed the user's Windows installation or Docker/Ollama processes. A reviewer should approve the accompanying spec and implementation plan before product-code changes. The plan files are staged as downloadable Markdown, **not committed to GitHub**.


## Extension — Screenshot-style live agent navigation (Phase 6)

**Added after the original five workstreams:** [06 — Claude-style agent orchestration and UI parity](06-claude-style-agent-orchestration-ui-parity.md); [dedicated design specification](../specs/2026-10-17-claude-style-agent-navigation-design.md).

The user additionally requires a persistent bottom main/worker roster, live token/time/activity visibility, real Git branch/worktree inspection, clickable per-agent transcript focus, Esc/click-to-main without cancelling work, queued main and worker instruction receipts, and crash-safe late-message handling. **14 additional reviewable tasks** span observability, terminal UX, queue durability, RPC/SDK consistency and Windows/Linux interactive evidence.

**Dependencies:** Phase 6 should run only after Phase 1 authority policy and Phase 3 origin/journal interfaces are stabilized; it can reuse early runtime telemetry tests while other phases proceed. Do not combine all six workstreams into one PR; Phase 6 has its own feature flag and release gates. The old 43-task count in earlier packet metadata refers to Phases 1–5 only.
