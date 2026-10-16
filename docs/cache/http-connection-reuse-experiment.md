# HTTP connection reuse experiment

Status: experimental, opt-in, off by default. Nothing here changes a default, a route, a header or a request body.

## Why

The 2026-10 HTTP versus WebSocket A/B (`docs/cache/public-websocket-experiment.md`) found that on the public sign-in
route later-request cache reuse was 0.56 to 0.67 per task over HTTP and 0.84 to 0.96 over one persistent WebSocket,
with the same binary, prompts and cache key. Per request, only 13% of later HTTP requests reached a 0.85 cache ratio
against 83% (persistent socket) and 93% (continuation). Most HTTP requests hit part of the prompt, not none of it.

`crates/davinci-ai/src/stream_http.rs` builds a new HTTP agent with `max_idle_connections(0)` for every provider
request, so every request opens a new TCP and TLS connection. The working hypothesis is that a new connection per
request lands on a backend that has not seen the previous turn. That is a hypothesis: the data fits it and does not
separate "same connection" from "same backend" or from the WebSocket protocol itself. This change makes the question
testable without WebSocket.

## What it does

`settings.json`: `"transport": "http-reuse"`. Only the public sign-in route (`https://api.openai.com/v1`, provider
`openai-codex`) honours it; any other model, and any other value, behaves exactly as before.

| `transport` | Connection |
|---|---|
| `auto` (default, unchanged) | new TCP and TLS connection for every request |
| `http-reuse` | one pooled connection per conversation (`session_id`), kept between requests |

Request bodies, headers, retries, budgets and decoding are unchanged. Only the connection is kept.

### Safety properties kept from the one-shot path

- **A cancellation belongs to one request.** Each request installs its own cancellation in its conversation's
  session before it is sent, and a transport that ever failed a check is poisoned permanently, so an aborted
  response can never hand its socket to a later request.
- **One request at a time per conversation.** A second concurrent request gets its own one-shot connection and
  never shares or evicts the session's connection.
- **A connection is reused only after a clean end.** The consumer stops at the stream's terminal event, before the
  chunk terminator arrives. A stream that ended on its own is marked finished, and the reader then drains the tail
  of the body (at most 64 KiB and 2 s) so the connection can return to the pool. A cancel, an abort or an error
  leaves the body unread and the connection is discarded.
- **Stale connections are probed away.** The pool checks a connection before reuse and drops idle ones older than
  55 seconds. A server that closed the connection costs a new connection, not an error.
- **No cross-conversation sharing.** Sessions are keyed by conversation id (and by whether a budget is active).
  A request with no conversation id keeps the one-shot path. At most 64 sessions are kept.

### Attempt labels

The provider attempt label states what the request expected, so a run proves from its own rows what it did:

- `http-reuse`: the previous response on this conversation ended cleanly less than 55 s ago, so a pooled connection
  should carry the request.
- `http-reuse-open`: a new connection is expected (first request, after an error or cancel, after an idle gap, or
  a concurrent request).

The label is an expectation made before the send. It is not a measurement of the connection. Whether connections were
actually reused is verified offline by loopback tests; live, it is read from the cache metrics. This is a limit of the
experiment, stated in its protocol.

## Exact changes

- `crates/davinci-ai/src/stream_http_reuse.rs` (new): session map, lease, `plan`, `LeasedReader`, transport that
  checks the current request's cancellation, and the loopback tests in `stream_http_reuse_tests.rs`.
- `crates/davinci-ai/src/stream_http.rs`: `send` is unchanged in behaviour; its body moved into `execute`, and
  `send_reusing` is the new entry. `Cancellation` gained a `finished` flag. `check` is unchanged for every existing
  caller; only the reuse transport calls `check_unless_finished`.
- `crates/davinci-ai/src/stream.rs`: the attempt is started with the plan's label and sent with `send_reusing`; a
  stream that ends on its own is marked finished. For every request that does not select `http-reuse` the plan is the
  one-shot path labelled `http`.
- `crates/davinci-ai/src/stream_reader.rs`: `ResponseLines::mark_finished`.

## Offline validation

A scripted loopback HTTP/1.1 server counts the connections it accepts. 16 tests:

- selection: only the public route and the exact value
- four requests of one conversation use one connection, with labels `open, reuse, reuse, reuse`
- no conversation id: one connection per request, as before; the default one-shot path never pools
- an abort mid-response discards the connection and the next request opens a new one
- an abort raised after a response finished does not poison the next request
- a concurrent request gets its own connection and leaves the session alone
- a server that closes after a response, and `Connection: close`, cost a new connection and no error
- an error status with a body does not waste the connection
- conversations never share a connection
- a failed send releases the session
- a cancelled transport stays unusable for a later request
- the drain: a finished response drains its tail, an unfinished one never does, and the drain is bounded

Mutation checks that each fail the suite: no tail drain, no poisoning, always-clean release, and letting a finished
flag excuse a cancel (which also broke an existing default-path test and was fixed before this commit).

## What offline testing cannot show

- That the server keeps the connection open and routes the next request to the same backend. The live experiment
  decides whether cache reuse improves.
- The wiring in `stream.rs` has no offline end-to-end test, because the public route cannot be pointed at loopback
  through the real entry point. The first live reuse run checks it through its attempt labels.
- The idle probe on TLS can miss a close already buffered by rustls; the request then fails explicitly and the
  existing provider retry opens a new connection.
