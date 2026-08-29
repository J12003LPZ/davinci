# pi Rust rewrite progress

**Complete: 100%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize.
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + published 0.84.4 catalogs, auth.json / env / OAuth storage, SSE stream lifecycle + usage/cost, fixture replay only.
- **pi-agent**: compaction, skills, prompt templates, AGENTS.md/CLAUDE.md, extension tool names, retry flags, steer/follow-up queues, built-in read/write/edit/bash/grep/find/ls.
- **pi-tui**: `Component::render(width)`, fullscreen alt-buffer, editor, keybindings, markdown, fuzzy selectors, themes.
- **pi-client / pi-server**: Unix + TCP + memory transports, handshake timeout, request correlation, leases via sqlite backend.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness (no live models).
- **pi-coding-agent `pi`**: every flag/subcommand from `args.ts` / `main.ts` (print, json, rpc, interactive, auth, install/remove/update/list/config, export), slash commands, RPC commands, settings/trust.
- **pi-parity**: required corpora (writer-leases, session entries, protocol hello/CBOR, assistant+usage, agent events, print/RPC) and optional `--parallel-run` / `--diff-jsonl`.

## Remaining product gaps

None. TypeScript remains in `vendor/pi` as reference only.

## Next crate/module

None — Done bar is met. Further work is upstream-tracking as `vendor/pi` moves.
