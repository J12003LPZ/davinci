# Davinci Design Artifacts: Product and Architecture Specification

**Status:** Proposed design for review. Planning deliverable only; implementation, installation, live model runs, repository changes, and publication are not authorized by this document.

**Prepared:** October 2, 2026.

**Inspected Davinci baseline:** `main` at `f24ed1c744a31ee49c95802f982740fe59d2de6d`, product version 1.0.71, Rust 1.83.0.

**Related work:** PR #82, head `dd6725f758a3619892349c59664f1d3f1a5e53fd`, is open and stacked on PR #81. Its durable subscription-budget changes are not part of the inspected `main`. Reconcile with the approved integration branch before implementing runtime admission. Do not automatically merge, cherry-pick, or duplicate those changes.

**Companion plan:** `docs/superpowers/plans/2026-10-02-davinci-design-artifacts.md`.

## 1. Outcome and assumptions

Build a native `/design` product workflow in Davinci: a user describes a visual artifact, reviews distinct concepts in a local browser workspace, edits supported properties or requests changes, resumes later, and explicitly exports or hands an accepted revision to the coding harness.

The useful reference behavior is conversation plus canvas, design-system context, direct edits, comments, and design-to-implementation handoff. This is an independent implementation, not integration with undocumented Claude services or a claim of feature-for-feature parity. Anthropic's current public guide documents `/design` and `/design-sync` [S1].

User requirements: personal harness, detailed implementation plan, Taste Skill integration, and avoidance of generic generated designs. Preserved project constraints: native Rust ownership and the user's OpenAI/Codex subscription route, without API billing or fallback. Proposed assumptions: one local operator, browser companion alongside the terminal, local persistence, no public sharing service, and no requirement to build a general vector editor.

Success is an editable, resumable, code-backed artifact whose appearance has been checked against its brief and whose accepted revision can be implemented without silently changing the repository. A generated screenshot alone does not satisfy this outcome.

## 2. Scope and release boundaries

### Release A: A usable local vertical slice

Support landing pages, constrained client-side product mockups, and HTML one-pagers/posters. Provide `/design new`, list/open/status, one generated concept, source storage, local preview, revision history, and source/PNG export. A single artboard has multiple viewport captures. Browser availability and visual-review limitations are explicit.

### Release B: The complete personal design workflow

Add two-concept generation and comparison, artboard collections for small screen flows, declared-property editing, anchored comments, forks, restore, repository design-system snapshots, bounded visual critique, and explicit implementation handoff. This is the target feature described by the rest of this specification.

### Deliberately deferred

Arbitrary drag-and-drop HTML reconstruction; lossless Figma import/export; a vector scene graph; unrestricted third-party package execution; Next.js server execution inside the preview service; real-time collaboration; cloud accounts; public links; a marketplace; autonomous deployment; and native editable DOCX/PPTX/PDF generation. Browser-print output can be added later with its own pagination tests. Do not imply that HTML exports are native office documents.

Inline arbitrary JavaScript in the privileged browser workspace is also deferred. Interactive prototypes run in the existing managed browser boundary, not beside control-plane credentials.

## 3. Architecture decision

Considered alternatives:

| Approach | Advantage | Problem | Decision |
| --- | --- | --- | --- |
| A prompt alias that writes HTML | Small implementation | No durable artifact ownership, editing contract, recovery, or source-bound evidence | Insufficient |
| Code-backed artifact service with a local companion | Reuses harness controls and produces implementable source | Requires deliberate edit bindings and a secure rendering boundary | Selected |
| Full vector editor and collaborative canvas | Broad visual manipulation | Introduces a second application platform and difficult source round-tripping | Defer |

The selected design adds a cohesive `design/` module inside `davinci-coding-agent`, a thin native-tools adapter, and a bundled browser companion. It does not replace `davinci-agent`, `davinci-ai`, the graph runtime, or the session store.

