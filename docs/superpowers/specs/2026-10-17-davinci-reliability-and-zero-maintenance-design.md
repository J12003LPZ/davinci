# DaVinci Reliability, Design Studio, and Zero-Maintenance Setup — Design Specification

**Date:** 2026-10-17  
**Status:** Proposed; awaiting review; not implemented or tested  
**Repository:** `J12003LPZ/davinci`, branch `main`  
**Source audit point:** `59d55fb7e145c792ae96271b6f32f219dbf56ab2` (the current observed tip while preparing this document)  
**Basis:** User requirements throughout this conversation, source-level inspection, repository's design-artifacts/readiness/vector-memory documentation, and prior context audits. Prior source-level concerns must be reproduced against the target commit before being classified as confirmed bugs.

## 1. Product outcome and non-negotiable requirements

A user opens DaVinci in a project, accepts or rejects project trust **before project-controlled executable resources load**, and uses ordinary coding commands without operating ancillary services. `/setup` is the sole guided entry point to configure memory and optional add-ons. On later launches, previously authorized local services start and repair safe failures automatically; missing software/models require one clear consented Install/Download action; explicit feature disablement wins over autostart. `/design <natural-language brief>` generates differing UI concepts, opens a confined local browser studio and permits comparison, selection, revision, and separate implementation approval without `--kind` or `--variants`.

### Explicit decisions from user
- Windows is the immediate primary target; retain compatible Linux/macOS paths and fail clearly rather than silently weaken confinement.
- `provider=ollama`, `embedding model=embeddinggemma` (768 dimensions), `memory backend=local`, `autostart=true`, `safe_auto_repair=true` are defaults for the requested setup profile.
- Local memory works without Docker. Qdrant is available only as an **optional, functioning** backend after full implementation and verification; installer must explain benefits and downsides, discover existing Docker services and never force its installation.
- User wants no recurring manual intervention after approved setup or reboot; actionable notifications for unrecoverable problems, no alert storms.
- Strict first-run project trust question: `No — Exit DaVinci`, `Yes, I trust this folder`. No must exit before project-controlled executable components load. Trust does **not** imply unrestricted tool execution approval.
- User wants all audited context-management defects investigated and fixed where reproducible, throughout session start, turns, manual/automatic compaction, branching, resume and shutdown.

## 2. Scope boundaries and architecture choices

### Option choice
1. **Recommended: reuse current local JSONL + Ollama, add typed setup/service supervisor + health diagnostics + isolated design repair.** Fastest safe path; backward-compatible; no compulsory daemon or schema migration.
2. Rewrite local memory onto SQLite now. Improves transactional querying but introduces migration risks and does not repair Ollama, provider health or design generation. **Deferred** until measured durability/performance warrants it.
3. Make Qdrant and Docker mandatory. Adds deployment burden, startup dependencies and no current production retrieval benefit. **Rejected**. Remote is a later, separately gated optional integration.

**Modular boundaries** (proposed interfaces; function names are new unless already in source):
- `SetupProfile` is a *host-owned* persisted profile of selected features, backend, provider, service starts, consent receipts and schema version. Store in global user data outside the project. It references existing `vector-memory.json`, `token-governor.json`, `settings.json` and must display configuration overrides, not write contradictory parallel config.
- `SetupCoordinator` performs deterministic discovery/preflight and renders `SetupAction`s; no installation occurs from mere detection. `/setup check` is guaranteed read-only; interactive `/setup` presents actions and requires explicit consent for installation, model download, privilege expansion and third-party runtime changes.
- `ManagedServiceSupervisor` only starts **explicitly approved, local, allowlisted executables**; it cannot run project-supplied commands, mutate arbitrary containers or auto-install packages. Per-service state machine: `Disabled | Unconfigured | Checking | Starting | Ready | Degraded | ActionRequired | Failed`. Persist configured intents and ownership identity, not transient process IDs as authority.
- `MemoryHealth` is actual persistence + embedding-service + retrieval health, never an approximate Boolean derived from cooldown alone. The local JSONL store remains authoritative; `EmbeddingGemma` vectors are a reconstructable projection, keyed to model identity, dimensions, and prefix revision. Remote Qdrant is a reconstructable optional index, not the only copy of memory.
- `ContextIntegrity` uses verified user origin, scoped authority, source reference, correction/supersession semantics and explicit omissions across the full provider request and folds. Provider-facing role alone is not authority.
- `DesignStudio` reuses session-owned artifact store, revision ledger, constrained browser companion, and current `openai-codex` subscription route. Natural language intent is classified without confusing the brief with command flags. It reuses the existing supervised runtime, not Claude's hosted artifact backend.

