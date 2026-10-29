# Context, Compaction, Cache, Skills and Tool-Discovery Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task. All steps use checkboxes.

**Spec:** `docs/superpowers/specs/2026-10-28-claude-runtime-capability-convergence-design.md`. **Master:** `docs/superpowers/plans/2026-10-28-claude-runtime-gap-closure-master.md`.
**Global constraints:** Rust 1.88/Cargo locked; owner approval before implementation and commits; preserve dirty working tree and openai-codex model/effort/OAuth, effect latch, permissions and user constraints; use deterministic offline tests. File paths are repo-relative, review at execution time. A plan item is not a completed change.

**Review focus:** stale verification after edits; restarted tasks losing receipts; permission revocation mid-operation; error after side-effect commit; unrelated tests passing falsely. Every corresponding task must test relevant cases.

**Goal:** Achieve Claude-style lazy capability exposure and context management while retaining Context VM's durable source-grounding and Codex route constraints. **Architecture:** enhance existing `ToolSurface`, prompt/cache builders, skill managers, and fold/checkpoint paths; never create a second context engine. **Tech Stack:** Rust agent/AI/extension crates and offline mock traffic.

### Task C1 — Constraint-preserving default compaction (G-04, X-09)
**Files:** `crates/davinci-agent/src/{compaction.rs,turn.rs,lib.rs,runtime/context_vm/{fold.rs,reducer.rs}}`, session history and existing `context_vm_hybrid` tests. **Interface:** current `ContextVmMode::{Off,Hybrid,Active,...}`, compaction snapshot, complete tool pair, user requirement authority.
- [ ] RED: five repeated compactions preserve early user restrictions, corrections, referenced artifacts and latest unverified changes; malformed summary, dropped instruction, overlap with new user turn, crash before commit, token-count mismatch, provider overflow remain recoverable.
- [ ] Extend Off summarizer with deterministic user-constraint inclusion/rejection or allow a tested opt-in Hybrid default. Never claim Claude's unobserved summary algorithm as known: report 35 did **not** trigger compaction. Preserve original history when summary fails validation and keep session append-before-swap guarantee.
- [ ] GREEN `cargo test --locked -p davinci-agent --lib compaction`, `context_vm_integrity`, `context_vm_hybrid`, interrupted-session tests. Measure tokens and extra compaction model calls separately.

### Task C2 — Deferred tool search and stable prefix (G-08, X-03)
**Files:** `crates/davinci-agent/src/{lib.rs,tools.rs,prompt/*}`, `crates/davinci-ai/src/{responses_tools.rs,stream.rs}`, `crates/davinci-coding-agent/src/extension_host.rs`, existing `lean_tool_surface` tests. **Interface:** existing `ToolSurface::Full/Lean`, capability registry, provider request surface and tool-use/result pair.
- [ ] RED: 160 fake MCP tools do not flood initial schemas in opt-in deferred mode; new schema available only after authorized discovery; stale name after catalog update refuses; denied tool never surfaces; cached stable prefix remains byte-identical; control/wire pairings survive compaction and resume.
- [ ] Implement exact-name and keyword discovery reusing current capability registry; append selected schema in current turn's permitted channel if *Codex route supports it*, otherwise expose provider-compatible stable tooling and clearly report the limitation. Never assume Anthropic `ToolSearch` behavior exists on OpenAI Codex.
- [ ] Measure initial request bytes, schema count, extra discovery round trips, cached/uncached tokens when reported, task outcome; preserve Full opt-out. Only propose Lean default if representative offline and authorized live results are no worse.
- [ ] GREEN `cargo test --locked -p davinci-agent --lib lean_tool_surface`, `cargo test --locked -p davinci-ai --lib`; tool-dispatch permission tests.

### Task C3 — Lazy SKILL.md and safe scope-aware activation (G-17, X-04)
**Files:** `crates/davinci-coding-agent/src/{skills.rs,extension_host.rs,native_extensions/learning/retrieval.rs}`, existing invocation paths/tests. **Interface:** lightweight metadata: skill identity, scope, file path/digest, description; load selected full body only on authorized invocation.
- [ ] RED: 100 installed skills result in metadata reads but zero unused body reads during startup; invoking one skill loads one body; changing skill file invalidates cached body; scoped duplicate resolution deterministic; symlink escaped path denied; revoking plugin removes its tools/skills.
- [ ] Reuse existing skill and extension registries, avoid loading all `SKILL.md` bodies to parse metadata; use metadata frontmatter reading where needed, bounded by bytes; do not weaken authority or trust constraints. Preserve inline and worker invocation behavior when supported.
- [ ] GREEN skills and extension-host tests; compare warm/cold startup I/O and retained RSS; no speculative model calls.

### Task C4 — Cache/protocol efficiency and measurement (X-10)
**Files:** existing `crates/davinci-ai/src/{stream.rs,stream_decoder.rs}`, `crates/davinci-agent/src/cache_stats.rs`, `scripts/bench/{campaign.py,README.md}` only if tests expose a gap.
- [ ] RED: dynamically added tool, MCP refresh, compaction, changed AGENTS.md, model/effort change each produce correct generation/cache identity; missing usage remains unknown; cached read is not double-subtracted; no stale schema rehydration.
- [ ] Compare Off/Hybrid/Active plus Full/Lean with identical synthetic tasks, provider request shape, token count and compaction reachability; record cache prefix byte identity and request count without equating with billed hits.
- [ ] Preserve `prompt_cache_key` and provider capabilities; do not introduce hidden API fields to Codex. Real quota comparisons require separately approved paired live runs with known usage fields and fixed task success oracles.
- [ ] GREEN existing cache stability, provider request, and compaction tests; promote default only after an explicit measured no-regression gate.

**Acceptance:** robust default compaction retains requirements; skills are actually read lazily; deferred tools remain authorized and provider-compatible; no false quota or task-success claims; existing Context VM recoverability passes fault injection.
