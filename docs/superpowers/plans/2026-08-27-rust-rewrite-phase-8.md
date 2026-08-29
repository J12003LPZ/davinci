# Phase 8 — Differential fixtures and quality gate

## Goal

Lock cross-crate JSON/CBOR shapes against TypeScript fixtures and run the program quality gate. TypeScript stays authoritative.

## Deliverables

- `pi-parity` crate with golden fixtures for:
  - session entries / writer-lease rows
  - protocol hello + command envelopes
  - assistant message + usage
  - agent events
  - CLI print JSON events
- Scripts documented in the root README.
- `cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`.

## End-state check

All twelve items in `2026-08-27-rust-rewrite-program.md` § End state are true. Cutover is **not** performed.