## 3. Startup/trust lifecycle and authorization

1. Resolve canonical project location; look up saved explicit trust decisions. If an untrusted interactive project has trust-requiring resources, show **only** `No — Exit DaVinci` (default) and `Yes, I trust this folder`; accepting saves canonical path securely and continues; declining saves denial and exits without project resource loading. Existing global resources may initialize as permitted. Never run project hooks/MCP/config/npm install before approval.
2. Headless/RPC/SDK sessions cannot prompt: deny project-controlled executable resource loading unless a previously stored trust grant or explicit trusted-host override authorizes it; report structured error. Security policy must not be weakened to satisfy a test.
3. With trust established, load user/global and trusted project resources. Trust applies to configuration *source*, not permission to execute arbitrary commands; existing tool permissions/hook approvals and sandbox checks remain active.
4. Service auto-start consent is separate from folder trust: `allow_start_local_ollama`, optional `allow_start_owned_qdrant_container`, `allow_download_embedding_model`, optional `allow_install_software`. Start consent survives restarts but can be revoked; download and install actions show source, version, bytes where available, and request explicit confirmation before first execution.
5. Service supervision should occur **after** trust policy selection and not block project startup for an optional unavailable feature. Explicitly disabled means no probes that start processes, no retries, no surprise re-enablement.

## 4. `/setup` UX and operations

- First run: `/setup` discovers Ollama executable/API and configured model, local memory state, governor store, optional Qdrant readiness, design runtime dependency state, and project instruction/trust status. Render clear `Ready`, `Already installed`, `Missing → Install`, `Model missing → Download`, `Disabled`, `Degraded`, `Cannot repair automatically` states.
- Default selections: local memory ON; Ollama ON; local embeddings ON; governor ON; design studio ON **only after runtime preflight and explicit enable**; Qdrant OFF; `autoStart=true`, `safeAutoRepair=true`.
- `/setup check` is read-only with structured JSON and screen output; `/setup` offers safe actions; `/setup repair` retries explicitly configured reversible actions only; `/setup status` reports live condition; support scoped disable/enable without deleting data. Preserve existing `/setup trust` for compatibility, but route to the same trust store and action semantics.
- After user accepts `Install/Download`, show progress, bounded timeout and cancellation; immediately test version/health and search function. No silent retries of unknown-outcome downloads or irreversible migration.
- Global settings transactionally updated with backup and schema migration, environment override precedence surfaced. Atomic file writes; no plaintext secrets in logs. Configuration corruption is `ActionRequired`, not silently replaced with defaults.
- Service start order on Windows: check approved Ollama path/identity; probe `GET /api/tags` and a test `POST /api/embed`; if offline, start only consented Ollama and poll readiness with deadlines/backoff. On success, lazily re-embed stale records in bounded batches, with status progress. On reboot and subsequent launch, do the same without reprompting; if an unexpected binary version/hash/source appears, ask again.
- Model download is a separately approved action (Ollama pull) after exact model and download details; do not redownload when already compatible. No model-change rebuild may overwrite/delete authoritative local memory.
- Optional Qdrant: probe `GET /readyz`, validate remote identity and collection schema, and use the approved existing container when present. If user selects management of Qdrant on Windows, check Docker Desktop availability and engine status; use an approved **owned container** ID or Compose project and explicit startup consent. Never change third-party containers or Docker Desktop startup settings without approval. `unless-stopped` is only for owned containers with the user's explicit choice; manually stopped services must respect disable intent. Docker engine availability is distinct from a container restart policy.
- Notifications are state transitions with source, remediation and suppression: one incident per failure fingerprint, backoff (e.g. initial 1s/2s/4s, then 120s cooldown for repeated failures), one recovery notice. Health checks cannot repeatedly block active model turns. Global `status` and `/memory-page` surface actual metrics; do not mark memory healthy merely because lexical fallback works.

