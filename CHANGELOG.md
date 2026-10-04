# Changelog

## Unreleased

Changes on `main` since `v1.0.71`.

### Security
- Remote model catalogs can no longer redirect requests or credentials. A catalog entry keeps the endpoint, API and headers of the built-in entry for its provider; entries for another provider, or with an endpoint no built-in entry of that provider uses, are dropped. Already-cached `models-store.json` entries are hardened the same way.
- Auto mode no longer runs `cargo test`, `check`, `build` or `clippy` without asking unless an OS execution sandbox is active. Auto can edit `build.rs` or a test, so an unconfined build was arbitrary code execution.
- With no home directory (`HOME`/`USERPROFILE`) and no `DAVINCI_CODING_AGENT_DIR`, credential storage now fails instead of writing `./.davinci/agent/auth.json` into the working directory.

### Privacy
- Removed the install ping to `https://pi.dev/api/report-install`. DaVinci sends no install or usage telemetry.
- Provider attribution headers (OpenRouter, NVIDIA NIM, Cloudflare) are now opt-in: `enableInstallTelemetry: true` or `PI_TELEMETRY=1`.
- The Radius gateway config (`https://radius.pi.dev/v1/config`) is fetched only for a configured Radius account, never while offline, and at most once per process. It was previously requested on every built-in catalog load.
- `/share` prints the secret gist URL and no longer defaults to a `https://pi.dev/session/` viewer link. Set `PI_SHARE_VIEWER_URL` to get one.

### Fixed
- A panic while holding workflow or learning state no longer poisons the lock for every later call.

### Removed
- Microphone voice input and the `davinci-voice` crate (#93).
- TypeSafe/Jev decision intelligence (#91).

### Changed
- MCP stdio servers no longer inherit the full parent environment. They receive a small platform-specific runtime allowlist, plus the `env` in their config. Pass secrets explicitly: `"env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" }`.
- OpenAI Codex Fast mode (#86) and ChatGPT-plan Responses Fast (#88); Codex subscription efficiency and usage reporting (#84); durable OpenAI harness budgets (#82); opt-in design artifacts (#85).
- CLI export rejects missing sessions and invalid modes (#90); delegation authority survives compaction and graph launches (#80); tool-call resource-claim conflicts fixed (#78).

### CI
- All GitHub Actions are pinned to commit SHAs. Dependabot tracks actions and crates.
- New `Dependency audit` workflow runs `cargo deny check advisories sources` on every push, pull request and weekly.
- Removed workflows bound to finished feature branches, including five with `contents: write`, and their one-off scripts and patches.
- The live behavioral A/B workflow is manual only; its weekly schedule could only fail without provider secrets.
