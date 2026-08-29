# pi Rust rewrite progress

**Complete: 99%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize (+ sha2 for PKCE).
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`. Entry types include TS `label`, `session_info`, and `custom_message`. `set_name` clears metadata when the name is empty.
- **pi-ai**: all TS provider IDs + catalogs, all 10 `KnownApi` URLs/bodies, SSE + `live_stream`/`events_from_complete`, OpenAI/Anthropic/Google/Bedrock tool-call parse, OAuth authorize URLs + PKCE + fixture/live token exchange, device-code poller, loopback callback, Codex websocket/SSE fixture corpus. Tests never call live LLM or OAuth HTTP.
- **pi-agent**: TS agent loop events, tool execution, retry, compaction, skills, templates, context files, steer/follow-up queues, built-in tools.
- **pi-tui**: ChatChrome, overlays, SettingsList with value cycling, fullscreen/mouse, editor, markdown, fuzzy, TuiBox, ScrollView, LaTeX, Kitty CSI-u + **encodeKitty / deleteKittyImage** (`a=T,f=100,q=2`, 4096-byte chunks), iTerm wrappers, InteractiveSession (alt screen, bracketed paste, Kitty keyboard query, raw-mode keys/mouse/paste), **tool execution cards** (TS `formatToolExecution` + 10-line preview), **transcript Kitty-image placements**, **slash/path/`@` autocomplete** with Tab accept, **double-escape** tree/fork/none (500ms), `!` bash lines, Shift+Enter newline, `/model` `/settings` `/tree` overlays, **first-time setup wizard** (logo, welcome copy, Dark/Light, analytics opt-in, ↑↓/j/k, enter continue/finish, escape skip) with **OSC 11 / color-scheme queries** plus COLORFGBG / `PI_OSC11_REPLY` / `PI_COLOR_SCHEME_REPLY` detection, **login-dialog TUI** (OSC-8, Cmd/Ctrl+click, device code, manual input, waiting/progress/info, Escape → `Login cancelled`, **openBrowser argv** matching TS `open`/`rundll32`/`xdg-open` with dry-run), **`/tree` filter modes** plus **label edit**, **fold/unfold** (`⊞`/`⊟`), **ctrl+x copy**, **horizontal viewport**, **scoped-models selector**, **Mermaid transformer**, **custom-message** purple box with `[type]` label and **extension `MessageRenderer` / `EntryRenderer` / `MarkdownTransformer`** (native + JS `renderMessage` / `renderEntry` / `transformMarkdown`).
- **Settings submenus**: `/settings` Theme opens Automatic (`light/dark`) plus Light/Dark/Apply/Change mode; Warnings toggles Anthropic extra usage; Per-model thinking walks model then level (`__clear__` removes an override). Values persist as `theme`, `warnings.anthropicExtraUsage`, and `modelThinkingLevels`.
- **Interactive extras**: `keybindings.json` overrides (defaults: `ctrl+g` external editor, `ctrl+v` image/text paste, `alt+enter` follow-up, `alt+up` dequeue). External editor uses `externalEditor` / `$VISUAL` / `$EDITOR` / `vi`, writes `pi-editor-*/prompt.md`, strips BOM, dry-runs via `PI_EXTERNAL_EDITOR_DRY_RUN`. Clipboard paste uses `PI_CLIPBOARD_IMAGE` / `PI_CLIPBOARD_TEXT` fixtures. Follow-up queues the editor buffer; dequeue restores it.
- **Session overlay extras**: `/resume` uses SessionSelector with **ctrl+p path**, **ctrl+s sort** (Threaded / Recent / Fuzzy), **ctrl+r rename** (empty name clears), **Tab scope** (Current Folder / All), **ctrl+n named filter**, **ctrl+d / ctrl+backspace delete** (refuses the active session; `trash` then unlink; `PI_SESSION_DELETE_DRY_RUN`), and `re:` / `"phrase"` search.
- **Live OSC probe**: `begin_osc_query` / `finish_osc_query` with TS `timeoutMs: 100`. Replies ingested from `handle_bytes`; timeout falls back to COLORFGBG / dark. Automatic theme applies the light or dark half when the probe completes.
- Unit tests do not require a TTY.
- **pi-client / pi-server**: Unix + TCP + memory, handshake timeout, request correlation, leases.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness.
- **pi-coding-agent `pi`**: flags/subcommands, slash (including `/login` dialog, `/model`, `/settings` cycling the full TS settings-selector item list plus the three submenus, `/tree` filters/label/copy/fold, `/scoped-models`, `/resume` selector, **`/import` JSONL**, **`/share`** gist/`PI_SHARE_DRY_RUN`/`PI_SHARE_URL`, **`/changelog`** TS `## [x.y.z]` parser), RPC (`prompt`/`steer`/`follow_up`/`clear_queue` and the rest of the TS command set), settings/trust (`treeFilterMode`, `markdown.mermaid`, `enableAnalytics`/`trackingId`, `enabledModels`, `httpIdleTimeoutMs`, `hideThinkingBlock`, `showCacheMissNotices`, transport/steering/follow-up, TUI/fullscreen, images, padding, `externalEditor`, `modelThinkingLevels`, warnings), HTML export using TS templates, JS/TS extension runner with virtual `@earendil-works/*` packages and `registerMessageRenderer` / `registerEntryRenderer` / `registerMarkdownTransformer`, `pi update --self` native copy or npm/pnpm/yarn/bun argv, `pi update --models` catalogs. First-time setup gate matches TS (`PI_EXPERIMENTAL=1`, no `PI_CODING_AGENT_DIR`, missing `settings.json`). Tree labels persist as JSONL `label` entries. Copy uses `pbcopy`/`clip`/`xclip` (dry-run in tests). `/reload` reloads skills, templates, context files, and `keybindings.json`.
- **pi-parity**: six required corpora + optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- **Clipboard conversion**: live `wl-paste` / `xclip` / `xsel` / `pngpaste` / `pbpaste` / WSL PowerShell readers run when `PI_CLIPBOARD_*` fixtures are unset. Photon BMP→PNG conversion from TS is not ported.
- **Keybinding catalog**: `keybindings.json` covers the interactive extras wired above. Remaining TS ids (`app.model.select`, `app.tools.expand`, `app.session.new`, extension shortcuts, …) are not a complete override map.
- **OSC TTY reader**: the 100ms probe consumes replies that arrive on the session byte path (including injected OSC). There is no separate raw-TTY drain independent of the crossterm event loop.
- **Share / reload depth**: `/share` uses gist/`PI_SHARE_*` fixtures, not the live Radius artifact upload. `/reload` does not restart the JS extension host or rescan theme files.
- **`re:` search**: session selector implements `.` `*` `+` `?` and escapes, not a full JavaScript `RegExp`.
