# Phase 7/8: Differential Conformance

Golden fixtures live in `crates/pi-parity/fixtures/` and are the shared contract between TypeScript packages and Rust crates. TypeScript under `vendor/pi` remains authoritative.

## Fixture families

1. **Writer-lease traces** — acquire / conflict / expire / fence takeover / renew / release.
2. **Protocol envelopes** — hello + CBOR frames.
3. **Assistant message + usage** — tool-call JSON shape.
4. **Session entries** — camelCase entry payloads.
5. **Agent-loop / CLI print events** — lifecycle type names.

Rust tests load these fixtures. TypeScript packages must serialize the same camelCase JSON.
