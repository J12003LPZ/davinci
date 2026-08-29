# Pi Monorepo: TypeScript to Rust Migration Program

## Executive Summary

This program governs the phase-gated migration of the Pi agent harness from TypeScript to idiomatic Rust. The TypeScript tree under `packages/` is the authority for wire formats, session semantics, and golden fixtures until the Phase 8 gate is proven. Rust crates under `crates/` must match those contracts, not invent a parallel product.

Authority source for contracts: the earendil-works/pi TypeScript implementation (packages `telemetry`, `ai`, `agent`, `protocol`, `client`, `server`, `session-backends/sqlite-node`, `tui`, `coding-agent`).

## Phase Architecture

- **Phase 0 — Workspace.** Cargo workspace, TypeScript reference packages, shared fixture directory.
- **Phase 1 — `pi-core`.** Shared types, CBOR subset, length-prefixed framing, protocol messages, session error codes.
- **Phase 2 — `pi-session-sqlite`.** Official schema, fenced writer leases, repository create/open/list/delete/fork, entry/lane/fact storage, conformance cases.
- **Phase 3 — `pi-ai`.** Message/content model, stream events that never throw, faux provider, tool-argument validation.
- **Phase 4 — `pi-agent`.** Agent loop, sequential/parallel tools, length-stop fail-all, steering/follow-up queues.
- **Phase 5 — `pi-client` / `pi-server`.** Hello handshake, framed CBOR commands, exclusive/shared session leases, in-memory transport.
- **Phase 6 — `pi-coding-agent` / `pi-tui`.** Print-mode CLI, built-in tools, JSONL session subset, differential TUI renderer.
- **Phase 7 — Differential fixtures.** Golden CBOR vectors, writer-lease traces, protocol envelopes, agent-loop transcripts.
- **Phase 8 — Gate.** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --check`. TypeScript remains the product authority.

## End State

The Rust port is complete for the program's defined slices: session-sqlite writer leases with fence takeover, protocol/client/server handshake and commands, pi-ai stream protocol, pi-agent loop, coding-agent print path and TUI differential render. TypeScript remains authoritative until a later product cutover that is *not* this gate.

## Authority Rule

TypeScript remains authoritative for protocol schemas and golden fixtures until Phase 8 gate criteria are completely verified. Rust must not redefine success around a simpler session or RPC model.
