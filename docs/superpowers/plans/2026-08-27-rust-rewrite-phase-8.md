# Phase 8 — Differential fixtures, quality gate, and cutover

## Goal

Lock cross-crate JSON/CBOR shapes against TypeScript fixtures, keep the quality gate green, and ship Rust `pi` as the default product.

## Deliverables

- `pi-parity` crate with golden fixtures for:
  - session entries / writer-lease rows
  - protocol hello + command envelopes
  - assistant message + usage
  - agent events
  - CLI print JSON events
- Root README, `Makefile`, and `scripts/install.sh` install the Rust binary.
- `cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`.

## End-state check

All twelve items in `2026-08-27-rust-rewrite-program.md` § End state are true. Cutover **has been performed**: `cargo install --path crates/pi-coding-agent` is the product path. `vendor/pi` is `legacy-pi`.
