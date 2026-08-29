# Phase 5: pi-client and pi-server

## Overview

Port framed CBOR protocol (`PROTOCOL_VERSION = 1`), client leases, and server handshake.

## Handshake

Client first message is `hello`. Server replies `hello` + snapshot or `hello_error` and closes. Unsupported version → `hello_error.version`. Request before hello → `hello_error.invalid_request`. Handshake timeout default 5s. Auth is transport-layer only.

## Commands

`list`, `create`, `attach`, `detach`, `prompt`, `steer`, `abort`, `set_model`, `set_thinking`.

## Client leases

`create` is exclusive. `attach` is shared. Exclusive fails if any lease exists; shared fails if exclusive exists. Detach is sent only when the last lease releases.

## Snapshots

Authoritative. Progress events do not mutate snapshot state. Ignore revisions older than current.