```text
TUI / CLI / existing stdio RPC
             |
       DesignController
             |
   +---------+-----------+-----------------+
   |                     |                 |
existing agent       artifact/session    trusted design host
runtime and policy   persistence         and compiler bridge
   |                     |                 |
subscription model   immutable source     managed Playwright
   |                 + revision events   + origin confinement
   +---------------------+-----------------+
                         |
              screenshots / geometry / evidence
                         |
                local browser workspace
                         |
             explicit accepted-revision handoff
                         |
        existing graph + worktrees + transactions
```

The controller owns product state transitions, not a new scheduler. All model turns, tools, approvals, cancellation, and accounting continue through their current owners.

### Existing owners to preserve

- `davinci-coding-agent/src/davinci_interactive.rs`: interactive integration. Add a child module instead of growing the large file with feature internals.
- `davinci-coding-agent/src/rpc.rs` and `sdk.rs`: existing embedding surfaces. Do not confuse stdio RPC with `davinci-protocol`'s separate typed client/server transport.
- `davinci-agent/src/skills.rs`: resource discovery and skill expansion. Add a design-specific immutable snapshot adapter; do not change ordinary skill semantics.
- `davinci-agent/src/runtime/evidence_store.rs`: `VerificationEvidenceStore::store_artifact` and verified artifact retrieval.
- `davinci-agent/src/runtime/evidence.rs`: `ArtifactRef`, evidence status, and current-source semantics.
- `davinci-coding-agent/src/native_extensions/browser.rs`: browser authorization, opaque resources, source observations, and retained screenshots.
- `davinci-coding-agent/src/interaction_testing/browser_process.rs`: trusted browser process protocol and supervision pattern.
- Existing session, process-manager, permission, transaction, worktree, graph, and context-governor owners.

These relationships are supported by inspected source and architecture documentation [S2–S7]. Exact integration call sites must be retraced after baseline reconciliation.

## 4. Global constraints

The following lines are repeated verbatim in the implementation plan.

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

## 5. User experience and command contract

All commands below are proposed additions. A shared parser serves terminal commands and CLI dispatch; RPC invokes the same typed controller, rather than reparsing command strings.

```text
/design "Create a restrained onboarding prototype for this repository"
/design new "Create a landing page" --kind landing --variants 2
/design list
/design open <artifact-id>
/design status <artifact-id>
/design revise <artifact-id> "Make the navigation easier to scan"
/design verify <artifact-id>
/design fork <artifact-id> --revision 4
/design restore <artifact-id> --revision 2
/design export <artifact-id> --revision 4 --format source
/design apply <artifact-id> --revision 4
/design-sync <repository-relative-path>
```

`/design` without arguments opens the artifact list. Free-form text means `new`; reserved subcommands are exact matches. `/design -- <text>` escapes a brief beginning with a reserved word. Quoted content is preserved; arguments are not interpolated into shell commands. The canonical sync operation is `DesignCommand::Sync`; `/design-sync` is an alias. Headless `davinci design ...` must expose the same behavior without opening a browser unless requested.

Workspace layout: artifact/variant navigation on the left, pan/zoom artboards in the center, inspector and comments on the right, and a compact run/revision/evidence strip. Separate **Edit** and **Interact** modes. The workspace must support keyboard navigation and a list-based alternative to spatial selection.

An artboard is one screen or document, not a viewport. A concept can contain multiple artboards. Viewport captures belong to those artboards. Default generation creates two concepts only in Release B; Release A exposes its one-concept limit honestly. Maximum three concepts and eight artboards per artifact initially.

The canvas shows screenshot-backed views and trusted selection overlays. The underlying artifact contains real source, tokens, content, and binding metadata. Supported inspector edits modify that source bundle and regenerate the view. There is no independent editable scene graph that can drift away from the exported code.

## 6. Data model and identity

Define shared domain types in `design/types.rs`; use a checked-in generated TypeScript contract plus golden serialization fixtures. Generate UI types from Rust-owned schemas during development, not at user startup. Reject unknown protocol fields and unsupported schema versions at mutation boundaries. A future schema can be opened as raw read-only metadata with an explicit incompatibility notice, never reinterpreted as v1.

