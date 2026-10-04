# Davinci Design Artifacts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` when authorized delegation is available, or `superpowers:executing-plans` for inline execution. Execute task-by-task with red/green checks and a review gate. Checkboxes track future implementation; none are claimed complete here.

**Goal:** Add a persistent, code-backed `/design` workspace with Taste-informed generation, bounded visual editing, evidence, and explicit implementation handoff.

**Architecture:** A `design/` module in the existing Rust coding-agent owns the product workflow. A supervised trusted browser companion displays inert rendered artboards, while generated code runs only in the existing managed-browser boundary. Existing runtime, session, permission, subscription admission, artifact, and transaction owners remain authoritative.

**Tech Stack:** Rust 1.83.0; existing serde, SHA-256, session and evidence utilities; a bundled React/TypeScript workspace with CSS tokens; trusted Node HTTP/stdio bridge; exact-pinned esbuild and existing trusted Playwright/Chromium. No hosted service or new provider SDK.

**Spec:** `docs/superpowers/specs/2026-10-02-davinci-design-artifacts-design.md`.

**Status:** Proposed plan for review, not executed. No repository, installed executable, credential, or account was changed. No live model requests or test runs were performed.

**Baseline:** Inspected `main` = `f24ed1c744a31ee49c95802f982740fe59d2de6d`. PR #82 is open on `dd6725f758a3619892349c59664f1d3f1a5e53fd`, stacked on PR #81. Reconcile against the actual approved execution branch; do not assume these changes are on main. The proposed file map must be checked against that branch before editing.

## Global Constraints

- Core Rust remains on 1.83.0; new dependencies use exact pins and a committed lockfile.
- All model requests use the authorized OpenAI/Codex subscription route; API-key routes and billing/model fallback are denied.
- The selected model and effort are inherited from the authorized session; capability support is verified, not inferred from a model name.
- Existing session events remain authoritative; artifact indexes, render caches, and UI state are derived.
- Generated source never executes in the privileged workspace browser origin or in the Node host process.
- No package installation, remote font fetch, asset download, or project build script runs implicitly during /design.
- New model-facing operations pass existing permission, task-contract, cancellation, and root-budget checks.
- Persistent revisions and source blobs are immutable; every mutation supplies an expected revision and idempotency key.
- Browser evidence is valid only for its exact source, viewport, runtime, and policy fingerprints.
- Missing browser, image capability, dependencies, or evidence produces an explicit incomplete state, never a fabricated pass.
- Ordinary sessions retain their previous tools and prompt projection when the design feature is disabled.
- New serialized counters and sizes use fixed-width integer types, not usize.

## Review Focus

1. A hostile page or local website must not obtain a workspace capability or use preview traffic to reach another origin; Tasks 5, 6, and 7 own these tests.
2. A human edit and a late model result based on the same revision must not overwrite each other; Tasks 3, 10, and 12 own these tests.
3. A restart after blob publication or browser shutdown must retain source and authorized evidence without reviving transient leases; Tasks 3, 7, and 15 own these tests.
4. Missing browser/fonts/images/vision capability, truncated checks, or stale source must never become a visual or implementation pass; Tasks 7, 8, 12, and 14 own these tests.
5. Unknown subscription usage, interrupted requests, and resumed child operations must not reset root limits or trigger API fallback; Tasks 1, 4, and 12 own these tests.

---

## Execution method and verification discipline

This is one feature divided into independently reviewable changes. Contracts and persistence come first. Rendering and the workspace can proceed independently only after shared contracts and fixtures stabilize. Do not parallelize modifications to the same session/runtime owner. Use authorized existing worktrees; preserve unrelated changes. No autonomous merge, deploy, or live acceptance campaign.

Use the repository's RTK/Headroom conventions where available. Commands below show underlying invocations; route shell execution through the project's approved wrapper without changing their arguments. Offline Cargo commands require already cached dependencies. A missing cache or browser installation is a setup blocker, not a passing test.

For each task: create the named regression tests, observe the intended failure, implement the smallest owner change, rerun the tests, inspect the diff, and commit only the task's paths when implementation and commits have been authorized. Never use `git add .`. Commit titles below are future checkpoints, not actions already taken.

## File map

Paths marked **existing** were located in the inspected repository. Paths marked **new** are proposed. A task may touch an existing integration file only to wire its focused module; no unrelated refactor.

