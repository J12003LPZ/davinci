# Design artifacts (experimental)

Design artifacts keep generated UI source, revisions, comments and evidence in the owning DaVinci session. They do not change the application until a user reviews and approves a concrete implementation proposal. The feature is disabled by default. Read [readiness](readiness/design-artifacts.md) before enabling it.

## Quick setup (Windows)

From a DaVinci checkout, with Node **24.19.0** installed (`winget install OpenJS.NodeJS --version 24.19.0`):

```powershell
pwsh -NoProfile -File .\scripts\setup-design-runtime.ps1
```

It installs the companion's locked dependencies, builds it, installs the browser builds the pinned Playwright expects (without deleting browsers other projects use), pins a new runtime under `%LOCALAPPDATA%\DaVinci\design-runtime`, and records it in DaVinci's global settings with `/config` → Design artifacts on. Restart DaVinci, choose an `openai-codex` model with thinking `low`, `medium`, `high` or `xhigh`, and run `/design new "a pricing page" --kind landing`. Run the script again after updating DaVinci: each run pins a new runtime and points the settings at it.

In the interactive shell you do not need to run it yourself: when the runtime is missing, `/design` asks whether to install it, runs this script on the terminal (DaVinci returns when it finishes), adopts the new runtime without a restart and then carries out the command you typed. When a runtime is installed but Design artifacts is off, it offers to turn it on instead. It will not run a script from a checkout inside the current workspace, where the session could have edited it: started in the DaVinci repository itself, it asks you to review the checkout and run the script from a separate terminal. Elsewhere (Linux, macOS, or a DaVinci built without a checkout) it says what to run.

The manual steps below are what the script does.

## Setup

Turn the feature on with `/config` → Design artifacts (`designEnabled` in the global `settings.json`) or `DAVINCI_DESIGN_ENABLED=1` in the launching process; the variable wins when both are set. A project's own settings cannot turn it on or choose its runtime. Generation requires the session's selected `openai-codex` OAuth subscription, model and thinking level (`low` to `xhigh`). A host that starts DaVinci with `--root-budget` accounts every design request to that root ledger, as described in the [root budget contract](readiness/openai-harness-implementation.md#budget-operation). Without one, each design operation gets a subscription-only ledger of its own (twelve requests and fifteen minutes for a generation, one request for an implementation draft), pinned to the session's model and effort and kept under `design/budgets` in the agent directory. Ordinary turns are never accounted to it. The stored ChatGPT login is renewed through its refresh token when it nears expiry, as every other DaVinci request renews it. There is no API-key fallback, implicit login, credential refresh or alternate model. Offline mode disables provider requests through CLI, RPC, SDK and the companion.

Listing, status, saved source and source export do not require Node or Chromium. Opening the companion, compiling, rendering and browser interactions require an explicitly prepared trusted runtime outside the application workspace:

1. Prepare Node **24.19.0** and the exact dependencies in `crates/davinci-coding-agent/design-ui/package-lock.json`. Dependency acquisition is a separate operator action; `/design` never installs packages.
2. Build the companion with `npm --prefix crates/davinci-coding-agent/design-ui run build`. Provide the Chromium revision expected by Playwright **1.62.1** in its normal cache, and installed local fonts. No browser/font download occurs during a design operation.
3. Run `node crates/davinci-coding-agent/design-ui/scripts/install-runtime.cjs <new-absolute-directory-outside-workspace>`. This explicit offline setup copies trusted code and records SHA-256 inventories of Node, packages, browser and fonts. It refuses an existing destination, missing dependencies, symlinks and incomplete inventories. A failed setup leaves its directory for inspection.
4. Set `DAVINCI_DESIGN_RUNTIME` to that directory and `DAVINCI_DESIGN_NODE` to the exact absolute Node executable, or store them as `designRuntime` and `designNode` in the global `settings.json`. Subsequent checks reject changed tooling, extra files, missing browser files, modified fonts and workspace-owned tooling. Runtime operations also refuse a workspace that contains your home directory (DaVinci started in `~` or above it): the runtime, its manifest, the browser cache and the settings that point at them all live under home, and a session that can write them could have its own code run as trusted tooling. Start DaVinci in a project folder to use `/design`. Rebuild and prepare a new runtime after changing the bundled UI or compiler.

The installer does not install DaVinci globally. A release installation and verification of the actual PATH-resolved executable are separate actions. Font inventories bind evidence to that machine; they do not establish pixel parity across operating systems. Explicit Linux setup resolves installed font links into bounded regular-file copies inside the pinned runtime and creates a private Fontconfig configuration. Runtime requests still reject links and verify the copied hashes. Recreate older Linux bundles with the setup command before rendering.

On Windows the runtime pins Chromium's headless shell, which has no crash reporter (Chrome's crash reporter cannot create its named pipe inside an AppContainer). The trusted host runs the pinned runtime's code in the supervisor's job with an explicit environment, and the browser that runs the generated content starts through DaVinci's launcher in an [AppContainer](sandbox.md#native-windows-appcontainer). The container has no network capability and receives a read capability for the pinned browser directory and a write capability for a per-launch temp directory where Playwright keeps its browser profile. Windows may additionally grant ambient ALL APPLICATION PACKAGES access to other paths; the [sandbox documentation](sandbox.md#native-windows-appcontainer) states this limit. `native_hostile_page_and_process_cleanup` checks every browser process's token on Windows: AppContainer, no network capability. It fails when the browser runs unconfined.

