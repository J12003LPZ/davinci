# Phase 1 — Workspace and `pi-core`

## Goal

Stand up the Cargo workspace and the shared error/id/json types every later crate depends on.

## Deliverables

- Root `Cargo.toml` workspace listing every program crate.
- `crates/pi-core`: `SessionError`, `PiError`, JSON value helpers, id generation.
- `vendor/pi` TypeScript tree vendored as the spec.
- README describing dual-tree layout and how to run Rust vs TypeScript.

## Done when

`cargo test -p pi-core` passes and later crates can depend on `pi-core` without redefining session error codes (`storage`, `invalid_payload`, `not_found`).