- `ArtifactId`, `VariantId`, `ArtboardId`, `NodeId`, `CommentId`, `OperationId`: validated opaque UUIDs.
- `RevisionId`: `u64`, monotonically increasing within an artifact. Serialize to decimal strings in browser-facing JSON to avoid JavaScript integer precision loss.
- `ArtifactManifest`: schema version, owner session/branch, workspace identity, title, kind, brief reference, current revision reference, and created timestamp.
- `DesignRevision`: immutable revision ID, parent revision, branch/fork provenance, source manifest hash, variants/artboards, token/content references, skill snapshot references, system snapshot, asset manifest, and author operation.
- `SourceBundle`: sorted relative-path-to-`ArtifactRef` map plus declared entry points. No absolute paths, drive prefixes, parent traversal, symlinks, duplicate normalized paths, or case-fold collisions.
- `EditableBinding`: `NodeId`, owning artboard, source file/hash, declared binding path, allowed property type, and validation constraints.
- `DesignEvidence`: revision/source hash, viewport, theme, motion preference, browser/compiler/font/asset versions, policy hash, deterministic checks, screenshot reference, and review attribution.
- `DesignEvent`: create/revision/comment/selection/acceptance/handoff/cancellation records stored through the session's existing custom-event extension mechanism. Confirm its exact adapter in the baseline task; do not silently introduce a second journal.

Persist under the existing resolved agent directory, logically `design/<workspace-key>/...`. Do not hardcode `~/.pi` versus `~/.davinci`; current documentation contains legacy naming, so use the actual resolver. The owning session remains the recovery authority. A session deletion action must disclose dependent artifacts and require explicit cascade or ownership transfer.

### Publication protocol

1. Recheck current session, workspace, task permissions, and expected revision.
2. Validate paths, types, sizes, asset references, and operation identity.
3. Write immutable blobs using the existing evidence store. Its hash-based identifier is a locator, not authorization.
4. Write the immutable revision manifest.
5. Append one committed revision event under the session writer's serialization/CAS boundary.
6. Advance the derived index only after the event is durable.

A crash before step 5 leaves unreferenced blobs, not a visible partial revision. A crash afterward is recovered from events. The same idempotency key and payload digest returns the original result; the same key with different payload is a conflict. Two operations based on revision 4 cannot both overwrite its head. Losing writes remain explicit conflicts, not automatic merges.

Use full SHA-256 values for integrity comparisons even where an existing display ID contains a short prefix. Recheck ownership on content-cache hits. Accepted revisions, referenced assets, and evidence stay pinned. Explicit prune can remove unreferenced blobs after a mark-and-sweep pass; never silently evict the only source copy to meet a quota.

## 7. Rendering and security boundary

### Trusted workspace and host

Use a bundled React/TypeScript workspace with ordinary CSS tokens. Build it with an exact-pinned toolchain; ship the built assets with the CLI. A small supervised, trusted Node host uses `node:http`, serving only its known static bundle and narrowly scoped JSON endpoints. This avoids adding a web framework or asynchronous server stack throughout the Rust core. Node is an optional requirement for this design/browser feature, not a new requirement for normal terminal use.

The host communicates with Rust over a private framed stdio channel. It never receives provider credentials, inherits an allowlisted environment, starts outside the user's project, and exposes no generic file, shell, evaluation, or proxy endpoint. Rust remains the authority on every read and mutation.

Bind to an OS-selected port on `127.0.0.1`, never all interfaces. Validate exact Host and Origin. Use a 256-bit, single-use pairing secret in the initial URL fragment, consume it once, immediately clear it from history, and exchange it for a per-tab capability held only in memory. Do not place secrets in query strings, logs, localStorage, or generated artifacts. Require bearer authorization for data endpoints; mutations require exact Origin and JSON content type. Deny CORS, cross-site fetches, CONNECT, and protocol upgrades. Revoke capabilities when the owning session/lease closes. Test each control rather than relying on localhost as authentication.

Initial server limits: 16 active connections, 8 queued RPC requests, 16 KiB headers, 1 MiB JSON bodies, 5-second header/idle timeouts, 30-second request timeout, and 256 KiB bounded event-poll responses. Poll only while work is active; do not introduce WebSocket infrastructure. Oversize or overloaded requests fail with typed errors and no partial mutation. Node exposes server timeout/header controls, but policy and application bounds still require implementation [S11].

