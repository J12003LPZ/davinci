# Davinci Ecosystem Integration 10/10 Roadmap

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this roadmap task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn Davinci's Graph Engineering, Token Governor, Vector Memory, prompt caching, pruning, self-learning, security scanning, verification, and observability into one closed, token-efficient ecosystem without adding an always-on coordination model.

**Architecture:** Keep the harness AI-thin. Deterministic code owns hard bounds, provenance, capabilities, verification, cache identity, and conditional gates; models keep investigation, planning, implementation, and review. Integration is implemented through small immutable contracts rather than a central intelligent orchestrator.

**Tech Stack:** Rust 1.83.0, existing Davinci crates/native extensions, JSONL session/run stores, provider prompt-cache fields, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`

## Global Constraints

- Add zero always-on orchestration model calls to normal turns.
- Add zero new director/orchestrator model calls to graph runs.
- Graph ecosystem context is capped at 2,500 tokens per worker by default.
- Graph skill context is capped at two full skill bodies / 1,000 tokens by default.
- Graph memory context is capped at four hits / 1,200 tokens by default.
- Governor recovery remains on-demand and lossless.
- Security verification is risk-triggered by default.
- Learning remains asynchronous and fail-open for normal foreground turns.
- Observability remains structured runtime data and is not injected into prompts by default.
- Preserve `--no-session --no-extensions --no-skills` graph-worker isolation.
- Preserve repository-pinned Rust 1.83.0 and exact dependency convention.
- Tests remain fixture-only and must not require provider/network access.
- Functional integration and dead-code cleanup are separate commits/phases.

---

## Execution Program

### Gate A — Graph correctness before ecosystem coupling

**Plan:** `docs/superpowers/plans/2026-09-04-ecosystem-a-graph-hardening.md`

Deliver a graph runtime whose contracts, topology, replay, deadlines, mutation provenance, and review coverage are trustworthy enough to become the backbone of the ecosystem.

Exit criteria:

- [ ] schema and runtime validation cannot drift silently;
- [ ] evidence findings cannot carry empty provenance;
- [ ] integer budgets reject fractional/overflow values;
- [ ] successful tasks cannot retain stale failed-attempt errors;
- [ ] run deadlines actively terminate workers;
- [ ] graph topology and transitions are explicit persisted data;
- [ ] replay requires compatible inputs/config/repository state;
- [ ] review sees only graph-owned mutation and covers every changed chunk;
- [ ] all hardening tests are green.

**Release gate:** Do not begin Gate B on a branch with failing Gate A invariants.

### Gate B — Runtime/context integration

**Plan:** `docs/superpowers/plans/2026-09-04-ecosystem-b-runtime-integration.md`

Close the Graph ↔ Governor ↔ Memory ↔ Skills ↔ Cache loop while preserving worker isolation and hard token caps.

Exit criteria:

- [ ] every graph role that can produce governor-compressed output automatically gets `retrieve_output`;
- [ ] `StreamOptions` supports cache affinity independent of persisted sessions;
- [ ] compatible ephemeral graph workers derive stable cache keys;
- [ ] graph context packets are bounded and role-scoped;
- [ ] no weak memory/skill match is injected merely to fill a quota;
- [ ] graph workers remain `--no-session --no-extensions --no-skills`;
- [ ] selected skill versions and memory provenance are recorded;
- [ ] resource snapshots expose cost/token/cache/governor/pruning facts without an extra model call;
- [ ] offline integration tests prove governor recovery and cache-key stability.

**Release gate:** Compare graph fixture runs before/after. Integration must not introduce an extra model call and must keep injected ecosystem context under the configured cap.

### Gate C — Learning feedback + conditional security

**Plan:** `docs/superpowers/plans/2026-09-04-ecosystem-c-learning-security.md`

Close the return path: verified graph work improves future graph work, and high-risk changes automatically receive the native security subsystem as a deterministic gate.

Exit criteria:

- [ ] one `VerificationBundle` represents deterministic and security verification evidence;
- [ ] graph task metadata records which skill versions were injected;
- [ ] verified graph outcomes update those skill versions' success/failure metrics;
- [ ] memory and learned-skill artifacts from one run can be retrieved by a later graph run;
- [ ] low-signal read-only turns skip the model-backed learning reviewer while still indexing memory;
- [ ] learning-token measurements show at least 40% lower median review input on the fixture corpus with no more than 5% relative loss of accepted high-confidence artifacts;
- [ ] deterministic diff/path risk classification triggers security only when warranted by default;
- [ ] required security failures prevent graph approval;
- [ ] a full-circle offline fixture proves graph #1 -> learn -> graph #2 -> verified reuse.

**Release gate:** Learning remains fail-open for foreground execution; security behavior follows explicit `off | risk | always` policy.

### Gate D — Proof, CI, observability, hygiene

**Plan:** `docs/superpowers/plans/2026-09-04-ecosystem-d-proof-ci-hygiene.md`

Make ecosystem claims measurable and continuously enforced.

Exit criteria:

- [ ] structured telemetry can explain memory, skills, governor, cache, pruning, graph, security, and learning participation for a run;
- [ ] `/status` and `/graph-status` expose compact integration facts without prompt injection;
- [ ] closed-loop integration tests run offline;
- [ ] GitHub Actions runs fmt, clippy, workspace tests, and named ecosystem tests;
- [ ] known production-looking dead source islands are proven unreferenced before removal/archive;
- [ ] documentation describes the actual final data flow and token/model-call invariants;
- [ ] `make build`, `make test`, `make fmt`, and `make clippy` all pass.

---

## Program-Level Acceptance Metrics

Track these before Gate B and after Gate D using deterministic fixture runs.

| Metric | Requirement |
| --- | --- |
| Extra orchestration model calls, normal turn | `0` |
| Extra orchestration model calls, graph run | `0` |
| Default graph ecosystem context | `<= 2,500 tokens / worker` |
| Default full skills injected | `<= 2 / worker` |
| Default graph memory hits | `<= 4 / worker` |
| Governor compressed output recoverability | `100%` for allowed worker tools |
| Required security failure bypasses | `0` |
| Graph-owned changed chunks omitted from review | `0` |
| Cache affinity key stable across compatible retry | `100%` |
| Cache affinity key changes on contract/toolset/model change | `100%` |
| Median learning-review input token reduction | `>= 40%` on fixture corpus |
| Relative loss of accepted high-confidence learning artifacts | `<= 5%` |
| CI required checks | fmt + clippy + workspace + ecosystem |
| Network-dependent tests | `0` |

## Architectural Guardrails During Implementation

### Do not build a god object

Reject a proposed `EcosystemController` that owns Graph, Governor, Memory, Learning, Security, caching, and UI. New code should be small contracts/helpers under `native_extensions/ecosystem/`, consumed by the existing owners.

### Do not over-prompt the model

A subsystem being integrated does not mean it gets an instruction paragraph in every system prompt. Prefer:

- one short governor-recovery sentence only when the tool exists;
- compact context sections with provenance;
- no telemetry dumps in prompts;
- no security explanation unless the security result is relevant to the review;
- no skill body unless retrieval says it is relevant and it fits the cap.

### Do not replace model judgment with brittle heuristics

Deterministic code may gate unsafe/impossible behavior. It must not attempt to micromanage how the model investigates or writes code.

Bad examples:

- a rules engine deciding which exact files a researcher must read;
- cache-hit percentage choosing the writer's implementation strategy;
- memory confidence dynamically rewriting a model's plan;
- an orchestration model reviewing another orchestration model.

Good examples:

- cap three parallel workers;
- stop at a deadline;
- provide two relevant learned skills and let the worker decide whether to use them;
- require a security check for authentication code;
- refuse approval when tests failed.

## Commit/Review Strategy

Every task in the four execution plans ends with an independently reviewable commit. Do not make one ecosystem mega-commit.

Recommended branch sequence:

```text
feature/ecosystem-a-graph-hardening
feature/ecosystem-b-runtime-integration
feature/ecosystem-c-learning-security
feature/ecosystem-d-proof-ci-hygiene
```

Each branch starts from the merged predecessor. Gate D is the first point at which the project should claim the full 10/10 ecosystem integration target.

## Rollback Strategy

- Gate A changes are correctness fixes and should be individually revertible by commit.
- Gate B context packet injection must have a configuration kill switch that restores current graph behavior without disabling memory for normal turns.
- Cache affinity must fall back to existing `session_id` behavior when `cache_key` is absent.
- Gate C security policy supports `off` to restore pre-gate graph behavior.
- Learning evidence gating may be disabled independently if fixture quality regresses.
- Telemetry/CI changes never alter agent behavior and can be reverted independently.

## Final Definition of Done

The program is complete only when an offline test can demonstrate this sequence without special manual wiring:

```text
Graph Run 1
  -> bounded memory/skill context
  -> governed tool output with lossless recovery
  -> deterministic verification
  -> risk-triggered security when applicable
  -> review
  -> learning persistence
  -> memory/skill provenance

Graph Run 2
  -> retrieves relevant persisted learning under hard token caps
  -> stable provider cache affinity without a saved session
  -> uses verified learned context
  -> verifies successfully
  -> updates the exact used skill versions' outcome ledger
```

At that point the features are no longer neighboring mechanisms; they are a measurable closed ecosystem.