## 5. Context quality and session lifecycle

**Mandatory audit/reproduction gates:**
- Session entry: project-scoped `AGENTS.md` for target paths, live `AGENTS.md` refresh or invalidation, source trust and plugin SessionStart context reset on `/new`, `/resume`, `/fork` and import.
- During turns: synthetic user-role notices cannot become authoritative user decisions; corrections and excluded/pinned overlay items affect *actual* provider request; provider-visible context diagnostics agree with selected and omitted items. Ephemeral content preserves supported structured attachments in active mode; otherwise explicitly rejects unsupported content rather than quietly dropping it. Context changes invalidate prepared images/cache correctly.
- Manual/automatic compact: reject empty/incomplete summaries, ensure compaction yields material reduction/size fit and preserves original history on failure; budget summary inputs; avoid failed auto-compact silently dispatching oversized requests; keep a recoverable source reference and preserve user corrections, current task, evidence, blockers, active decisions and tool provenance. Never mislabel synthetic/custom context as user instructions.
- Active/hybrid folds: correct supersession, deterministic fallback conflict handling, multimodal source references and retrievable image content; exact retrieval including long single-line pages. Store immutable source reference identity with integrity hash, not source text that can change under references.
- End/resume/branch: commit and surface persistence failures at their operation boundary, restore only selected branch context and current session plugin sources, preserve durable checkpoints and session recovery. Benchmark off/hybrid/active on real tasks; no default mode switch without measured quality, cache and resume parity.

**Context acceptance invariant:** For every provider request, a diagnostic packet (without secrets) can show item identity, origin, authority, provenance, selection/omission reason, freshness, and retrieval path. No internal notice is `UserDecision`; no old false assertion becomes authoritative after a correction; no compaction failure destroys existing history.

## 6. `/design` contract

- Canonical UX: `/design Create a modern dashboard for managing AI agents` (no required kind/variant flags). Model/classifier infers kind; the product defaults to 3 noticeably different concepts when possible. Legacy `/design new ... --kind ... --variants ...` remains supported and overrides classification.
- Generation requires validated selected subscription route/effort, budget, runtime, workspace confinement and authorized session. Open a paired local browser studio after a committed revision: three distinct variant cards/artboards, compare and select, revise in natural language, zoom/viewport, export, comments and source-backed edits. Never auto-apply implementation changes.
- On output validation failure, report exact phase, expected schema, safe sanitization of invalid output, remaining budget and recoverable checkpoint; allow bounded shape normalization, bounded retries, and no infinite looping. The existing one-variant `variants` object normalization is preserved. Do not relax sandbox/browser network/process constraints to 'make it work'.
- Runtime installer must be signed/trusted out-of-workspace source, not project-editable scripts. Windows automated path checks Node/Playwright/Chromium fingerprints; Linux/macOS clearly report required manual steps until managed installers are verified. Design can be disabled without deleting artifacts; `list/status/source export` continue to work.

## 7. Token Governor contract

The token governor is local and independent of Ollama/Qdrant. On startup verify configured output-store path/permissions without destroying existing outputs. A reversible end-to-end test compresses a large synthetic output, persists it, retrieves an exact hidden line and handles restart. `/governor-status` reports `Enabled/Disabled/Degraded`, store path, compression and retrieval counters, last health error. An explicit disabled state prevents compression/retrieval hooks from changing ordinary output behavior; retained artifacts remain recoverable per policy.

## 8. Failure, crash, migration and rollback rules

