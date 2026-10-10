# Changelog

## Unreleased

### Added
- `/plugins`, `/skills` and `/mcp` are redesigned for reading. Tabs are one row with the open one as a filled pill, plus a live `n/total` position; the tabs and the rounded search box stay pinned while the list scrolls. Each entry is a name with `· status · source` after it on the same line, one clipped description under it, and a blank row between entries. The selected entry shows two description lines and its note. Plugins and MCP servers that fail, need authentication (401/403/login in the status or note) or need attention are badged on their row (`× error`, `! needs auth`) and counted at the right of the tab bar, including from Discover. Skills never show a problem.
- `/skills` is filed by source instead of one flat list of hundreds: your own skills first, then one heading per plugin (`▸ ecc@ecc · 293 skills`). A plugin with more than 8 skills starts folded and shows a one-line preview of what is inside; enter or space on a heading folds and unfolds it. `/` filters by name, description or plugin, opens every group it matches and says `n of total`; esc clears it.
- The context meter no longer sits flush against the last transcript line: one blank row now separates them.
- `/design` renders on Windows. The generated-content browser runs in a per-launch AppContainer with no network capability and disjoint pinned-browser read and private-temp write mounts; Windows may also grant ambient ALL APPLICATION PACKAGES access. Native capture, prototype-action and hostile-page tests pass, and the hostile test checks each browser process's token. A completed live subscription generation remains unverified.
- `scripts/setup-design-runtime.ps1`: one command prepares the pinned design runtime and turns `/design` on. `/config` gains a Design artifacts toggle; the runtime paths live in the global settings, and a project's own settings cannot set them.
- `windows_app_container` sandbox backend, chosen explicitly (never by `auto`, because confined processes cannot create the named pipes most tools use for subprocesses).
- `ask_user_question` asks 1-4 questions in one dialog, as Claude Code does. Each question is single-select (the default) or `multi_select`, with optional `min_selections`/`max_selections`; on multi-select a typed custom answer counts as one more selection. The dialog shows `Question N of M` with a ✓ per answered question; `↑↓` moves, `space` toggles a checkbox, `1-9` pick, `tab`/`shift+tab` (or `←→`) change question without losing answers, `enter` continues and then submits from a review page, `d` defers and `esc` cancels the whole dialog. All answers are validated and written to the Living Plan in one revision, or none are; the tool result lists every answer with option ids and labels. The flat single-question input still works, RPC clients and the hosted TUI get one select at a time, and old sessions load unchanged.

### Fixed
- Design runs no longer need `--root-budget`. Each operation gets a subscription-only ledger of its own (12 requests and 15 minutes for a generation). A host-provided root budget still wins.
- Design requests renew the ChatGPT login when it nears expiry, use SSE as subscription admission requires, and give the model the exact binding schema.
- A rejected source tool call or binding now goes to a repair pass instead of ending the run, and its message names the rule that failed.
- An HTML design entry that loads `./src/App.jsx` or TypeScript the dev-server way is bundled. Previously the browser refused the raw JSX as `text/plain`.
- An image counts as at most 3,000 tokens in context accounting, not its base64 size. A pasted screenshot no longer looks like a million tokens.
- A host model request that fails before sending reports the provider's error, not "did not produce settled root admission evidence".
- A subscription request the provider refuses (a ChatGPT plan usage limit, for one) reports the provider's message, then the response status, HTTP code and returned model. Previously it said only "unsuccessful or unidentified response".
- Two starts of one design operation agree on one budget deadline. The limits file is linked into place, so it is never overwritten or read half-written.

## 1.1.6

Changes since `v1.1.5`.

