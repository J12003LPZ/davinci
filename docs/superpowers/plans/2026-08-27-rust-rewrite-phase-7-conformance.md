# Phase 7: Differential Conformance

## Overview

Golden fixtures live in `crates/pi-conformance/fixtures/` and are the shared contract between TypeScript packages and Rust crates.

## Fixture families

1. **CBOR RFC 8949 vectors** — hex encode/decode parity.
2. **Writer-lease traces** — acquire / conflict / expire / fence takeover / renew / release.
3. **Protocol envelopes** — hello, commands, responses, errors (strict objects, no extra fields).
4. **Agent-loop transcripts** — tool order, length-stop fail-all, abort.
5. **Session repository traces** — create/list/open/fork/delete + sequence assignment.

Rust tests load these fixtures. TypeScript packages must serialize the same camelCase JSON.