### Compiler and untrusted previews

HTML/CSS and client-only TSX are supported. A trusted, exact-pinned esbuild worker accepts a validated virtual file map and a fixed library allowlist; it does not load project configs, plugins, lifecycle scripts, SSR code, or project node_modules. Only host-authored resolver/loader hooks are permitted. React and its JSX runtime come from the verified preview tool bundle. No arbitrary new imports or network dependency resolution. esbuild's virtual-module plugin interface supports this design, but the allowlist is Davinci policy, not a claim that esbuild itself is a security sandbox [S12].

Compilation is supervised with a fixed executable path, arguments, deadline, and clean environment. Never `require`, import, or evaluate generated modules in Node. Compiled browser code runs only inside an isolated managed Playwright context on its own constrained content origin. There is no control API on that origin, and no workspace capability is passed to it.

Retain and extend the existing proxy boundary for foreign hosts, foreign loopback ports, redirects, WebSocket traffic, service workers, popups, and navigations. Rendering requires a positive confinement readiness check, including non-HTTP channels such as WebRTC/WebTransport and speculative network activity. A configured HTTP proxy alone is not proof that every browser egress path is closed. Disable unsupported channels and require effective network-isolation evidence where needed; otherwise scripts-enabled preview remains unavailable rather than silently weakening the boundary. An iframe alone is insufficient; MDN specifically warns about combining script and same-origin permissions for same-origin content [S10]. No claim of OS process isolation follows from a permission preset. A compromised same-user host remains outside the initial threat model; arbitrary host execution is not authorized.

The ordinary workspace shows inert captures. Interactive preview opens a managed headed browser context using the same confinement, with only one active interactive context per artifact. If headed support is unavailable, show that limitation and keep noninteractive review; do not open raw generated HTML in the user's ordinary browser as a fallback. Continuous remote video streaming is deferred.

Host-only capture helpers may collect layout rectangles, computed styles, focus state, and declared node bindings. The model never supplies arbitrary JavaScript or CDP instructions. Extend the existing browser bridge protocol deliberately rather than weakening its current selector boundaries.

### Assets and leakage controls

Use local user-provided assets or explicit trusted imports. Strip unnecessary metadata, verify media types, bound decoded pixel counts, and preserve license/source attribution. Untrusted SVG is rasterized or sanitized in the managed pipeline; never embed it as executable workspace markup. Font files must have redistribution permission before export.

No image API is implicitly authorized by a Codex subscription. Image generation is used only through an actually available and separately authorized capability with known accounting; otherwise request/provide an explicit missing-asset slot and mark completion accordingly. Do not invent customers, testimonials, compliance claims, or business metrics to fill a design.

## 8. Source-backed edits, comments, and flows

Use declared editable data, not arbitrary DOM mutation. Generated components refer to a `design.content.json` and `design.tokens.json` contract, with stable `data-design-node` identifiers. Supported v1 edits are plain text, numeric token values within type-specific ranges, color tokens, local asset references, and enum-valued layout choices. An edit to a global token discloses all affected artboards before confirmation.

`apply_edit` requires artifact ID, expected revision, node ID, expected binding hash, operation ID, and typed value. Apply to a copy, validate, then publish a new revision. Editing JSX expressions, arbitrary CSS selectors, route logic, and component restructuring uses a scoped agent revision with source diff review. Do not pretend that a WYSIWYG editor can reliably round-trip every React program.

A comment stores revision ID, artboard ID, node ID when present, text, and optional normalized screenshot coordinates. Its anchor is version-specific. Removed nodes become orphaned comments; never attach them to a different node merely because the text or screen location looks similar.

Undo/restore creates a new revision from a previous snapshot; it does not delete history. A fork has a new artifact ID and explicit ancestor. Prototype flows use local fixture data and constrained screen navigation; loading, empty, error, and success states are explicit. No real login, purchase, message send, or production API is invoked by a mockup.

## 9. Taste Skill integration

### Source and version

Inspect and vendor the chosen upstream snapshot deliberately:

