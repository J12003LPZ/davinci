# Changelog

## Unreleased

### Changed
- MCP stdio servers no longer inherit the full parent environment. They receive a small platform-specific runtime allowlist, plus the `env` in their config. Pass secrets explicitly: `"env": { "GITHUB_TOKEN": "${GITHUB_TOKEN}" }`.