| Path | Status | Responsibility |
| --- | --- | --- |
| `crates/davinci-coding-agent/src/design/mod.rs` | New | Module exports and host-facing facade |
| `.../design/types.rs`, `.../design/error.rs` | New | Domain contracts and typed failures |
| `.../design/store.rs`, `.../design/events.rs` | New | Immutable revisions, session events, derived index |
| `.../design/admission.rs`, `.../design/controller.rs` | New | Existing-runtime authorization adapter and product state transitions |
| `.../design/commands.rs` | New | Shared command parser |
| `.../design/skills.rs`, `.../design/context.rs` | New | Pinned curated profiles and bounded context |
| `.../design/host.rs`, `.../design/compile.rs` | New | Supervised workspace host and virtual-file compilation |
| `.../design/render.rs`, `.../design/quality.rs` | New | Managed capture, durable evidence, quality state |
| `.../design/edits.rs`, `.../design/comments.rs` | New | Typed source edits and revision-specific comments |
| `.../design/sync.rs`, `.../design/handoff.rs`, `.../design/export.rs` | New | Static repository extraction, explicit coding handoff, exports |
| `crates/davinci-coding-agent/src/native_extensions/design.rs` | New | Narrow model-facing tool adapter |
| `crates/davinci-coding-agent/src/davinci_interactive/design.rs` | New | TUI command/results adapter |
| `crates/davinci-coding-agent/src/{main.rs,lib.rs,args.rs,startup.rs,rpc.rs,sdk.rs,davinci_interactive.rs}` | Existing | Entry-point wiring; actual module declarations checked in Task 1 |
| `crates/davinci-coding-agent/src/native_extensions/{mod.rs,browser.rs}` | Existing | Conditional tool registration and host-only browser capture integration |
| `crates/davinci-coding-agent/src/interaction_testing/browser_process.rs` | Existing | Add typed, bounded host-only capture/headed-mode messages |
| `crates/davinci-agent/src/runtime/{evidence.rs,evidence_store.rs}` | Existing | Reuse evidence contracts/storage; change only if a narrowly justified API is missing |
| `crates/davinci-agent/src/skills.rs` | Existing | Reuse discovery; no generic loading behavior change |
| `crates/davinci-tui/src/davinci/views/design.rs` | New | Artifact list/progress/evidence view, not a terminal canvas |
| `crates/davinci-coding-agent/design-ui/` | New | Browser workspace, trusted host/compile scripts, generated contracts, tests, built assets |
| `crates/davinci-coding-agent/design-resources/` | New | Pinned skill source/license/profiles/provenance and preview runtime manifest |
| `crates/davinci-coding-agent/tests/design_*.rs` | New | Deterministic domain, integration, and recovery tests |
| `crates/davinci-evals/tests/design_quality.rs` | New | Fixture completeness and paired-evaluation accounting |
| `docs/design-artifacts.md` | New | User/operator instructions and explicit limitations |

In rows abbreviated `.../design`, the full prefix is `crates/davinci-coding-agent/src/design`.

## Shared contracts

Task 2 owns these types. Later tasks consume them rather than inventing variants.

| Contract | Required shape / meaning |
| --- | --- |
| `AuthorizedDesignContext` | Non-serializable host-minted owner/session/workspace/task identity plus cancellation and current-policy access. Never accepted from browser JSON. |
| `CreateDesign` | Brief reference, kind, title, operation ID, variant count. Owner fields come from the context. |
| `RevisionWrite` | Artifact ID, expected revision, operation ID, payload digest, source bundle, bindings, asset/profile/system references. |
| `DesignEdit` | Artifact ID, expected revision, node ID, expected binding hash, operation ID, typed new value. |
| `RenderRequest` | Artifact/revision/artboard IDs, viewport, theme, fixture state, motion preference. No URL or executable path. |
| `RenderReceipt` | Exact source/runtime/policy fingerprints, capture/geometry artifact refs, bounded check results and coverage. |
| `QualityReport` | Separate source/render/interaction/accessibility/visual/assets dimensions with evidence references. |
| `ProfileSelection` | Kind, named profile, exact source refs/hashes, selected complete sections, dial values, explicit overrides. |
| `DesignSystemSnapshot` | Source fingerprint, observed facts with path/hash/range, confidence, unresolved facts, excluded paths. |
| `HandoffRequest` | Accepted artifact/revision/source/evidence references, target workspace fingerprint, explicit user action. |
| `ExportRequest` | Artifact/revision, format, host-approved destination handle, overwrite approval. |
| `DesignError` | `Denied`, `Conflict`, `NotFound`, `InvalidInput`, `UnsupportedSchema`, `BudgetExceeded`, `MissingCapability`, `StaleSource`, `Cancelled`, `CorruptArtifact`, `IncompleteEvidence`, `IoFailure`. |

All cross-process messages have protocol version, request ID, typed operation, and bounded payload. Opaque IDs are UUIDs; `u64` revisions use decimal strings in browser JSON. An object hash never grants access.

## Task dependency map

```text
1 baseline/integration contracts
  -> 2 types/fixtures -> 3 persistence
                     -> 4 admission/command surfaces
                     -> 5 trusted host
                     -> 6 virtual compiler
3 + 5 + 6 -> 7 confined render and durable evidence -> 8 quality reports
3 + 5 + 7 -> 9 workspace -> 10 edits/comments
2 + 3 -> 11 profiles and read-only design-system sync
4 + 7 + 8 + 11 -> 12 generation/repair controller
3 + 8 + 10 + 12 -> 13 export
4 + 8 + 11 + 12 -> 14 coding handoff
all -> 15 crash/permission/resource integration -> 16 evaluation/release
```

## Implementation tasks

### Task 1: Freeze the integration baseline and dependency boundary

**Files:** Create `docs/design-artifacts-baseline.md`, `crates/davinci-coding-agent/design-resources/toolchain-manifest.json`, and `crates/davinci-coding-agent/tests/design_baseline.rs`. Inspect the existing owners listed above, the actual session custom-entry API, and the approved root-admission implementation from the pending harness work.

**Interfaces:** Produces a recorded integration map: exact session event append/replay methods, permission/admission methods, cancellation propagation, module declarations in binary/library targets, current resource resolver, and trusted Node/browser paths. This task introduces no substitute implementations for those owners.