- Repository: `Leonxlnx/taste-skill`.
- Commit: `ce26fc25c0e5e8cab638f883de62d9a86ee5e45b`.
- Default skill path: `skills/taste-skill/SKILL.md`.
- Inspected Git blob: `b72132fcd466da605623ffe96e370b3991fc5285`.
- Frontmatter name: `design-taste-frontend`.

The Git blob identifier is not a SHA-256 checksum. Compute and record SHA-256 of the actual vendored bytes, upstream license, source URL, review date, and any local derivative profile. Never auto-update from `main` during generation. Upstream describes v2 as experimental and retains v1 separately [S8].

### Apply the right rules to the right surface

The current default skill explicitly excludes dashboards, dense product UI, data tables, multi-step forms, code editors, and real-time collaborative interfaces. Its full marketing layout rules must not be applied to the Davinci workspace or a dashboard artifact [S9].

| Profile | Guidance | Proposed variance / motion / density |
| --- | --- | --- |
| Marketing | Pinned upstream brief inference, relevant design-system guidance, redesign protocol, and applicable preflight | 7 / 4 / 4 |
| Product UI | Local `davinci-product-design` profile, existing product design system, relevant attributed Taste principles | 4 / 2 / 7 |
| One-pager / poster | Local document profile: content hierarchy, typography, controlled layout, readable export | 5 / 1 / 4 |

These are proposed Davinci defaults, not upstream defaults. Keep the exact dial names `DESIGN_VARIANCE`, `MOTION_INTENSITY`, `VISUAL_DENSITY`, each from 1 to 10. User brand requirements and accessibility constraints override stylistic heuristics. Never equate more motion or more asymmetry with higher quality.

Declare a design read before generation: artifact kind, audience, primary task, brand constraints, visual direction, and rationale. Produce two short concept briefs that differ in information hierarchy, layout, type, or imagery, not only accent color. Preserve real content in all comparisons.

### Bounded context, not a global prompt dump

The existing skill loader reads mutable files and ordinary graph context has a small skill allocation. Snapshot selected content through a design adapter and retain exact provenance. Do not silently truncate the upstream document to fit a generic graph budget. Do not load every Taste variant.

The runtime profile consists of a reviewed compact adapter plus complete, named upstream sections selected for the task. Record their source byte ranges and individual hashes. Label this honestly as a curated profile, not the entire upstream skill. Reserve up to 12,000 estimated input tokens for the design profile and references inside the existing governor; available context may be lower and can block admission. Keep stable profile/tool instructions before dynamic brief/source changes for cache locality. Normal non-design prompts must remain unchanged.

### Quality loop

1. Freeze the brief, supplied copy, references, and design-system snapshot.
2. Choose the applicable profile and record explicit heuristic overrides.
3. Generate the source and declared editable bindings.
4. Compile and capture the required viewport/theme/state set.
5. Run deterministic checks for missing assets, console/network failures, clipping/overflow, labels, focus, contrast cases supported by the checker, and interactions.
6. Review actual screenshots against the brief, reference direction, visual hierarchy, content, and concept distinctness. Use image-capable model input only when the authorized route supports it; otherwise require human visual review.
7. Repair only concrete findings, with a maximum of two automatic repair rounds for the selected concept. Re-run affected checks after each source change.
8. Preserve the last valid revision on failure and surface unresolved findings.

Accessibility requirements are not delegated to stylistic advice. Use WCAG's actual contrast definitions rather than repeating an incorrect large-text pixel threshold from a skill [S13]. Automated checks and an ARIA snapshot do not establish full accessibility conformance. Aesthetic preference is human judgment; there is no objective universal “anti-slop score.”

## 10. Read-only design-system sync

`/design-sync` extracts a versioned snapshot, not a synchronized second component repository. Inspect authorized source using existing indexing/tree-sitter capabilities: CSS custom properties, static theme objects, package names/versions, representative component exports and props, route structure, font declarations, and asset references.

Separate observed facts from heuristics. For every extracted item record source path/hash/range, confidence, and resolution status. Ignore credentials, environment files, private logs, generated directories, and denied paths. Read project instructions as lower-trust task context, never as authority to install tools or change permissions.