- Every phase has a code and action: `INSTALL_REQUIRED`, `MODEL_MISSING`, `PROVIDER_OFFLINE`, `EMBEDDING_MISMATCH`, `LOCAL_STORE_UNREADABLE`, `QDRANT_OFFLINE`, `DESIGN_RUNTIME_INVALID`, `UNTRUSTED_PROJECT`, `CONTEXT_ADMISSION_FAILED`, `PERSISTENCE_FAILURE`. Errors and fallbacks are visible; offline/degraded != healthy.
- `Repair` is idempotent, safe and reversible: retry a service that was previously approved, reindex derived embeddings from local records, verify record IDs and projection lags; **never** delete authoritative records, reset trust, install new dependencies, escalate privileges or switch memory backends automatically.
- Support cancellation and app shutdown: stop only DaVinci-owned child processes as configured; do not kill a user's existing Ollama server or Docker Desktop instance. A crash leaves durable preferences and user data safe; supervisor reconciliation on next startup handles stale PIDs and interrupted downloads.
- Migration respects `<project>/.pi/vector-memory` legacy fallback, existing `.davinci/vector-memory/records.jsonl`, existing user config and environment overrides. Never reanimate erased records; backup before changing formats.
- Qdrant unavailable -> local lexical/dense if compatible; label fallback and reconciling projection. User choice to make Qdrant mandatory is not the default and would require an explicit settings action.

## 9. Test and release gates

- **Windows primary:** Windows 11 + Docker Desktop optional + Ollama installed/missing/offline, model present/missing, cold reboot simulation, ten consecutive relaunches, trust first-use No/Yes, service crash and recovery, PATH changes, permission failures, full `/design` capture and browser selection.
- **Linux/macOS:** read-only checks and mode-appropriate installer outcomes; no unrestricted Chromium fallback or namespace security bypass.
- **Context:** 300-turn multi-compact fixture, nested `AGENTS.md`, conflicting corrections, synthetic notices, giant JSON line paging, image-containing turns across fold, branch switch, source hash and crash-resume.
- **Design:** no-flags natural-language entry, 3 distinct concepts, malformed model source repair, accepted variant selection and persistence, strict permission/transport sandbox, source export, native preview, and implementation approval boundary.
- **Memory:** local read/write/reload/search, live `/api/embed` dimension check, stale embedding rebuild, invalid/corrupted JSONL, rollback/repair, Qdrant service outage and namespace collision (only after optional Qdrant implementation).
- **Governor:** exact retrieval after output compression across restart, dedupe invalidation, store unwritable, disabled mode zero semantic behavior changes.
- **Observability:** an unavailable service yields one visible actionable incident, not an unbounded loop or misleading green status. No secrets or absolute user paths in incident telemetry unless user opts in.
- **CI:** `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, focused package tests, existing offline browser tests and pinned strict native confinement tests, plus staged live-provider tests only when account consent and budget permit. Source tests alone do not certify design quality or installed-binary behavior.

## 10. Delivery protocol and quality bar

Each implementing PR starts with a failing regression and root-cause trace, then changes the smallest component, runs the focused suite, records evidence and has an independent review. New features ship behind explicit settings/flags, preserve old data and existing CLI commands, and include a rollback instruction. Program integration ships only after all mandatory gates pass; optional Qdrant or richer installers must never delay or block local memory or coding.

**Out of scope for first release:** replacing local JSONL with SQLite without benchmark/migration evidence, cloud sync of private memories, silently installing Docker, relaxing sandbox confinement, adding unrelated design providers, forcing Qdrant as a dependency, or modifying the user's live DaVinci installation without explicit permission.

## 11. Source anchors

- Repo: https://github.com/J12003LPZ/davinci
- Existing setup: `crates/davinci-coding-agent/src/native_extensions/setup.rs`
- Existing memory: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`, `memory_page.rs`, `docs/vector-memory.md`
- Existing governor: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`
- Existing runtime host: `crates/davinci-coding-agent/src/runtime_host.rs` and `src/native_extensions/mod.rs`
- Existing trust: `crates/davinci-coding-agent/src/trust.rs`, `src/permissions.rs`, `src/main.rs`, `src/davinci_interactive.rs`
- Existing design: `crates/davinci-coding-agent/src/design/{commands,generation,model,setup,runtime,host,controller}.rs`, `design-ui/`, `docs/readiness/design-artifacts.md`
- Existing context: `crates/davinci-agent/src/{context,compaction,pruning,turn,lib,context_usage}.rs`, `runtime/context_vm/`, `runtime/context_overlay.rs`
- Qdrant projection decision: `docs/superpowers/plans/2026-09-15-memory-projection-decision.md`
- External operational references: https://docs.docker.com/engine/containers/start-containers-automatically/ ; https://docs.docker.com/desktop/settings-and-maintenance/settings/ ; https://api.qdrant.tech/api-reference/service/readyz
