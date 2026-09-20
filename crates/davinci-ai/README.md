# davinci-ai

Provider authentication, model discovery, request shaping and streaming.
Start with [src/lib.rs](src/lib.rs), which defines the public types and exports;
its module declarations identify the active implementation.

| Concern | Source |
| --- | --- |
| Request/response orchestration | [stream.rs](src/stream.rs), [request_shape.rs](src/request_shape.rs) |
| SSE body reading and limits | [stream_reader.rs](src/stream_reader.rs) |
| Responses, Anthropic and Chat Completions decoding | `src/stream_decoder*.rs` |
| Codex transport and connection state | [codex.rs](src/codex.rs), [codex_ws.rs](src/codex_ws.rs) |
| Credentials and provider auth | [auth.rs](src/auth.rs) |
| OAuth coordination, provider flows and callback listener | [oauth.rs](src/oauth.rs), [oauth_providers.rs](src/oauth_providers.rs), [oauth_callback.rs](src/oauth_callback.rs) |
| Model metadata and prices | [catalog.rs](src/catalog.rs), [catalog_include.rs](src/catalog_include.rs), [catalogs/](catalogs/) |
| Prompt-cache controls | [cache.rs](src/cache.rs) |
| Optional wire tracing | [trace.rs](src/trace.rs) |

The SSE reader applies backpressure and limits each raw frame, including its
blank terminator, to 16 MiB. Foreground abort polling does not interrupt a worker
already blocked in a socket read; that worker still depends on transport timeout.
Final decoded messages retain the accepted response.

Tests are inline and use fixtures, loopback servers and local subprocesses:

```sh
cargo test -p davinci-ai --offline --locked
```

This check does not establish live provider/OAuth availability. See the
[workspace architecture](../../docs/ARCHITECTURE.md) for the surrounding runtime.
