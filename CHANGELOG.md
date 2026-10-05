# Changelog

## 1.1.1

Changes since `v1.1.0`. Found by live gpt-6-luna (medium) verification of the installed 1.1.0 on a Sign in with ChatGPT login.

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
