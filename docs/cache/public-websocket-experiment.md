# Public Responses WebSocket experiment

Status: experimental, opt-in, off by default. Nothing here changes a default, a route or a header for existing users.

## What it tests

Every DaVinci request on the "Sign in with ChatGPT" route goes to `https://api.openai.com/v1/responses` as a separate
HTTPS POST that replays the whole conversation. Codex, on its own backend, keeps one WebSocket per turn and sends only
new items with `previous_response_id`. The 2026-10 efficiency diagnosis (WS1) found that DaVinci's later-request cache
reuse is 0.6 to 0.74 against 0.90 for Codex, that the gap is not prompt content and not the cache key, and listed the
transport as the open item. This branch makes the public route's WebSocket reachable so the question can be measured.

It separates two effects that are easy to confuse:

| `transport` setting | Socket | Request shape |
|---|---|---|
| `auto` (default, unchanged) | none, HTTPS POST per request | full input |
| `public-websocket` | one persistent socket per conversation | **full input**, no `previous_response_id` |
| `public-websocket-continuation` | the same socket | `previous_response_id` plus **only new items** when the previous request is an exact prefix of the next |

`public-websocket` against `auto` isolates the persistent connection. `public-websocket-continuation` against
`public-websocket` isolates continuation. No setting changes both relative to HTTP in one step without the middle arm.

## Is it supported on this route

Established from documentation, read 2026-10-15. Nothing below was tested against the live service.