Do not evaluate Tailwind/Next/Vite configuration, execute Storybook, or import arbitrary components during extraction. Static extraction of computed configuration can be incomplete; record `unresolved` and let the user select an example. Executable integration previews are a separate authorized coding task.

A snapshot is immutable and stale once its source fingerprint changes. Generation must acknowledge stale snapshots; apply must revalidate target files and regenerate its implementation plan when necessary. Brand-preserving redesign retains routes, identifiers, legal copy, and analytics hooks unless explicitly changed.

## 11. Runtime admission, budgets, and recovery

Use the existing authorized session's model and effort. No model names are hardcoded into the feature. The fixed model mentioned in PR #82 is a campaign setting, not a design product default.

Proposed initial root limits: 12 model requests across all phases and children, one concurrent generation worker, two default concepts, three maximum concepts, two automatic repair rounds for the selected concept, and 600 seconds wall-clock per generation operation. Every request, critique, repair, retry, and capability probe counts. The stricter parent budget always wins. These are resource limits, not latency promises or claims about account allowance.

Integrate with the reconciled root admission owner from the harness work. Durable reservations precede dispatch; unknown outcomes remain unknown and are not replayed blindly. Subscription allowance observations are advisory unless attribution and enforcement are actually supported. Never treat missing usage as zero or API-equivalent dollars as a payment authorization.

Default source limits: 64 files, 256 KiB per text file, 2 MiB total source, 20 assets, 10 MiB encoded per asset, 16 million decoded pixels per image, and 100 MiB per artifact including retained revisions. Rendering respects the existing 50 MiB run and 500 MiB aggregate task evidence caps; lower applicable limits win. A single capture is bounded to 8 million pixels. Tall pages use bounded segments with coverage accounting, not an unbounded full-page image. Quota failure preserves current source and reports evidence omissions.

Render cache key: source + toolchain + assets/fonts + viewport/theme/state + policy fingerprints. Cache hits require current ownership and verification status. A changed source invalidates evidence; a changed policy invalidates safety approval; an expired browser lease cannot authorize retrieval.

Existing screenshot resources are tied to browser leases. Before closing a browser, adopt the screenshot bytes into the durable design artifact store through a host-authorized operation, verify their digest, and append their reference to evidence. Resume must work after the original browser process has exited. Do not store only a transient `browser_id`.

## 12. Evidence states and honest completion

Track separate dimensions rather than one misleading green badge:

- Source: valid / invalid.
- Rendering: current / stale / failed / unavailable.
- Interaction checks: current / incomplete / failed / not required.
- Accessibility checks: current / incomplete / failed.
- Visual review: pending / model-reviewed / human-approved, bound to revision.
- Assets: complete / placeholders remaining.
- Implementation: not requested / planned / applied / verified / failed.

“Ready for review” requires a valid source, current required captures, no failing required deterministic checks, and explicit disclosure of incomplete asset/accessibility/review dimensions. “Accepted” requires user approval of that revision. Neither means production integration has passed.

A browser receipt is evidence, not permission to transition a repository transaction to Verified. Preserve the existing completion/evidence protocol [S5–S7].

## 13. Export and implementation handoff

`source` export writes the exact selected revision, entry points, local assets with rights, tokens/content, provenance, and a README containing toolchain and limitations. No auth material, transcripts, or hidden reasoning. `png` exports a retained current capture. `html` exports a validated single-artboard build where supported, with local dependencies and an explicit executable-code warning; opening it outside the managed environment relinquishes Davinci's preview isolation. Do not auto-open or upload it.

All filesystem exports require a user-selected destination and the existing write permission. Do not overwrite existing files without confirmation. Reject path traversal and symlink destination escapes.

`/design apply` creates a handoff request referencing one accepted revision and its source/evidence hashes. The coding harness inspects the target stack, dependencies, routes, and current working-tree changes, then presents a scoped implementation plan and diff. Reuse existing worktree isolation and transactional edits. Do not blindly copy preview React into a Next.js server component, or turn mock data into a production integration.

