# Design artifacts (experimental)

Design artifacts keep generated UI source, revisions, comments and evidence in the owning DaVinci session. They do not change the application until a user reviews and approves a concrete implementation proposal. The feature is disabled by default. Read [readiness](readiness/design-artifacts.md) before enabling it; native generated-content rendering currently fails closed on Windows.

## Setup

Set `DAVINCI_DESIGN_ENABLED=1` in the launching process to enable mutation and the local companion. Generation requires the session's selected `openai-codex` OAuth subscription, model and thinking level, plus an existing `--root-budget` configuration. The [root budget contract](readiness/openai-harness-implementation.md#budget-operation) describes that configuration. There is no API-key fallback, implicit login, credential refresh or alternate model. Offline mode disables provider requests through CLI, RPC, SDK and the companion.

Listing, status, saved source and source export do not require Node or Chromium. Opening the companion, compiling, rendering and browser interactions require an explicitly prepared trusted runtime outside the application workspace:

1. Prepare Node **24.19.0** and the exact dependencies in `crates/davinci-coding-agent/design-ui/package-lock.json`. Dependency acquisition is a separate operator action; `/design` never installs packages.
2. Build the companion with `npm --prefix crates/davinci-coding-agent/design-ui run build`. Provide the Chromium revision expected by Playwright **1.62.1** in its normal cache, and installed local fonts. No browser/font download occurs during a design operation.
3. Run `node crates/davinci-coding-agent/design-ui/scripts/install-runtime.cjs <new-absolute-directory-outside-workspace>`. This explicit offline setup copies trusted code and records SHA-256 inventories of Node, packages, browser and fonts. It refuses an existing destination, missing dependencies, symlinks and incomplete inventories. A failed setup leaves its directory for inspection.
4. Set `DAVINCI_DESIGN_RUNTIME` to that directory and `DAVINCI_DESIGN_NODE` to the exact absolute Node executable. Subsequent checks reject changed tooling, extra files, missing browser files, modified fonts and workspace-owned tooling. Rebuild and prepare a new runtime after changing the bundled UI or compiler.

The installer does not install DaVinci globally. A release installation and verification of the actual PATH-resolved executable are separate actions. System font inventories bind evidence to that machine; they do not establish pixel parity across operating systems.

## Commands and controls

Use `/design` in an existing persistent coding session. The terminal `design` entry point accepts the same arguments; put ordinary DaVinci flags before `design`. UUIDs and revision numbers below are placeholders from `list` or `status`.

| Command form | Result |
| --- | --- |
| `/design list` | List artifacts on the selected session branch. |
| `/design new "brief" --kind landing --variants 2` | Generate one to three concepts; kinds are `landing`, `product`, `document`. |
| `/design open <artifact-id>` | Open the paired companion for this session. |
| `/design status <artifact-id>` | Report source and evidence state independently. |
| `/design revise <artifact-id> "change request"` | Generate a new source revision. |
| `/design verify <artifact-id>` | Attempt current captures and deterministic checks. |
| `/design fork <artifact-id> --revision <number>` | Create a separate artifact from retained source. |
| `/design restore <artifact-id> --revision <number>` | Publish a new head using the historical source. |
| `/design sync <relative-path>` or `/design-sync <relative-path>` | Retain bounded static repository facts with file/line hashes. |
| `/design export <artifact-id> --revision <number> --format source --destination <new-absolute-directory>` | Export exact retained source atomically. Formats also include `html` and `png`; PNG requires `--artboard <id> --viewport <width>x<height>`. |
| `/design apply <artifact-id> --revision <number>` | Open the accepted revision's implementation workflow; this command does not approve a patch. |

The companion provides revision selection, artboards, zoom, viewport selection, comments, source-backed text/token edits, generation, verification, export and implementation review. Direct edits update declared JSON bindings and require the expected revision; unsupported editing is rejected. Shared bindings require confirmation of all affected nodes. No model call is made for direct edits. Raster ingestion currently uses the trusted SDK store API; the UI does not offer arbitrary URL downloads or an upload control.

Edit mode displays inert captures and node geometry. Interact mode sends bounded click, fill, press and text-expectation actions to the separate confined browser. Prototype state is disposable. Forms and mocked data are prototypes, not production API integrations. The workspace origin never loads generated scripts or iframes.

## Evidence and approval

Source, render, interaction, accessibility, visual review, assets and implementation have separate states. A successful source compile does not establish visual acceptance. Captures bind the source and assets, artboard, viewport, theme, fixture, reduced-motion setting, runtime and policy. An edit makes older evidence stale. Basic geometry and accessible-name checks do not certify full accessibility compliance.

Acceptance records the exact revision and evidence, including explicitly acknowledged incomplete dimensions. It does not mark target implementation verified. Implementation drafting uses selected clean UI paths in this coding session's linked Git worktree and one accounted subscription request. Review the actual native plan and patch preview before approving its hash. Changed source, dependencies, acceptance, worktree identity or authority invalidate approval. Package/config changes, moves and deletions are outside this first handoff.

Applying uses DaVinci's existing patch transaction and preserves unrelated edits. It returns **applied / pending_verification**. Run the actual target build, tests and primary browser flow through the normal coding workflow. Design previews and synthetic fixture receipts never mark the target transaction Verified. Automatic target verification orchestration is still a release blocker; see readiness. There is no automatic commit, installation, deployment or upload.

## Limits and failure behavior

| Resource | Bound |
| --- | --- |
| Source | 64 files, 256 KiB each, 2 MiB total; portable paths, no case collisions |
| Concepts / artboards | 1–3 concepts, 8 artboards |
| Raster assets | PNG/JPEG/WebP only; 20 files, 10 MiB each, 16 million decoded pixels each |
| Evidence | 50 MiB per render run, 100 MiB cumulative per artifact, 500 MiB per session task store |
| Model run | At most 12 root-accounted attempts, two repair passes, original ten-minute deadline; stricter parent limits win |
| Implementation draft | One accounted request; 1–16 UI paths, 128 KiB target context, 512 KiB full context |
| Companion | 1 MiB requests, 256 KiB replies, 32 jobs, one active long operation; 8 pending bridge requests |
| Pairing / lease | One-time 60-second pairing capability; eight-hour host lease |

Host authority comes from live permission and task-contract checks, never browser payload fields. Cancellation and revocation stop work; the last committed revision remains readable. Unknown provider outcomes retain their budget reservation and block automatic repetition. A compiler error can trigger a bounded repair; unavailable trusted tooling or confinement cannot be repaired by relaxing security.

Session events are authoritative. Immutable blobs are written before event publication; interrupted unreferenced blobs may be pruned under the session writer lock. Pruning retains references from all branches, accepted revisions and checkpoints. Missing or corrupt referenced data is an error and the evidence is retained for diagnosis. On first write after a torn final session record, its exact bytes are flushed to a sibling `.torn-<uuid>.bak` before truncation; a failed backup blocks recovery. A terminated corrupt record remains an error. A crash after target patch application uses the native transaction journal for recovery; do not blindly replay an approval.

## Rollback

Close the companion and unset `DAVINCI_DESIGN_ENABLED` (or set it to `0`) for the next launch. Ordinary tools and prompts remain unchanged. The disabled controller retains list/status/read and source export; it refuses new mutations and rendering. Do not delete the session WAL or artifact store to disable the feature. Existing accepted source, comments, revisions and evidence remain available for a later enabled session.