| Question | Evidence | Status |
|---|---|---|
| Does the public Responses API have a WebSocket mode? | The WebSocket Mode guide describes one persistent connection to `/v1/responses` where each turn is a `response.create` event; the SDKs expose `responses.connect()` (https://developers.openai.com/api/docs/guides/websocket-mode). | documented |
| Does the sign-in route allow it? | The Sign in with ChatGPT preview-limitations page says "Don't use `previous_response_id` over HTTP" and that "WebSocket continuation can reference only responses from the same authenticated connection" and "doesn't provide persistent conversation storage". That treats WebSocket continuation as the supported way to continue on this route (https://developers.openai.com/siwc/token-sharing-open-source/preview-limitations). | implied, not a statement that the OAuth token is accepted at the handshake |
| Does it need different auth? | The guide names no WebSocket-specific header. The official Python SDK's `connect()` sends the normal bearer `Authorization` plus its default client headers and no `OpenAI-Beta` header (openai-python 3.31.0, `ResponsesConnectionManager._connect_ws`). This branch sends the same `Authorization` as the HTTP path and nothing from the ChatGPT backend (no account id, no beta flag, no originator). | documented by SDK source; live acceptance of the OAuth token unverified |
| Is `store: false` kept? | "WebSocket mode is compatible with both Zero Data Retention (ZDR) and `store=false`", previous-response state is "retained only in memory". With `store=false` there is no persisted fallback and an unknown id returns `previous_response_not_found`. The sign-in route requires `store: false`, so the privacy behaviour is preserved. | documented |
| Does the model work on it? | The guide gives no model list. `gpt-6-luna` is served on this route over HTTP. WebSocket support for that model is **not established**. | unknown, settled by the first warm-up |
| Do usage fields arrive? | The guide does not mention usage. The decoder reads `response.completed.response.usage` exactly as it does for SSE; the harness needs `cache_write_tokens` present for a complete receipt. | unknown, settled by the first warm-up (`usage_complete`) |
| Is `prompt_cache_key` accepted? | Not documented for WebSocket. It is part of the create body today and is sent unchanged. | unknown |

Not done, by design: no API key is used or accepted on this path, the route is never redirected to the ChatGPT
backend, and nothing assumes Codex's backend capabilities apply to the public route.

Request semantics that change with the transport, and how this branch handles each:

- **`stream` and `background`**: transport-only fields, removed from the create event. `type: "response.create"` is added.
- **`previous_response_id`**: never sent by `public-websocket`. Sent by the continuation mode only on the socket that
  produced the id; a new socket always starts from the full input and drops the stored id.
- **Input replay**: `store: false` plus `include: ["reasoning.encrypted_content"]` are unchanged. In the full-replay
  mode the encrypted reasoning items ride in `input` exactly as over HTTP. In continuation mode the server holds them
  in memory and they are not resent.
- **Session affinity and headers**: the HTTP request headers are reused, minus `content-type`, `accept`,
  `content-length` and `accept-encoding`. `prompt_cache_key` is the existing key.
- **Compaction or any edit to earlier input**: the previous request is no longer a prefix, so the next request is a
  full replay on the same socket (no `previous_response_id`).
- **Usage**: unchanged decoder; each attempt row records the usage of its own response.

## Failure behaviour

There is no silent fallback. The existing Codex WebSocket path downgrades to SSE after a failure; this path never does.

- Any connect, send or read failure is returned as `public WebSocket transport failed (no SSE fallback): ...`.
- `previous_response_not_found` and other provider errors surface as a failed response. The failed attempt is
  recorded, the socket is closed and the stored id is cleared, so the caller's own retry starts from the full input on
  a new socket. The transport performs no hidden retry.
- A cached socket the server has already closed is detected before sending and replaced, so an idle close costs no
  failed attempt. A 60-minute connection limit is handled the same way on the next request.
- A cancelled response closes its socket and clears the stored id.
- `PI_CODEX_WS_REPLY` (fixture replay) is refused on this path.
- Subscription-policy admission still requires HTTP/SSE, so the policy mode refuses these values.

Each attempt is labelled in the provider observation as `websocket` (full input) or `websocket-continuation`
(`previous_response_id` sent), so a run proves from its own rows which shape it used.

## Enabling it

`settings.json`: `"transport": "public-websocket"` or `"transport": "public-websocket-continuation"`. Only the
public sign-in route (`https://api.openai.com/v1`, provider `openai-codex`) honours these values; any other model
ignores them. `auto`, `sse`, `websocket` and `websocket-cached` behave exactly as before.

## Exact changes

- `crates/davinci-ai/src/codex_ws.rs`: `PublicWsMode`, `process_public_websocket`, `socket_is_quiet_and_open`,
  the attempt labels and the public URL. The Codex backend functions are untouched.
- `crates/davinci-ai/src/stream.rs`: one `if let Some(mode) = PublicWsMode::select(..)` branch placed after the HTTP
  headers are built and before the HTTP send, and `complete_public_websocket`.
- `crates/davinci-ai/src/codex_ws_public_tests.rs`: the offline suite below.

For every request that does not name one of the two new values, `select` returns `None` and the code path is the
one that shipped.

## Offline validation

A scripted loopback server plays the provider; no network and no credentials. 21 tests:

- selection: only the two explicit values on the public route; `auto`, `sse`, `websocket`, `websocket-cached`, empty and
  mis-cased values, the ChatGPT backend route and other providers never select it
- one socket for a conversation; full input in every request; no `previous_response_id`; no `stream` field; `store`
  and `prompt_cache_key` preserved
- continuation: only new items with the right id, across a tool call and its output; request 3 chains from response 2
- continuation never crosses connections; a server-closed idle socket is replaced before sending
- a changed prefix (compaction) or changed top-level field replays in full on the same socket
- `previous_response_not_found`: surfaced as a failed attempt, no hidden retry, next request starts full on a new socket
- closed before any event, closed mid-stream, failed response: explicit errors, socket and state dropped
- cancellation before connecting (no socket) and mid-response (socket closed, next request full)
- usage and cached tokens decoded from the terminal event; tool calls decode as over HTTP
- reasoning items replayed in full mode and carried by the server in continuation mode
- handshake carries the bearer token and no backend headers; fixture replay refused; the stream entry point errors
  with "no SSE fallback" and never reaches the HTTP path
- attempt rows carry the shape label and complete usage

Mutation checks: removing the `stream` strip, allowing continuation across sockets and removing the stale-socket
probe each fail the suite.

## What offline testing cannot show

These are decided by the first live warm-up and stop the experiment if they fail: the OAuth token is accepted at the
WebSocket handshake; `gpt-6-luna` is served over WebSocket; `response.completed` carries complete usage including
`cache_write_tokens`; `previous_response_id` with `store: false` is accepted on this route; later-request cache reuse
actually improves. The last is the experiment's question and is not assumed.
