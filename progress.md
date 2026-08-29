# Rust rewrite progress

**Complete: 99%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`. TypeScript remains in `vendor/pi` as the behavioral reference only. Rust under `crates/*` is the product.

A TypeScript `pi` user can install Rust `pi` and use the same flags, `~/.pi` sessions, provider credentials, TUI, print, RPC, JS extension registration (including CLI flags and keyboard shortcuts), and HTML export built from TypeScript `template.html` / `template.css` / `template.js` plus vendored marked and highlight.js, with live theme colors and custom-tool TUI→ANSI→HTML pre-render.

## What landed

- Vendored TypeScript `pi` (reference only).
- Workspace crates mapping every `vendor/pi/packages/*` plus session-backends and Rust-only `pi-parity`: `pi-telemetry`, `pi-protocol`, `pi-session`, `pi-session-sqlite`, `pi-ai`, `pi-agent`, `pi-tui`, `pi-client`, `pi-server`, `pi-evals`, `pi-coding-agent` (`pi` binary), `pi-parity`. Each crate has TypeScript-fixture tests.
- **pi-ai**: 39 provider catalogs, env-key map, `auth.json`, OAuth authorize URLs + token fixture parse + refresh, request bodies for anthropic-messages / openai-completions / openai-responses / google-generative-ai / google-vertex / pi-messages / openrouter-images / bedrock-converse-stream / mistral-conversations, SSE corpora, usage/cost, retry classification, live `ureq` SSE when credentials + `allow_network` (tests stay fixture-only; `PI_DISABLE_NETWORK=1` blocks live calls). Error string `No API key for provider: {id}`. Bedrock SigV4 (`AWS_ACCESS_KEY_ID` / secret / session token) and `AWS_BEARER_TOKEN_BEDROCK`. Vertex ADC marker `gcp-vertex-credentials`, placeholder `/^<[^>]+>$/`, `GOOGLE_CLOUD_PROJECT` / `GCLOUD_PROJECT` / `GOOGLE_CLOUD_LOCATION` with TypeScript error strings, fixture ADC `token` / `access_token`. Codex transports `sse|websocket|websocket-cached|auto` with account-scoped cache and SSE fallback when a websocket does not open.
- **pi-session / sqlite**: JSONL v4 header, v3→v4 migrate, continue/resume/fork/clone/fork-from-entry, entries/tree/stats/leaf, `~/.pi` + `--session-dir`. Session tree `SessionRepo` (memory + SQLite) runs TypeScript `conformance.ts`. SQLite applies TypeScript `001_initial.sql` plus sequences, stats, branch tips/entries, lanes, records, facts, FTS5, fence leases, TS lease timing errors.
- **pi-agent**: compaction, skills, templates, context files, builtin tools read/write/edit/bash/ls/grep/find, Ask/Allow/Deny permission policies, retry, steer/follow-up queue modes, thinking cycle, fixture-driven loop that executes tool calls. Codex transport + session id are forwarded on `openai-codex`.
- **pi-tui**: `Component::render(width)`, wrap, `wrapTextWithAnsi`, editor, keybindings + Ctrl/CSI byte decode, markdown, mouse/alt-buffer, raw `stty` input helpers, themes, selectors, Container/Overlay/ChatView, differential `DiffScreen`, SGR 1006 mouse + overlay hit-test, Box/VStack/HStack/ScrollView/Input/SettingsList, OSC 0 `setTitle`.
- **pi-protocol/client/server**: Unix+TCP+memory, handshake timeout, leases, request correlation, CBOR vectors. Server implements the full TypeScript `Command` union (list/create/attach/detach/prompt/steer/abort/set_model/set_thinking).
- **pi-evals / pi-parity**: fixture evals + required golden corpora. Optional `--parallel-run` / `--diff-jsonl`.
- **pi binary**: flags/subcommands from `args.ts` / `main.ts`, print/json/rpc/interactive sharing `SessionRuntime`. RPC `{type:"response",command,success}` plus streamed agent events. Prompt `streamingBehavior` queues steer/follow-up while a turn is running. Unknown command error `Unknown command: {type}`. Extension UI host implements every `RpcExtensionUIRequest` method (`select`, `confirm`, `input`, `editor`, `notify`, `setStatus`, `setWidget`, `setTitle`, `set_editor_text`) with pending-response correlation matching TypeScript. Interactive applies those requests to the TUI (title, footer statuses, widgets, selectors, editor text). JS extensions receive `pi.on` plus `ctx.ui` / `hasUI` in the Node host; `session_start` / `turn_start` / `turn_end` apply collected UI calls. Interactive raw-stdin key loop and builtin slash handlers. `/copy` uses platform clipboard tools then OSC 52 with TypeScript error strings. `/changelog` parses `CHANGELOG.md` version headers and normalizes GitHub links. `/share` tries Radius (`https://radius.pi.dev/v1/artifacts`) when a bearer token is present, then a secret gist (`gh gist create --public=false`) with `PI_SHARE_VIEWER_URL` (`https://pi.dev/session/#<id>`).
- **JS extension API**: `pi.registerProvider` / `unregisterProvider` (name+config or native `{id}` object), `pi.registerTool` (schema + `execute(toolCallId, params, signal, onUpdate, ctx)` + optional `renderCall`/`renderResult`), `pi.registerCommand` (slash handler), `registerShortcut` dispatched from the interactive key loop (reserved `ctrl+c`/`ctrl+p`/`ctrl+t`/`escape`/`enter`/`ctrl+d`/`ctrl+z` cannot be overridden), `registerFlag` parsed as CLI flags with TypeScript errors (`Extension flag "--{name}" requires a value`, `Unknown option: --x` / `Unknown options: --a, --b`) and `getFlag`. `registerMessageRenderer` / `registerEntryRenderer` / `registerMarkdownTransformer` are captured and invoked (custom transcript lines, chained Markdown transforms on user/assistant text). `appendEntry` / `sendMessage` / `sendUserMessage` persist session JSONL entries. Load-time capture; tools join `ToolRegistry`; commands appear in `/help`, RPC `get_commands` (`source: "extension"`), print/RPC prompt, and interactive slash; providers merge into model list and `AgentConfig` (`baseUrl`, `api`, interpolated `apiKey`, headers, `authHeader`). `resolveConfigValue` matches `$ENV` / `${ENV}` / `$$` / `$!` / leading `!command`. Exact errors: `Provider config is required when registering by name`, `Provider id must not be empty.`
- **HTML export**: embeds TypeScript `template.html` / `template.css` / `template.js` plus vendored `marked.min.js` and `highlight.min.js`. Session JSONL is normalized to `{header, entries, leafId}` (message entries wrap `role`/`content`/`toolCallId`/`toolName`/`details`/`isError` as `message` when missing). Theme CSS vars come from `getResolvedThemeColors` / `getThemeExportColors` (builtin dark/light plus `PI_CODING_AGENT_DIR/themes/{name}.json`). `/export` and RPC `export_html` attach `systemPrompt`, tool schemas, and `renderedTools`. Custom tools (not `bash`/`read`/`write`/`edit`/`ls`) are pre-rendered via TUI→ANSI→HTML (`ansi-to-html.ts` SGR, whitespace fixture, grep/find formatters, JS `renderCall`/`renderResult`). `escapeHtml` / `sanitizeMarkdownUrl` stay as Rust helpers locked to the TypeScript allow-list.

## What remains

- `sendMessage` `triggerTurn` / `deliverAs` queue options are stored as session entries but do not start or queue an agent turn.
- HTML export does not pre-render custom message/entry renderer output (only custom tools go through `renderedTools`).

## Next crate/module

Wire `sendMessage` turn/queue options and custom-entry HTML, then re-audit the Done bar (no documented product gaps, every `vendor/pi/packages/*` crate/module with TS-fixture tests, gates green, `progress.md` 100%).

## Gates

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```

Last run: green on 1.83 after theme-aware HTML export, extension flag/shortcut dispatch, custom-tool HTML pre-render, and message/entry/markdown renderer APIs.