- [ ] Record `git rev-parse HEAD`, `git status --short`, and the relationship to the inspected main and PR #82. Preserve unrelated work. Use a worktree only after implementation authorization.
- [ ] Add baseline regression tests named `design_disabled_keeps_tools_unchanged`, `design_disabled_keeps_prompt_projection_unchanged`, and `design_requires_no_node_when_disabled`. Snapshot existing behavior before new modules are wired.
- [ ] Run `cargo test -p davinci-coding-agent --test design_baseline --offline --locked`. These characterization tests should PASS on the baseline; subsequent tasks must preserve them.
- [ ] Resolve and review exact versions for React, TypeScript, esbuild, and the existing trusted Playwright runtime. Record package checksums, Node patch, compiler binary/platform hashes, build commands, and licenses in the manifest. No `latest` specifiers or runtime download path. Dependency installation is a separate explicit setup action.
- [ ] Confirm the approved branch supplies durable subscription root admission. If it does not, mark Task 12's live dispatch blocked and continue fixture-only work; do not recreate the ledger. Record source-level API differences in the integration map.
- [ ] Commit the baseline contract and characterization tests only. Suggested checkpoint: `test: freeze design integration baseline`.

**Gate:** Exact owners and dependency pins are known; ordinary behavior is captured. This is a compatibility gate, not a claim that the proposed feature exists.

### Task 2: Define versioned domain types and fixtures

**Files:** Create `src/design/{mod.rs,types.rs,error.rs}` under `crates/davinci-coding-agent`; create `tests/design_types.rs`, `tests/fixtures/design/v1/`, and `design-ui/src/contracts.generated.ts`; wire existing `src/lib.rs` and `src/main.rs` module declarations minimally.

**Interfaces:** `validate_bundle(bundle: &SourceBundle, limits: &DesignLimits) -> Result<(), DesignError>`; `parse_revision_decimal(input: &str) -> Result<RevisionId, DesignError>`; typed commands and DTOs from the shared-contract table. Define host-minted `AuthorizedDesignContext` as a capability wrapper around existing authority, not caller-supplied strings.

- [ ] Write failing tests for a valid v1 round-trip; revision `9007199254740993` preserved as a decimal string; duplicate UUIDs; unknown schema; parent path; Windows drive path; case-fold collision; duplicate normalized path; 65 files; 256 KiB + 1 byte file; and 2 MiB + 1 byte total source.
- [ ] Run `cargo test -p davinci-coding-agent --test design_types --offline --locked`; observe failures for missing contracts/validation.
- [ ] Implement strict serde DTOs, checked fixed-width arithmetic, canonical path validation, and explicit schema-version handling. Schema annotations sent to a model do not replace host validation.
- [ ] Generate TypeScript contracts from the Rust-owned schema through a checked development script and compare golden JSON in both languages. Add no runtime code-generation dependency.
- [ ] Rerun the suite and check both binary/library targets with `cargo check -p davinci-coding-agent --all-targets --offline --locked`.
- [ ] Commit only contracts, fixtures, and minimal declarations. Checkpoint: `feat: define design artifact contracts`.

**Gate:** Invalid or oversized data fails before storage, model dispatch, or process launch.

### Task 3: Persist immutable revisions through session events

**Files:** Create `src/design/{store.rs,events.rs}` and `tests/design_store.rs`. Reuse `davinci-agent/src/runtime/evidence_store.rs`; adapt the existing session event writer identified in Task 1 without creating another authoritative journal.

**Interfaces:** `DesignStore::create(ctx: &AuthorizedDesignContext, input: CreateDesign) -> Result<ArtifactManifest, DesignError>`; `commit_revision(ctx, write: RevisionWrite) -> Result<DesignRevision, DesignError>`; `read_revision(ctx, artifact: ArtifactId, revision: RevisionId) -> Result<DesignRevision, DesignError>`; `rebuild_index(ctx) -> Result<RebuildReport, DesignError>`.

- [ ] Write failing tests: revision reopens after process restart; duplicate identical operation returns the first result; same operation ID/different digest conflicts; two writers on one expected revision yield one winner; cross-session access is denied; corrupt blob fails digest verification; crash before event leaves no visible head; crash after event rebuilds the same head.
- [ ] Run `cargo test -p davinci-coding-agent --test design_store --offline --locked`; verify intended failures.
- [ ] Implement blob-first, manifest-second, event-commit-third publication using `VerificationEvidenceStore::store_artifact`. Enforce owner-scoped lookup before blob retrieval, including cache hits. Use the session writer's serialization point for compare-and-swap.
- [ ] Implement restore as a new immutable revision and fork as a new artifact with ancestry. Pin accepted revisions and source assets. Session deletion must not silently destroy dependent artifacts.
- [ ] Rerun tests, including fault injection at each publication boundary. Assert the source and event sequence, not merely that a file exists.
- [ ] Commit: `feat: persist resumable design revisions`.

**Gate:** The index can be deleted and rebuilt without losing committed designs or reviving uncommitted revisions.

### Task 4: Add commands and existing-authority admission

**Files:** Create `src/design/{commands.rs,admission.rs}`, `src/native_extensions/design.rs`, `src/davinci_interactive/design.rs`, and `tests/design_commands.rs`. Modify existing args/main/startup/rpc/sdk/interactive/native-registration owners minimally.

**Interfaces:** `parse_design_command(input: &str) -> Result<DesignCommand, DesignError>`; `authorize_design_operation(existing_context: &ToolContext, operation: &DesignOperation) -> Result<AuthorizedDesignContext, DesignError>`; `dispatch_design(ctx, command: DesignCommand) -> Result<DesignResponse, DesignError>`. Browser clients call typed operations, not arbitrary CLI text.

