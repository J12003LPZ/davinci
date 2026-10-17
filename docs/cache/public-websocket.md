# Public Responses WebSocket transport

Status: the default on the "Sign in with ChatGPT" route (`https://api.openai.com/v1`, provider `openai-codex`).
Set `"transport": "sse"` (or `"http"`) to force plain HTTP. Every other route and provider is unchanged. A failure
before any output falls back to HTTP for that request, so a network that blocks WebSocket costs one failed connect.

## What it is

On the "Sign in with ChatGPT" route, DaVinci sends every model request to `https://api.openai.com/v1/responses` as a
separate HTTPS POST that replays the whole conversation. With `"transport": "public-websocket"` the same requests go
over one persistent WebSocket per conversation to `wss://api.openai.com/v1/responses`. Each request still carries the
full `input` and never a `previous_response_id`, so no request depends on server-side state of an earlier one. Request
bodies, headers (minus the ones that only describe an HTTP body), retries, budgets and decoding are unchanged.

| `transport` | Connection |
|---|---|
| unset, `auto`, `public-websocket` (default) | one WebSocket per conversation, full input replayed on every request |
| `sse`, `http` and every other value | a new HTTPS POST per request, unchanged |

Only the public sign-in route (`https://api.openai.com/v1`, provider `openai-codex`) honours the value. The ChatGPT
backend route and every other provider ignore it. No API key is used on this path, and nothing assumes that Codex's
own backend capabilities apply to the public route.

## Why

The 2026-10 confirmation study (`docs/perf/harness-gap/ws-confirmation-study.md` on the harness branch) compared HTTP,
this transport and a continuation variant on four public task families, three repetitions each, one frozen binary
and one model (`gpt-6-luna`, high effort), 36 measured runs.

| 12 runs per arm | HTTP | `public-websocket` |
|---|---|---|
| Verified success | 12/12 | 12/12 |
| Uncached input tokens | 1,331,470 | 324,645 (0.24x) |
| Requests | 227 | 217 (0.96x) |
| Wall time | 1,938 s | 1,587 s (0.82x) |

Uncached input was lower in 12 of 12 paired blocks and on every task. The continuation variant (`previous_response_id`
with new items only) was a further 15% lower in uncached input and 8% faster, which missed the registered bar for
adopting its extra state, so it is not shipped. The gain comes from the persistent connection; an earlier experiment
showed that reusing one HTTP connection did not produce it.

Limits: four public task families on one model; repetitions of one task are not independent samples. This supports
production hardening and independent holdout validation, not a default change and not a claim about other tasks.

## Failure behaviour

A failure is classified by whether any part of the response reached the caller.

| Situation | Behaviour |
|---|---|
| Cached socket closed by the server while idle | detected before sending (TCP peek and TLS state) and replaced; no failed attempt |
| Reused socket dies after the request is sent, nothing delivered | one retry on a new socket; the dead attempt is recorded as `failed` |
| Connect error, refused upgrade, or a fresh socket that fails before any output | the request falls back to HTTP, and the conversation stays on HTTP for 5 minutes |
| Failure after output began | returned as an error; never replayed over HTTP |
| `response.failed` or another provider error | surfaced as a failed response; the socket is closed |
| Cancellation | the socket is closed; the next request opens a new one |
| Busy cached socket (two requests at once) | the second runs on a temporary socket that is closed afterwards |
| No conversation id (summaries, titles) | a private id; its socket is closed right after |
| `PI_CODEX_WS_REPLY` (fixture replay) | refused, and never hidden by a fallback |

A fallback is visible in the provider observation: the failed `websocket` attempt is followed by an `http` attempt, so a
benchmark run shows a fallback instead of counting it as WebSocket. Sockets are dropped after 5 minutes idle or 55
minutes of age (the service limit is 60).

The HTTP replay after a fallback can, in theory, generate a response the server already started. No tool has run by
then: tools execute on the client after the stream ends. The cost is at most a duplicated model request.

## Enabling it

Nothing to set. To opt out, `settings.json`: `"transport": "sse"`. Subscription-policy admission requires an explicit
`sse`, so policy mode is unaffected and refuses every other value.

## Exact changes

- `crates/davinci-ai/src/codex_ws.rs`: `public_websocket_selected`, `process_public_websocket`, the typed
  `PublicWsFailure`, the dead-socket retry, the TLS-aware `socket_is_quiet_and_open`, and the per-conversation backoff.
  The Codex backend functions are untouched.
- `crates/davinci-ai/src/stream.rs`: one branch after the HTTP headers are built, `public_ws_headers`, `public_ws_next`
  (return or fall back) and `complete_public_websocket`.
- `crates/davinci-ai/src/codex_ws_public_tests.rs`: the offline suite below.

## Offline validation

A scripted loopback server plays the provider; no network and no credentials.

- selection: only the exact value on the public route; `auto`, `sse`, `websocket`, `websocket-cached`, mis-cased and
  the continuation value never select it; the ChatGPT backend route and other providers never select it
- one socket for a conversation; full input in every request; `previous_response_id`, `stream` and `background`
  removed; `store` and `prompt_cache_key` preserved
- a socket closed while idle is replaced before sending with no failed attempt
- a reused socket that dies after the request is retried once on a new socket, and the retry is bounded to one
- fallback classification: refused connection, refused upgrade (HTTP 403) and a fresh socket closed before any output
  are safe to replay; a close mid-stream, on a fresh or a reused socket, is not
- backoff: five minutes per conversation, expiry, isolation between conversations, cleared on close, bounded table
- TLS: a real rustls session over loopback. A close_notify that rustls already consumed, while the TCP connection is
  still open and shows no readable byte, fails the probe; a healthy socket passes it
- headers: `public_ws_headers` removes only the four HTTP-body headers and keeps order and values; the real entry point
  sends the assembled headers in the handshake, with no duplicates and none of the HTTP-body headers, and the header
  set is pinned so a new header cannot reach the public route unnoticed
- failed response, cancellation before connecting and mid-response, usage and cached tokens, tool calls, reasoning
  items replayed in the input, attempt rows with complete usage, and a busy socket forcing a temporary one

## What offline testing cannot show

Live acceptance of a new sign-in token at the handshake, behaviour behind a proxy that blocks WebSocket (the fallback
is tested against a refused upgrade, not a real proxy), and performance on tasks outside the four studied. The next
validation is an independently authored holdout against Codex CLI and Claude Code.