Applying is an explicit code-writing action distinct from generating, selecting, accepting, and exporting. Existing plan-mode restrictions still apply. On target changes, block stale application and replan. Run actual target build/test/browser gates before implementation can be reported as verified. No automatic commit, merge, install, or deploy.

## 14. Validation and rollout

Offline deterministic tests use recorded model responses and local fixtures. Browser gates use a trusted installed runtime, pinned fonts, fixed viewports, and a controlled clock; maintain platform-specific baselines because screenshots vary with browser, OS, hardware, and rendering configuration [S14].

Required fixtures cover marketing, dense product UI, forms with all states, a document with long copy, brand-preserving redesign, malicious markup/URLs, path traversal, source conflicts, corrupt storage, cancelled runs, missing browser/fonts/images, and cross-artifact access. Test keyboard flow and reduced motion in addition to screenshots.

For a later explicitly authorized live quality evaluation, freeze at least 12 briefs across the three artifact classes. Compare baseline generation against the integrated workflow using the same model, content, tools, and root allowance. Blind the ordering and record human preference, ties, task fidelity, broken interactions, repair rounds, request counts, and incomplete assets. Report counts and limitations, not invented statistical certainty or a universal taste metric. No live evaluation is authorized by this planning task.

Roll out behind a host-level feature flag. First prove source persistence and safe rendering, then editing and handoff. Disabled mode must not start Node, change ordinary prompts, or add design tools. Rollback disables entry points while preserving artifact data and read-only export.

For a future approved implementation delivery, follow the repository's installed-executable verification rule on the user's actual launch path. A source build is not proof that the installed application changed. Documentation-only delivery here changes no executable.

## 15. Source record and limitations

Read-only source inspection informed this proposal. The repository was not compiled, tests were not run, and no live model or browser feature was exercised. Some integration owners were located through architecture docs and source excerpts rather than a complete call-graph audit. Task 1 requires exact-baseline tracing before implementation.

[S1] Claude Design public workflow: https://support.claude.com/en/articles/14604416-get-started-with-claude-design

[S2] Davinci architecture: https://github.com/J12003LPZ/davinci/blob/f24ed1c744a31ee49c95802f982740fe59d2de6d/docs/ARCHITECTURE.md

[S3] Davinci project rules: https://github.com/J12003LPZ/davinci/blob/f24ed1c744a31ee49c95802f982740fe59d2de6d/CLAUDE.md

[S4] Davinci skill loader: https://github.com/J12003LPZ/davinci/blob/f24ed1c744a31ee49c95802f982740fe59d2de6d/crates/davinci-agent/src/skills.rs

[S5] Browser verification contract: https://github.com/J12003LPZ/davinci/blob/f24ed1c744a31ee49c95802f982740fe59d2de6d/docs/browser-verification.md

[S6] Native browser implementation: https://github.com/J12003LPZ/davinci/blob/f24ed1c744a31ee49c95802f982740fe59d2de6d/crates/davinci-coding-agent/src/native_extensions/browser.rs

[S7] Durable evidence storage: https://github.com/J12003LPZ/davinci/blob/f24ed1c744a31ee49c95802f982740fe59d2de6d/crates/davinci-agent/src/runtime/evidence_store.rs

[S8] Taste Skill repository/README: https://github.com/Leonxlnx/taste-skill/tree/ce26fc25c0e5e8cab638f883de62d9a86ee5e45b

[S9] Taste Skill source, especially sections 0, 1, 11, 13, and 14: https://github.com/Leonxlnx/taste-skill/blob/ce26fc25c0e5e8cab638f883de62d9a86ee5e45b/skills/taste-skill/SKILL.md

[S10] iframe sandbox limits: https://developer.mozilla.org/en-US/docs/Web/HTML/Reference/Elements/iframe

[S11] Node HTTP server controls: https://nodejs.org/api/http.html

[S12] esbuild virtual-module interfaces: https://esbuild.github.io/plugins/

[S13] WCAG contrast definitions: https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html

[S14] Playwright visual comparisons: https://playwright.dev/docs/test-snapshots

[S15] Pending root-budget integration: https://github.com/J12003LPZ/davinci/pull/82