- [ ] Write failing tests for the commands in spec §5, including `/design-sync` mapping to the canonical Sync operation; Unicode and quoted briefs; `/design -- open a cafe landing page`; unknown flags; absent IDs; a browser request forging ownership; plan-mode denial of apply/export; revoked policy after open; and disabled-mode tool/prompt parity.
- [ ] Run `cargo test -p davinci-coding-agent --test design_commands --offline --locked` and capture the expected failures.
- [ ] Wire parsing and typed dispatch to one controller facade. Start with fixture responses for generation; no live call before Task 12. Use existing permission/approval decisions for writes and existing read policy for inspection.
- [ ] Expose a small design tool family only in authorized design operations: create/read/patch/render. Keep apply, acceptance, export destination selection, and permission decisions host-owned. No tool may broaden task authority.
- [ ] Verify interactive, headless CLI, stdio RPC, and SDK adapters return the same IDs/errors. Do not claim that the distinct CBOR client/server surface gained support unless separately adapted and tested.
- [ ] Commit: `feat: add design commands with native authorization`.

**Gate:** Command entry never bypasses the parent task contract, even when invoked outside the TUI.

### Task 5: Build the authenticated local workspace host

**Files:** Create `src/design/host.rs`; `design-ui/package.json`, lockfile, `host/server.cjs`, `host/protocol.cjs`, `tests/host.test.cjs`; `tests/design_host.rs`. Package scripts: `build`, `typecheck`, `test`, `test:e2e`, each bound to Task 1's pinned dependencies.

**Interfaces:** `DesignHost::start(ctx: &AuthorizedDesignContext, config: &TrustedDesignHostConfig) -> Result<DesignHostLease, DesignError>`; `DesignHostLease::close() -> Result<(), DesignError>`. Host stdio accepts only versioned `DesignRequest`/`DesignResponse` envelopes; owner context is resolved in Rust.

- [ ] Write failing tests for missing token, replayed pairing secret, wrong Origin, forged Host, CORS preflight, query-string secrets, unauthenticated image retrieval, unsupported RPC operations, oversized JSON, oversized headers, 17th connection, queue overflow, slow headers, and lease revocation.
- [ ] Run `node --test crates/davinci-coding-agent/design-ui/tests/host.test.cjs` and the Rust `design_host` test target; verify the intended failures.
- [ ] Implement the loopback Node HTTP host and private stdio channel. Apply spec §7's exact limits. Use single-use fragment pairing and per-tab in-memory bearer capability; clear the fragment immediately. Never serialize provider credentials or generic filesystem handles.
- [ ] Add structured error handling, redacted logs, child cleanup, and parent-exit cancellation through the existing supervisor. Browser launch failure must still print a usable explicit local opening instruction without exposing a reusable long-lived credential.
- [ ] Rerun hostile-origin and resource-limit tests. Verify the host cannot serve generated HTML/JS as trusted workspace assets and never launches during normal sessions.
- [ ] Commit: `feat: add bounded local design host`.

**Gate:** A website running in the user's browser cannot operate Davinci merely by contacting localhost.

### Task 6: Compile only validated virtual artifact sources

**Files:** Create `src/design/compile.rs`; `design-ui/host/compile.cjs`, `tests/compile.test.cjs`; `tests/design_compile.rs`; store the preview runtime allowlist and hashes under `design-resources/`.

**Interfaces:** `compile_bundle(ctx: &AuthorizedDesignContext, revision: &DesignRevision, runtime: &PreviewRuntimeManifest) -> Result<CompiledDesign, DesignError>`. `CompiledDesign` contains artifact references and entry-point mappings, never executable commands supplied by the model.

- [ ] Write failing tests for a minimal TSX entry; local CSS; undeclared dependency; `node:fs`; absolute import; parent traversal; dynamic foreign import; project plugin/config discovery; missing pinned package; import-cycle bounds; source quota; malformed HTML; and cancelled compilation.
- [ ] Run `node --test crates/davinci-coding-agent/design-ui/tests/compile.test.cjs` plus the Rust `design_compile` target. Confirm untrusted imports fail before any external resolution.
- [ ] Implement fixed host-authored esbuild virtual-file resolvers. Resolve only manifest files and the exact preview library allowlist. Do not run Next/Vite configs, package scripts, generated modules, or arbitrary plugins.
- [ ] Apply a 30-second compile deadline, bounded stdout/stderr, supervisor cleanup, and content-addressed output. Preserve the last valid revision when a new build fails.
- [ ] Rerun with a fixture containing a malicious project `node_modules` and config script; assert neither executed and no network requests occurred.
- [ ] Commit: `feat: compile isolated design source bundles`.

**Gate:** Rendering source does not imply permission to execute a repository's toolchain.

### Task 7: Render through managed browsers and adopt durable evidence

**Files:** Create `src/design/render.rs`, `tests/design_render.rs`. Extend existing `native_extensions/browser.rs` and `interaction_testing/browser_process.rs`. Add a focused trusted helper `interaction_testing/design_capture.cjs`; locate the existing browser bridge entry point through Task 1 and wire the helper there.