### Added
- Installed skills and plugin commands are slash commands, named as in Claude Code: `/<skill>` for your own, `/<plugin>:<name>` for a plugin's (`/superpowers:writing-plans`), listed with their descriptions when you type `/`. An install from `/plugins` (manager or typed `/plugins install …`) loads them at once; no `/reload`. Built-in and native commands keep their names; `/skill:<name>` still works.
- `alt+v` pastes a clipboard image as an `[Image #N]` chip in the draft, with a count under the input; deleting the chip or clearing the draft drops the image. On native Windows the clipboard is now read at all (screenshots and image files copied in Explorer); before, only WSL, Wayland, X11 and macOS were.
- The context bar above the input: used of the window, split by category, with a tick at the configured auto-compact point and a warning as it nears. `/config` → Context bar: off, compact (default) or full.
- ChatGPT plan usage for openai-codex models under the context bar: the 5-hour window (Plus, Pro), the weekly one, or whatever single window a plan reports (a free plan's 30-day one), with what is left and when each resets, read from the Codex CLI's `codex app-server` (`account/rateLimits/read` plus `account/rateLimits/updated`). `/config` → Plan usage.
- The goal path: the model's task list pinned under the working line while it works, numbered, with the active task spinning and finished ones checked and struck through. The `todo` tool now asks for a list on work of three or more steps, as Claude Code's does.

### Fixed
- Two tests that depended on scheduler timing (`file_mutation_queue` ordering, background subagent completion) wait on the event instead.

## 1.1.5

Changes since `v1.1.4`. Tagged `v1.1.5` at `3a0b118c`.

### Added
- `/skills`, `/plugins` and `/mcp`, one manager each, like Claude Code. Discover is a global marketplace you browse without adding anything: Anthropic's official plugin directory (superpowers, context7, github, playwright and 300 more) and Agent Skills repository are added automatically the first time and refreshed daily, and `/mcp` opens on featured MCP Registry servers (Context7, GitHub, Playwright, Notion, …) followed by recently updated ones. Private marketplaces can still be added. Installed lists what DaVinci loads with update, enable/disable, hook approval and delete. Discover is a search bar: plugins from your marketplaces, skills from your marketplaces and skills.sh, MCP servers from the official MCP Registry; Enter twice installs. `/plugins` also adds, refreshes and removes marketplaces. `/skills pdf` opens Discover with the search typed in. An MCP server's secrets come from variables named for it (`DAVINCI_MCP_<SERVER>__<NAME>`), so a registry entry cannot ask for a host secret such as `AWS_SECRET_ACCESS_KEY`; the result row shows what it runs or contacts before you confirm.
- Escape twice clears a draft in the composer, as in Claude Code; `↑` brings it back. The first Escape says "esc again to clear".

### Removed
- `/plugin`, `/skill-list` and `/skill-view`. Typing one says where its job went: `/plugins`, `/skills` or `/mcp`. `/plugins install …` and the other `davinci plugin` subcommands still work as text.

## 1.1.4

Changes since `v1.1.3`. Tagged `v1.1.4` at `deb4e0f1`.

### Fixed
- Codemode turned on in `/config` stayed off with "Workspace-provided Node runtime is not admitted" when DaVinci ran from the home directory: the workspace then contains `~/.davinci/agent/codemode`, and DaVinci's own runtime was refused as project-provided. A runtime under `<agent dir>/codemode` is now admitted wherever the workspace is; a Node binary or host elsewhere in the workspace is still refused, and the error names both paths.

## 1.1.3

Changes since `v1.1.2`. Tagged `v1.1.3` at `7c554f40`.

### Added
- A system prompt for each GPT-6 model. Sol and 6.1 Sol look for the root cause before patching; Luna works one focused job at a time; Astra keeps its lean policy. All three share GPT-6 rules for tools, delegation and answer style, which replace the generic OpenAI adapter. See [docs/prompt-engineering.md](docs/prompt-engineering.md#gpt-6-family-policies).
- Luna can hand hard work to a GPT-6 Sol worker. When a task needs multi-step debugging or a check keeps failing, Luna asks first (Sol worker, continue with Luna, or switch models). DaVinci then asks you to approve any worker on a model other than the session's, in every mode except Always Approve, including a worker whose agent profile names another model; "allow for this session" covers that model only.
- Leaving Plan Mode tells the model how to carry out the approved plan: steps in order, each step's checks, progress in `update_plan`, and a proposed revision instead of a silent change.
- `davinci-evals` regression suites and A/B model policies for `gpt6-sol` and `gpt6-luna`.
- **Vox** theme: an editorial collage palette (ink ground, oxblood and navy layers, cream text, mustard focus, cyan and magenta accents). Pick it in `/config` → Theme or at first-time setup; it applies at once and is saved as `"theme": "vox"`. Every text color clears 4.5:1 contrast on every surface, in truecolor and 256 colors. Tokens are in [docs/ui/design.md §2](docs/ui/design.md).

### Removed
- `/act`, which duplicated Shift+Tab: Shift+Tab leaves Plan Mode without approving the plan. `/plan` stays for plan review and approval (`show`, `diff`, `edit`, `reject`, `approve`, `accept [mode]`).
- `/thinking`, which duplicated `/effort`. `/effort <level>` is the one reasoning command.
- `/fast`. Fast applied only to the OpenAI Codex route; request it with `"serviceTier": "fast"` in user settings (or `DAVINCI_OPENAI_SERVICE_TIER=fast`). The status label and downgrade notice still report the tier.

### Fixed
- Enter in the composer no longer swaps a typed command for a different one that merely contains it: `/act` + Enter ran `/compact`. Enter takes a suggestion only when its name begins with what you typed or you chose it with the arrows.
- A removed command typed with arguments (`/thinking low`, `/fast on`) is answered locally with where its job went, instead of being sent to the model as a prompt.
- `ask_user_question` works as advertised. Its schema offered `kind` values (`approach`, `tradeoff`, ...) that the validator rejected, and outside Plan Mode every evidence reference failed as "Unknown plan evidence". The schema now lists the accepted kinds, and a question may cite workspace files it read; they are re-checked when you answer, under the same path rules as plan evidence.
- Behavior evals no longer fail every run for "unrelated files changed" in `.davinci-transactions/`, the edit journal DaVinci keeps outside git repositories.
- A worker's `model` override is no longer dropped when the catalog does not list that model: `gpt-6-sol` from a Luna session ran on Luna. Unlisted ids on a known provider now run as asked, a bare id uses the session's provider, and an unknown provider fails the call.
- Choosing `vox` in `/config` or first-time setup rendered the dark theme. The name had been aliased to dark while still being offered.
- Codemode admission errors name the path they checked (`Codemode Node runtime unavailable: no Node 24.21.0 at <path>`). Without it, a runtime installed in another agent directory (`~/.pi/agent` while DaVinci used `~/.davinci/agent`) failed with no clue where DaVinci looked. The docs' install example now picks the active agent directory.

## 1.1.2

Changes since `v1.1.1`. Tagged `v1.1.2` at `5b7ba537`.

### Added
- `/config` has a **Codemode** switch. On, the model can run sandboxed read-only JavaScript over your tools from the next prompt; off removes the tool. Without explicit `nodePath`/`hostPath`, the runtime is expected in `<agent dir>/codemode` (Node 24.21.0 in `node/`, the host bundle in `host/`). See [docs/codemode.md](docs/codemode.md).

### Changed
- `/config` is the only way to open the settings panel; `/settings` is no longer a command.
- Update pulldown-cmark to 0.13.4, regex to 1.13.1, image to 0.25.10, unicode-width to 0.2.2 and webpki-roots to 1.0.9; CI actions setup-node and upload-artifact to v7.
- Codemode enabled through settings no longer stops startup when its runtime is missing or the run has no session; DaVinci starts without it and says why. `--codemode read-only` still fails in those cases.

### Fixed
- The live governor, session and graph evals now write a rotated ChatGPT login back to your auth store (owner-only), instead of leaving it with a dead refresh token.

## 1.1.1

Changes since `v1.1.0`. Tagged `v1.1.1` at `027c3d8b`. Found by live gpt-6-luna (medium) verification of the installed 1.1.0 on a Sign in with ChatGPT login.

### Fixed
- A Codemode script stopped by its own sandbox deadline now reports `TIMEOUT`, and an aborted one `CANCELLED`. Both were reported as `SANDBOX_FAILED` ("script failed in the sandbox") whenever the sandbox deadline fired before the host watchdog.
- `/compact` and automatic compaction now summarize with the session's own model. A model missing from the catalog, such as `gpt-6-luna`, fell back to the provider's first record, `gpt-5.3-codex-spark`, which the ChatGPT-plan route refuses, so every compaction failed.
- Once the live Codex model list is available, built-in `openai-codex` models it does not offer (`gpt-5.3-codex-spark`, `gpt-5.4`, `gpt-5.4-mini` today) are no longer listed or selectable. The ChatGPT-plan route refused them with "not supported when using Codex with a ChatGPT account".
- Codemode can be enabled from the CLI again. `--codemode read-only` or `codemode` settings failed at startup with "Codemode requires the parent operation journal", because the journal is attached per turn. The admitted host is now registered on each turn's runtime. With `--no-session` (no journal), startup says so.

## 1.1.0

Changes since `v1.0.71`. Tagged `v1.1.0` at `4d25406a`.

### Upgrade notes
- **Sign in again with ChatGPT.** The `openai-codex` provider now uses Sign in with ChatGPT and the public Responses route. Logins created by 1.0.71 or earlier, and API keys, no longer work for it. Run `davinci`, then `/login openai-codex`, once after upgrading.

### Security
- Remote model catalogs can no longer redirect requests or credentials. A catalog entry keeps the endpoint, API and headers of the built-in entry for its provider; entries for another provider, or with an endpoint no built-in entry of that provider uses, are dropped. Already-cached `models-store.json` entries are hardened the same way.
- Auto mode no longer runs `cargo test`, `check`, `build` or `clippy` without asking unless an OS execution sandbox is active. Auto can edit `build.rs` or a test, so an unconfined build was arbitrary code execution.
- With no home directory (`HOME`/`USERPROFILE`) and no `DAVINCI_CODING_AGENT_DIR`, credential storage now fails instead of writing `./.davinci/agent/auth.json` into the working directory.

### Privacy
- Removed the install ping to `https://pi.dev/api/report-install`. DaVinci sends no install or usage telemetry.
- Provider attribution headers (OpenRouter, NVIDIA NIM, Cloudflare) are now opt-in: `enableInstallTelemetry: true` or `PI_TELEMETRY=1`. Subagents follow the same setting.
- The Radius gateway config (`https://radius.pi.dev/v1/config`) is fetched only for a configured Radius account and never while offline. A fetched config is kept for the process; a failed fetch is retried after 60 seconds, not on every catalog load. Logging in to Radius mid-session takes effect without a restart.
- `/share` prints the secret gist URL and no longer defaults to a `https://pi.dev/session/` viewer link. Set `PI_SHARE_VIEWER_URL` to get one.

### Fixed
- A stored openai-codex login that predates Sign in with ChatGPT is now named as the cause, with `/login openai-codex` as the fix. Previously `--list-models` reported no models, a prompt failed with "No model matched", and `/model` said there was no credential.
- A panic while holding workflow or learning state no longer poisons the lock for every later call.

### Removed
- Microphone voice input and the `davinci-voice` crate (#93).
- TypeSafe/Jev decision intelligence (#91).

### Changed
- Require Rust 1.88.0 (previously 1.83.0).
- Upgrade ratatui to 0.30.0, sha2 to 0.11.0, rusqlite to 0.40.2, tempfile to 3.27.0, and crossterm to 0.29.0. The ratatui upgrade removes the unmaintained paste dependency (RUSTSEC-2024-0436).
- MCP stdio servers no longer inherit the full parent environment. They receive a small platform-specific runtime allowlist, plus the `env` in their config. Pass secrets explicitly: `"env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" }`.
- OpenAI Codex Fast mode (#86) and ChatGPT-plan Responses Fast (#88); Codex subscription efficiency and usage reporting (#84); durable OpenAI harness budgets (#82); opt-in design artifacts (#85).
- CLI export rejects missing sessions and invalid modes (#90); delegation authority survives compaction and graph launches (#80); tool-call resource-claim conflicts fixed (#78).

### CI
- All GitHub Actions are pinned to commit SHAs. Dependabot tracks actions and crates.
- New `Dependency audit` workflow runs `cargo deny check advisories sources` on every push, pull request and weekly.
- Removed workflows bound to finished feature branches, including five with `contents: write`, and their one-off scripts and patches.
- The live behavioral A/B workflow is manual only; its weekly schedule could only fail without provider secrets.