Linux rendering also requires a host that permits Bubblewrap's user, process and network namespaces. The native CI fixture uses Ubuntu 22.04 and checks this prerequisite before compilation. Ubuntu 24.04's [AppArmor namespace restrictions](https://documentation.ubuntu.com/security/security-features/privilege-restriction/apparmor/) require an appropriate administrator-managed profile; an installed `bwrap` alone does not prove the capability is usable. A namespace denial leaves rendering unavailable. DaVinci does not disable host protections or use an unrestricted browser fallback.

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

Applying uses DaVinci's existing patch transaction and preserves unrelated edits. It returns **applied / pending_verification**. The trusted SDK/RPC erify_implementation operation collects actual target build/test and managed-browser receipts. Its VerifyHandoff request binds the reviewed proposal hash, exact build/test command strings from its verification obligations, a timeout, an operation ID, and optional managed process/port/path plus click/type/select actions and DOM/accessibility assertions. Commands use the existing permission, hook, supervisor and execution-receipt owners; Node tests require the explicit TAP reporter to retain discovery counts. Unsupported discovery remains incomplete. Browser checks retain their screenshot in the design store before closing the browser. Source changes invalidate retries; an interrupted check requires an explicit new operation after inspecting command outcomes. Even passing measured checks leave implementation pending: production API integration, mock removal, RSC/client boundaries and transitive source coverage still require target-specific review. Design previews and synthetic fixture receipts never mark the target transaction Verified. There is no automatic commit, installation, deployment or upload.

## Limits and failure behavior

| Resource | Bound |
| --- | --- |
| Source | 64 files, 256 KiB each, 2 MiB total; portable paths, no case collisions |
| Concepts / artboards | 1–3 concepts, 8 artboards |
| Raster assets | PNG/JPEG/WebP only; 20 files, 10 MiB each, 16 million decoded pixels each |
| Evidence | 50 MiB per render run, 100 MiB cumulative per artifact, 500 MiB per session task store |
| Model run | At most 12 root-accounted attempts, two repair passes, a fifteen-minute deadline fixed when the operation first starts; stricter parent limits win |
| Implementation draft | One accounted request; 1–16 UI paths, 128 KiB target context, 512 KiB full context |
| Companion | 1 MiB requests, 256 KiB replies, 32 jobs, one active long operation; 8 pending bridge requests |
| Pairing / lease | One-time 60-second pairing capability; eight-hour host lease |

Host authority comes from live permission and task-contract checks, never browser payload fields. Cancellation and revocation stop work; the last committed revision remains readable. Unknown provider outcomes retain their budget reservation and block automatic repetition. A refused subscription request halts that operation's ledger and the run reports the provider's own message first, for example `Codex error: The ChatGPT user has reached their Subscription Sharing usage limit. ... (subscription-only receipt rejected: response failed (HTTP 200), model not reported, expected gpt-5.6-luna; campaign halted)`. Wait for the plan limit to reset, then start a new `/design new`; the halted operation is not retried. A compiler error can trigger a bounded repair; unavailable trusted tooling or confinement cannot be repaired by relaxing security.

Session events are authoritative. Immutable blobs are written before event publication; interrupted unreferenced blobs may be pruned under the session writer lock. Pruning retains references from all branches, accepted revisions and checkpoints. Missing or corrupt referenced data is an error and the evidence is retained for diagnosis. On first write after a torn final session record, its exact bytes are flushed to a sibling `.torn-<uuid>.bak` before truncation; a failed backup blocks recovery. A terminated corrupt record remains an error. A crash after target patch application uses the native transaction journal for recovery; do not blindly replay an approval.

## Rollback

Close the companion and unset `DAVINCI_DESIGN_ENABLED` (or set it to `0`) for the next launch. Ordinary tools and prompts remain unchanged. The disabled controller retains list/status/read and source export; it refuses new mutations and rendering. Do not delete the session WAL or artifact store to disable the feature. Existing accepted source, comments, revisions and evidence remain available for a later enabled session.