**Interfaces:** `render_revision(ctx: &AuthorizedDesignContext, request: RenderRequest) -> Result<RenderReceipt, DesignError>`; `adopt_capture(ctx, browser_capture: BrowserCaptureHandle, revision: &DesignRevision) -> Result<ArtifactRef, DesignError>`. New host-only capture messages accept IDs and validated viewport/state enums, not scripts or URLs.

- [ ] Write failing tests for foreign origin, foreign localhost port, redirect, popup, WebSocket, service-worker request, WebRTC/WebTransport or speculative egress, external font/image, navigation, forged lease, stale source, wrong artifact owner, and 8-million-pixel capture limit.
- [ ] Run `cargo test -p davinci-coding-agent --test design_render --offline --locked`. Run the trusted real-browser fixture suite separately; missing browser yields BLOCKED, not simulated success.
- [ ] Serve compiled source on a content-only origin behind existing browser confinement. Deny service workers, unapproved navigation, and unsupported egress channels; verify effective confinement rather than assuming an HTTP proxy covers every protocol. Fail closed when the host cannot provide that evidence. Retain console/network/DOM evidence. Validate typed computed-geometry results and mark them as rendering data, not security authority.
- [ ] Capture web artboards at 390×844, 768×1024, and 1440×900; document/poster artboards use their declared size. Segment tall captures with explicit coverage. Adopt screenshot bytes into the durable evidence store while the original lease is valid.
- [ ] Close the browser, restart the session, and prove authorized screenshots still open. Prove revoked/cross-owner retrieval fails. Add optional managed headed preview with the same confinement; no raw-browser fallback.
- [ ] Commit: `feat: bind design renders to durable evidence`.

**Gate:** Screenshots survive renderer shutdown, but their former lease cannot authorize new activity.

### Task 8: Produce separate deterministic and visual-review outcomes

**Files:** Create `src/design/quality.rs`, `tests/design_quality.rs`, and fixtures for long text, missing fonts, clipping, forms, reduced motion, and keyboard focus.

**Interfaces:** `evaluate_quality(revision: &DesignRevision, receipts: &[RenderReceipt], review: Option<&VisualReview>) -> QualityReport`; `is_ready_for_review(report: &QualityReport) -> bool`. `VisualReview` identifies reviewer kind, exact revision, reference set, concrete findings, and unresolved limitations.

- [ ] Write failing tests for missing required viewport, old-source screenshot, console/network failure, omitted capture segment, missing label, keyboard trap fixture, motion under reduced-motion preference, unresolved asset, and an ARIA snapshot wrongly treated as full accessibility certification.
- [ ] Run `cargo test -p davinci-coding-agent --test design_quality --offline --locked`; verify failures reflect missing dimensions, not a hardcoded total score.
- [ ] Implement deterministic finding aggregation with evidence references. Use measured layout rectangles and controlled interaction fixtures. Contrast checks support known computed color cases; images/gradients needing manual assessment are incomplete rather than guessed passes.
- [ ] Keep aesthetic findings attributed to a model or human. Missing vision support leaves visual review pending. A failed required check cannot be overridden by a high subjective review rating.
- [ ] Rerun fixtures for each state and verify report transitions after a new revision invalidate old evidence.
- [ ] Commit: `feat: report source-bound design quality dimensions`.

**Gate:** “Ready for review,” “human accepted,” and “implementation verified” remain different facts.

### Task 9: Build the browser workspace and terminal summary

**Files:** Create `design-ui/src/{App.tsx,api.ts,styles.css}`, `components/{ArtifactList,Canvas,Inspector,RevisionHistory,EvidencePanel}.tsx`, and `tests/workspace.spec.ts`. Create `crates/davinci-tui/src/davinci/views/design.rs`; wire the existing TUI model/view registration confirmed in Task 1.

**Interfaces:** `DesignApi` uses the generated contracts and in-memory host capability. Canvas receives immutable `ArtboardCapture` and `ElementGeometry` records; it cannot load artifact JavaScript. The TUI receives `DesignSummary` with IDs, revision, state, and explicit open action.

- [ ] Write failing browser tests for artifact list/open, two concepts, viewport switching, zoom/pan, keyboard selection, a list-based artboard alternative, revision switching, evidence display, expired capability, reconnect, and unavailable renderer.
- [ ] Run `npm --prefix crates/davinci-coding-agent/design-ui run test:e2e` against trusted fixtures. Expected: missing workspace behavior, not external network access.
- [ ] Implement a restrained product interface: readable compact navigation, strong current-selection states, accessible inspector controls, and unambiguous Edit/Interact modes. Use the product profile, not Taste's landing-page hero rules.
- [ ] Display screenshots as inert images with trusted geometry overlays. Render all names, comments, errors, and code as text; no `dangerouslySetInnerHTML` for artifact content. Interact mode opens only a managed preview lease.
- [ ] Add loading/error/empty/conflict states and cancellation. Browser close does not delete a design. Terminal normal input remains responsive while a design operation runs through the existing runtime.
- [ ] Run `build`, `typecheck`, and `test:e2e`; rerun disabled-mode baseline tests. Commit: `feat: add local design artifact workspace`.

**Gate:** A user can compare and reopen designs without running generated code in the control origin.

### Task 10: Implement declared-property editing, comments, restore, and fork

**Files:** Create `src/design/{edits.rs,comments.rs}`, `tests/design_edits.rs`, `tests/design_comments.rs`; extend workspace Inspector/RevisionHistory and add `tests/editing.spec.ts`.

