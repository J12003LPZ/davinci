# davinci-ai

`davinci-ai` is the provider integration and model communication engine. It abstracts multi-provider API protocols, real-time Server-Sent Events (SSE) streaming, prompt caching, OAuth token lifecycles, and model catalog resolution.

---

## Key Capabilities

- **Unified Streaming Pipeline (`stream.rs`)**:
  - Asynchronous background reading of SSE HTTP bodies, piping decoded frames directly into the agent sink.
  - Abort signals polled between frames for instant cancellation.
- **Provider Protocol Decoders**:
  - `stream_decoder_anthropic.rs`: Anthropic Messages API (`content_block_delta`, `message_delta`, usage reporting).
  - `stream_decoder.rs`: OpenAI Responses / Codex WebSocket streaming.
  - `stream_decoder_completions.rs`: OpenAI Chat Completions SSE format.
- **Provider Coverage**:
  - Anthropic (Claude 3.5 Sonnet, Claude 3 Opus/Haiku)
  - OpenAI (GPT-4o, o1, o3-mini)
  - OpenAI Codex (Internal & Enterprise models)
  - Amazon Bedrock (Converse Stream)
  - Google Gemini (Gemini 1.5 Pro/Flash, 2.0 Flash)
  - Local Models (Ollama, vLLM, llama.cpp)
  - Cloud Providers (Groq, Mistral, OpenRouter, Azure OpenAI)
- **Prompt Cache Parity (`cache.rs`)**:
  - Supports cache breakpoints and retention markers across Anthropic, OpenAI, and Bedrock.
  - Granular retention configuration (`short`, `long`, `none`) via environment flags (`PI_CACHE_RETENTION`).
- **Authentication & OAuth (`auth.rs`, `oauth.rs`)**:
  - Browser-based OAuth PKCE flow and Device Code flow for headless/remote environments.
  - Automatic token refresh and secure persistence.
- **Model Catalog & Cost Tracking (`catalog.rs`, `catalogs.json`)**:
  - Token cost calculations, max context window limits, and thinking budget specifications.

---

## Directory Structure

```
davinci-ai/
├── src/
│   ├── stream.rs                      # Streaming orchestrator and thread sink
│   ├── stream_decoder.rs              # Codex / Responses format decoder
│   ├── stream_decoder_anthropic.rs    # Anthropic Messages format decoder
│   ├── stream_decoder_completions.rs  # OpenAI Chat Completions decoder
│   ├── codex.rs                       # Codex provider implementation
│   ├── codex_ws.rs                    # Codex WebSocket transport
│   ├── cache.rs                       # Prompt caching logic
│   ├── auth.rs                        # API key and credential resolution
│   ├── oauth.rs                       # OAuth state and flow coordination
│   ├── catalog.rs                     # Model metadata, token costs, context limits
│   └── catalogs.json                  # Bundled model database
└── Cargo.toml
```

---

## Debugging

Enable verbose protocol frame tracing:
```bash
PI_AI_TRACE=1 cargo test -p davinci-ai
```
