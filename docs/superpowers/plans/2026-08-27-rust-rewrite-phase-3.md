# Phase 3 — Protocol, client, server

## Goal

Port the framed CBOR protocol and the in-memory client/server loop.

## TypeScript sources

- `vendor/pi/packages/protocol/`
- `vendor/pi/packages/client/`
- `vendor/pi/packages/server/`

## Deliverables

- `pi-protocol`: RFC 8949 subset codec, 4-byte big-endian frames, `PROTOCOL_VERSION = 1`, command/event schemas.
- Known CBOR vectors from `packages/protocol/test/cbor/cbor.test.ts`.
- Frame decoder: incremental, fail-once, truncate-on-end.
- `pi-server`: `PiServer`, handshake (5s default), request dispatch, session attach, in-memory `PiServerService`.
- `pi-client`: `PiClient`, request correlation by id, exclusive/shared session leases, snapshot revision monotonicity.

## Done when

Protocol vector tests and an in-memory client/server loopback (`hello` → `create` → `prompt` → `detach`) pass.