**Interfaces:** `apply_edit(ctx, edit: DesignEdit) -> Result<DesignRevision, DesignError>`; `add_comment(ctx, input: NewDesignComment) -> Result<DesignComment, DesignError>`; `restore(ctx, request: RestoreRevision) -> Result<DesignRevision, DesignError>`.

- [ ] Write failing tests for text and token edits, invalid enum/numeric values, HTML injection in text, missing node, changed binding hash, concurrent inspector/model updates, shared-token impact disclosure, duplicate request, orphaned comment, restore, and fork ancestry.
- [ ] Run `cargo test -p davinci-coding-agent --test design_edits --offline --locked` and `--test design_comments`; observe failing assertions before implementing edits.
- [ ] Edit only declared content/token JSON bindings and publish through Task 3. Preserve source consistency; do not mutate screenshot overlays or arbitrary DOM. Unsupported edits offer a scoped model revision, not a false visual success.
- [ ] Bind every comment to a revision and stable node when available. Preserve screenshot coordinates only as secondary reference. Restoring a revision creates a new history entry; forks receive new owner-checked artifact IDs.
- [ ] Run browser editing tests and assert a simple text/token edit makes zero model requests. Re-render after changes; mark previous screenshots/reviews stale immediately.
- [ ] Commit: `feat: add revision-safe design editing and comments`.

**Gate:** Human changes cannot be overwritten by stale agent output or detached from exported source.

### Task 11: Integrate Taste profiles and static design-system sync

**Files:** Create `src/design/{skills.rs,context.rs,sync.rs}`, `tests/design_profiles.rs`, `tests/design_sync.rs`; `design-resources/taste-skill/{SOURCE.json,LICENSE,SKILL.md}`; `design-resources/profiles/{marketing,product,document}.md` and their provenance manifests.

**Interfaces:** `select_profile(kind: ArtifactKind, brief: &DesignBrief, snapshot: Option<&DesignSystemSnapshot>) -> ProfileSelection`; `compile_design_context(ctx, selection: &ProfileSelection, inputs: &DesignInputs) -> Result<PreparedDesignContext, DesignError>`; `extract_system(ctx, root: WorkspaceHandle) -> Result<DesignSystemSnapshot, DesignError>`.

- [ ] Write failing tests for marketing vs dashboard routing; upstream source/section hash mismatch; missing license/provenance; dials outside 1..10; user-brand purple override; mandatory policy preserved during context pressure; no silent skill truncation; and non-design prompt parity.
- [ ] Run `cargo test -p davinci-coding-agent --test design_profiles --offline --locked`; verify missing profile behavior fails.
- [ ] Vendor the selected source at commit `ce26fc25c0e5e8cab638f883de62d9a86ee5e45b`, inspect its license, compute SHA-256, and record the actual bytes. Do not confuse the Git blob hash with SHA-256. Create reviewed compact profiles referencing complete named sections, with explicit local adaptations. Record differences from the upstream default.
- [ ] Load task-selected profiles through the existing context governor with the spec's 12,000-token maximum allocation. Preserve stable prefixes and provenance, and block when required context cannot fit. Do not modify the generic loader to reread mutable skill content mid-artifact.
- [ ] Add sync tests for CSS tokens, static TS theme objects, component props, missing/computed config, ignored/denied files, hostile project instructions, and modified source fingerprints. Implement static-only extraction using existing repository/indexing facilities; never execute project configs.
- [ ] Run `cargo test -p davinci-coding-agent --test design_sync --offline --locked` and verify stale/uncertain facts remain labeled. Commit: `feat: add pinned taste profiles and design-system snapshots`.

**Gate:** Taste improves relevant surfaces without overriding brand, security, accessibility, or source constraints.

### Task 12: Implement bounded generation, critique, repair, and resume

**Files:** Create `src/design/controller.rs`, `tests/design_controller.rs`, `tests/design_budget.rs`, and deterministic recorded-response fixtures. Extend the thin design adapter rather than modifying provider clients.

**Interfaces:** `DesignController::execute(ctx, command: DesignCommand) -> Result<DesignRunResult, DesignError>`; host-owned `DesignRunState` records phase, operation ID, expected revision, pinned context, root-admission lineage, and last committed revision. All request dispatch uses the existing agent/session API recorded in Task 1.

- [ ] Write failing tests for brief→directions→generation→render→review; two materially different concept briefs; repair after a real finding; a third repair denied; 13th model request denied; child requests counted at root; stopped stream/unknown outcome; cancellation; stale human edit; and restart after committed revision.
- [ ] Run `cargo test -p davinci-coding-agent --test design_controller --offline --locked` and `--test design_budget` using stub transports. Verify zero real provider calls.
- [ ] Implement a small phase coordinator on the existing runtime. Default to one worker. Parallel variants require explicit policy and share the same root limits; no per-variant reset or new scheduler.
- [ ] Use exactly the authorized session route/model/effort. Block API-key routes, fallback, unsupported output/vision assumptions, and missing admission support. The feature never purchases credits, accepts a paid image fallback, or treats a campaign-specific model as mandatory.
- [ ] Preserve the last committed artifact on compilation, review, quota, or model failure. Resume from committed state; do not repeat an unknown provider request or external effect. A lost parent authorization terminates child work.
- [ ] Rerun a repair after an inspector edit and assert the late result becomes a conflict. Integrate with the approved PR #82 admission owner only after its baseline gate clears. Commit: `feat: orchestrate bounded subscription design runs`.

