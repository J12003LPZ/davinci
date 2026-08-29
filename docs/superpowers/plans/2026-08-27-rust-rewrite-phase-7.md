# Phase 7 — Coding agent CLI

## Goal

Port the product CLI surface: print mode, session listing, and the four built-in tools.

## TypeScript sources

- `vendor/pi/packages/coding-agent/src/main.ts`
- `vendor/pi/packages/coding-agent/src/core/tools/`
- `vendor/pi/packages/coding-agent/src/modes/print-mode.ts`

## Deliverables

- Binary `pi` (`crates/pi-coding-agent`).
- `--help`, `--version`.
- Print mode (`-p` / `--print`) using `pi-agent` + mock or configured model.
- `sessions` lists SQLite or JSONL sessions.
- Tools: `read`, `write`, `edit`, `bash` with cwd-relative paths.

## Done when

CLI unit/integration tests cover help, print, RPC, interactive, tools, and session list. Rust `pi` is the default product.