**Gate:** Generation and review are one accounted operation, not an unbounded multi-agent loop.

### Task 13: Export the exact artifact revision safely

**Files:** Create `src/design/export.rs`, `tests/design_export.rs`; add export UI and `tests/export.spec.ts`.

**Interfaces:** `export_revision(ctx, request: ExportRequest) -> Result<ExportReceipt, DesignError>`. Receipt includes exact source revision, manifest hash, exported paths, asset-license omissions, and any executable-output warning.

- [ ] Write failing tests for source bundle reimport/hash match; PNG from a current capture; stale screenshot; incompatible HTML build; path traversal; symlink escape; existing-file overwrite; unknown format; unlicensed font exclusion; private metadata exclusion; and interrupted export.
- [ ] Run `cargo test -p davinci-coding-agent --test design_export --offline --locked` and observe the intended failures.
- [ ] Export into a staged destination after current write authorization. Publish completed files without clobbering existing data. Source export includes content/tokens/runtime manifest and a README, not prompts containing private sessions or credentials.
- [ ] Preserve licensing restrictions and disclose omitted assets. A partial or missing required asset is an incomplete export, not a finished package. Executable HTML is not auto-opened or deployed.
- [ ] Reopen the exported source with the trusted fixture compiler and compare render/source fingerprints. Export performs no model call and cannot advance implementation state.
- [ ] Commit: `feat: export versioned design artifacts`.

**Gate:** Exports are reproducible and explicit filesystem actions, not accidental publication.

### Task 14: Hand accepted designs to existing coding transactions

**Files:** Create `src/design/handoff.rs`, `tests/design_handoff.rs`; wire the existing graph/worktree/transaction owners located in Task 1. Do not add a parallel patch engine or modify `vendor/davinci`.

**Interfaces:** `prepare_handoff(ctx, request: HandoffRequest) -> Result<ImplementationProposal, DesignError>`; `approve_handoff(existing_user_decision: HostDecision, proposal: &ImplementationProposal) -> Result<ExistingTaskHandle, DesignError>`. User approval references the exact proposal/revision, not an open-ended future mutation.

- [ ] Write failing tests for unaccepted revision; changed accepted revision; dirty target file; modified dependency/route snapshot; plan-mode denial; revoked write permission; mock API mistaken for production integration; invalid RSC/client boundaries; transaction failure; and incomplete target verification.
- [ ] Run `cargo test -p davinci-coding-agent --test design_handoff --offline --locked` using target repository fixtures.
- [ ] Construct a bounded implementation brief: accepted source/evidence refs, target stack, preserved routes/content, required dependencies, explicit non-goals, and test obligations. Plan before mutation and present affected files/dependency changes.
- [ ] Reuse existing isolated worktrees and transactions. Preserve unrelated edits. A repository change after approval invalidates the proposal and requires revalidation; never overwrite silently.
- [ ] Verify actual target build/tests and browser behavior using existing evidence receipts. A successful preview or source copy cannot alone mark the coding task Verified. Do not commit/install/deploy without separate authorization.
- [ ] Commit: `feat: hand design revisions to verified coding tasks`.

**Gate:** Generating or selecting a design cannot modify the user's application.

### Task 15: Harden restart, ownership, quotas, and platform behavior

**Files:** Create `tests/design_recovery.rs`, `tests/design_security.rs`, `tests/design_limits.rs`; extend host/workspace tests; add documented fixture cases for Windows path casing/drive behavior and browser process cleanup.

**Interfaces:** Keep existing contracts. Add fault injection through test-only hooks, never a production “skip validation” switch.

- [ ] Write failing integration tests for termination at every publication boundary; corrupt/truncated last event; missing blob; stale index; expired browser lease; capability revocation during render/export/apply; concurrent sessions; cancelled child; and forged artifact ownership.
- [ ] Run `cargo test -p davinci-coding-agent --test design_recovery --offline --locked`, `--test design_security`, and `--test design_limits`. Confirm each failure proves a specific missing boundary.
- [ ] Harden session-tail recovery through existing session semantics; preserve corrupt evidence for diagnosis rather than silently replacing it. Scope prune to unreferenced blobs, including concurrent readers and accepted-revision pins.
- [ ] Test limits: source/files/assets, decoded image pixels, capture segments, 50 MiB render-run and 500 MiB task evidence caps, 100 MiB per artifact, bridge queue, request body, and model root counts. Use checked arithmetic and deterministic rejection before allocation/dispatch.
- [ ] Run Windows and Linux integration jobs with actual process-tree cleanup and installed trusted browser fixtures. Test macOS or mark it explicitly unverified; do not extrapolate native platform evidence.
- [ ] Test disabled-mode rollback, ordinary prompt/tool parity, preserved sessions, and source-only CLI operation without Node. Commit: `test: harden design lifecycle and security boundaries`.

**Gate:** Errors fail predictably without lost source, new authority, hidden requests, or false evidence.

### Task 16: Run evaluation gates and prepare a controlled release

**Files:** Create `crates/davinci-evals/tests/design_quality.rs`, `docs/design-artifacts.md`, `docs/readiness/design-artifacts.md`, and a narrowly scoped CI workflow or extend the existing compatible job. Add fixture manifest and a live-evaluation protocol under `docs/evals/design/`.

**Interfaces:** `DesignEvaluationRecord` stores task/source/profile/runtime hashes, baseline/candidate identity, deterministic gate results, reviewer preference/tie, model request counts, repairs, latency observations, and explicit unknowns. It is an evaluation record, not a fabricated product score.

- [ ] Write offline manifest tests: each of at least 12 briefs has target class, real content constraints, fixed references, expected states, viewport requirements, and secret-free fixture data. Include negative fixtures that must remain incomplete or denied.
- [ ] Run `cargo test -p davinci-evals --test design_quality --offline --locked`; implement loader/report validation until green. Do not initiate live evaluation automatically from CI.
- [ ] Run the final offline and trusted-browser commands below on the exact release source. Bind binaries, bundled UI, compiler, fonts, browser, fixture source, and evidence hashes in the readiness record.
- [ ] For later explicitly authorized live trials, compare the old workflow and Taste-integrated workflow on the same briefs/model/allowance. Blind order, retain ties, and report actual human preference and defect counts. Security, source fidelity, and mandatory interactions must pass regardless of preference. Poor preference outcomes trigger profile revision and a new versioned evaluation, not cherry-picked examples.
- [ ] Document setup without runtime downloads, controls, quotas, permission boundaries, missing-capability behavior, prototype limitations, and rollback. Release behind a host flag; preserve existing artifacts when disabled.
- [ ] Only after implementation/install authorization, follow the repo's installed-executable hash verification on the actual launch path. Preserve settings and active sessions. This planning task authorizes no installation or release. Commit: `docs: certify design feature evidence and rollout`.

**Gate:** Completion claims match the specific dimensions and environments actually tested.

## Suggested pull-request boundaries

| PR | Tasks | Review target |
| --- | --- | --- |
| A | 1–3 | Baseline, typed contracts, durable source and recovery |
| B | 4–6 | Command authority, local control host, compiler boundary |
| C | 7–8 | Confined rendering, durable evidence, truthful verification |
| D | 9–11 | Workspace, source-backed editing, profiles, static sync |
| E | 12–14 | Subscription-accounted generation, exports, explicit coding handoff |
| F | 15–16 | Adversarial integration, platform evidence, evaluation and rollout |

Do not merge a UI-only change that can execute unconfined content while waiting for the security PR. Feature flags and test-only fixtures keep incomplete paths inaccessible.

## Final verification commands

Run affected targets throughout implementation. These final commands are gates to execute later, not tests performed while writing this plan.

```sh
cargo fmt --all --check
cargo check --workspace --all-targets --offline --locked
cargo clippy --workspace --all-targets --offline --locked -- -D warnings
cargo test --workspace --offline --locked
node --test crates/davinci-coding-agent/export-html/template.test.cjs
npm --prefix crates/davinci-coding-agent/design-ui run typecheck
npm --prefix crates/davinci-coding-agent/design-ui run build
npm --prefix crates/davinci-coding-agent/design-ui test
npm --prefix crates/davinci-coding-agent/design-ui run test:e2e
```

A complete test inventory must prove each new `design_*` target ran; a substring selector that accidentally selects zero tests is not evidence. Browser integration records distinguish fake-process unit fixtures from real browser execution. The built UI manifest must match shipped assets, and browser baselines are platform/version-specific.

## Product acceptance checklist

- [ ] `/design` produces an artifact without changing application source or invoking an API-key route.
- [ ] Artifacts reopen after process restart with their exact source, revisions, comments, and adopted screenshots.
- [ ] Release B exposes two meaningfully different concepts, not just palette swaps.
- [ ] Direct text/token edits produce a new source revision without a model call; unsupported edits are labeled accurately.
- [ ] Concurrent human/model changes conflict rather than overwrite; restore/fork preserve history.
- [ ] Static sync preserves design-system provenance and never executes project configuration.
- [ ] Generated scripts cannot execute in the workspace origin or Node host; hostile networking tests fail closed.
- [ ] Required source/viewport/runtime fingerprints match current evidence; unavailable capabilities remain incomplete.
- [ ] Taste routing respects upstream scope and records explicit brand/accessibility overrides.
- [ ] Root limits include critiques, repairs, children, retries, and probes; unknown request outcomes stay unknown.
- [ ] Exports preserve exact revision and license boundaries; no public upload or auto-open occurs.
- [ ] Apply requires accepted revision, current target source, explicit approval, and real target verification.
- [ ] Disabling the feature preserves normal tools/prompts/startup and does not delete designs.
- [ ] Readiness distinguishes offline tests, real browser checks, human acceptance, platform coverage, and installed-binary verification.

## Stop and report conditions

Stop the affected operation, retain the last committed design, and report the precise limitation when authorization is revoked, source/evidence hashes diverge, required context cannot fit, trusted tooling fails verification, quota is exhausted, or a provider request ends in an unknown state. Do not repair any of these by silently changing provider, installing a package, disabling a security control, discarding history, or claiming a partial check proves completion.

## Sources and inspection limitations

The companion specification contains pinned repository source URLs and primary documentation. In particular, it cites the existing Rust architecture, skill loader, browser confinement contract, lease-bound screenshot behavior, durable artifact implementation, current Taste Skill scope, and pending PR #82 status.

This plan is based on read-only source inspection, not an implementation or executable audit. Some target integrations require the exact owner tracing in Task 1. All new paths, APIs, limits, tests, commands, and PR boundaries above are proposals; their existence is not asserted for the current Davinci repository.
