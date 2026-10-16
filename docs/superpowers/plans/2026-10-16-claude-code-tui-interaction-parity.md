# Claude Code TUI interaction and rendering parity: audit and plan

Status: analysis and implementation plan only. No product code was changed for
this document. Nothing below claims installed behavior, visual parity, or
terminal-compatibility results that have not been run.

## 0. Scope and relation to existing plans

This document covers the terminal **infrastructure and interaction layer**:
the prompt editor's caret and editing model, mouse and clipboard, footer state,
focus and key routing, the renderer (screen buffers, alternate screen, escape
sequences, cursor, incremental updates, flicker), and animation timing.

It deliberately does not repeat the visual contract that already exists:

| Existing document | What it owns | Status from source inspection on 2026-10-16 |
| --- | --- | --- |
| `docs/superpowers/plans/2026-09-24-claude-code-terminal-ui.md` | palette (`Theme::cc()`), composer/completion/footer text layout, transcript rows, working line, permission and picker panels, measured from 2.1.281 frames | Largely landed: `Theme::cc()` (`davinci/theme.rs:732`), `?` shortcuts panel (`app.rs:840`), mode labels and `(shift+tab to cycle)` footer (`views/chrome.rs:118`), `·✢*✶✻✽` working glyphs (`views/opera.rs:11`), shimmer (`views/opera.rs`), Enter-on-slash (`app.rs` `handle_suggestion_key`). Its "Intentional Differences" still apply. |
| `docs/superpowers/plans/2026-09-20-claude-code-terminal-parity*.md`, `docs/superpowers/specs/2026-09-20-claude-code-terminal-parity-reference-audit.md` | whole-screen inventory, capture protocol, graph | Reference ledger; capture gaps remain as recorded there. |
| `docs/ui/claude-code-reference/` | 2.1.281 frames (text + style runs) | Ground truth for static layout. Captured through `pyte`, which does not model mouse, focus, or synchronized output, so it cannot validate this document's areas. |

Out of scope, per the request: model routing, caching, agent reasoning,
orchestration. Subscription constraints in `AGENTS.md` are unaffected: none of
the work below adds an API path, a paid service, or extra model calls.

## 1. Reference and evidence method

**Pinned reference:** Claude Code **2.1.301**, the npm `latest` on 2026-10-16
(`@anthropic-ai/claude-code-linux-x64@2.1.301`, a Bun single-file executable).

The public repository `anthropics/claude-code` does **not** contain the CLI
source. It ships the README, CHANGELOG, plugins, examples and `mods/`; its
license reserves rights. So "verify against the actual source" was done in two
ways, and every claim below names which:

- **[B]** the JavaScript bundle embedded in the 2.1.301 executable, inspected
  with `strings` for identifiers, constants and control flow. Used only to
  establish *behavior and constants*. No Anthropic code is copied into DaVinci
  or into this repository, and the executable is not committed. DaVinci's
  implementation stays independent Rust on ratatui/crossterm.
- **[D]** the official documentation pages fetched on 2026-10-16:
  `fullscreen`, `interactive-mode`, `keybindings`, `terminal-config`,
  `statusline`, `settings` under `https://code.claude.com/docs/en/`.
- **[F]** the existing 2.1.281 reference frames in `docs/ui/claude-code-reference/`.
- **[DV]** DaVinci source at `8119a17` (branch `claude/dazzling-shannon-st9ndv`).

Unverified items are marked **unverified** rather than inferred.

## 2. Reference architecture (Claude Code side)

### 2.1 Two renderers

[D][B] Claude Code has two renderers, selected by `/tui default|fullscreen`,
the `tui` setting, and `CLAUDE_CODE_NO_FLICKER`:

- **Classic (`default`)**: draws on the main screen; history goes to the
  terminal's own scrollback; the input box moves as output streams.
- **Fullscreen**: alternate screen buffer, the input pinned at the bottom,
  only visible messages kept in the render tree ("virtual scroll",
  `CLAUDE_CODE_DISABLE_VIRTUAL_SCROLL` opts out), in-app scrolling, mouse
  capture, in-app selection and copy-on-select. Background/attached sessions
  always use it. A failed fullscreen start (no first frame + 10 s, or no clean
  exit) is counted per version and falls back to classic.

DaVinci today has only an alternate-screen renderer
(`davinci/runtime.rs` `Session::open`), so **the parity target for this plan
is the fullscreen profile**. A classic renderer is listed as P3 and is not
recommended (section 6).

### 2.2 Renderer core [B]

Claude Code runs its own fork of Ink (React reconciler + Yoga layout). Its
exported surface includes `Box`, `Text`, `Link`, `NoSelect`, `RawAnsi`,
`Button`, `FocusManager`, `PointerEvent`, `ClickEvent`, `ThemeProvider`,
`useFocus`, `useHasFocus`, `useTerminalFocus`, `useSelection`,
`useAnimationFrame`, `useAnimationTimer`, `useClock`, `useInputClock`,
`useTerminalViewport`, `usePaintedRows`, `useTerminalTitle`, `useTabStatus`.

Facts established from the bundle:

| Concern | Reference behavior [B] |
| --- | --- |
| Cell storage | Each screen is a packed typed array, two 32-bit words per cell (character id, style id), with interning pools for characters, styles and hyperlinks (`CharPool`, `StylePool`, `HyperlinkPool`). |
| Double buffering | `frontFrame` / `backFrame`; each frame diffs the new screen against the previous one and writes only changed cells ("sends only the cells that changed", [D]). |
| Frame pacing | A paced scheduler: minimum **16 ms** between frames normally; after a keystroke an *input-priority* window of **50 ms** lowers the minimum to **4 ms** so typing echoes immediately; when non-blocking stdout has more than **16 KiB** queued the frame is held and retried after **250 ms** (backpressure instead of piling writes). Rendering is queued on a microtask, coalescing bursts of state changes. |
| Synchronized output | DEC mode 2026 (`CSI ? 2026 h/l`) wraps frames when supported. Support is **probed** with DECRQM (`CSI ? 2026 $ p`); XTVERSION (`CSI > 0 q`) identifies the terminal; Apple Terminal and old VTE are not asked. tmux ≤3.6 lacks it ([D]). |
| Full repaint escape hatch | `CLAUDE_CODE_ALT_SCREEN_FULL_REPAINT=1` repaints every cell each frame for ConPTY hosts that mis-coalesce positioned writes ([D]). |
| Scroll region renderer | An optional DECSTBM scroll-region path exists behind a flag (`CLAUDE_CODE_DECSTBM`), disabled under tmux/zellij. |
| Scroll containers | `ScrollBox` nodes keep `scrollTop`, `pendingScrollDelta`, `stickyScroll` (auto-follow), `scrollAnchor`, and a high-water mark so content growth above the viewport does not jump the view. |
| Terminal modes | Table `{CURSOR_VISIBLE:25, ALT_SCREEN:47, ALT_SCREEN_CLEAR:1049, MOUSE_NORMAL:1000, MOUSE_BUTTON:1002, MOUSE_ANY:1003, MOUSE_SGR:1006, MOUSE_SGR_PIXELS:1016, FOCUS_EVENTS:1004, BRACKETED_PASTE:2004, THEME_NOTIFY:2031, SYNCHRONIZED_UPDATE:2026, WIN32_INPUT_MODE:9001}`. Mouse mode **full** = 1000+1002+1003+1006, **scroll** = 1000+1006 (clicks disabled), **off**. 1016 pixel reports are used when the terminal answers the probe. Keyboard: kitty push `CSI > 1 u` / pop `CSI < u`, xterm modifyOtherKeys `CSI > 4 ; 2 m` / reset `CSI > 4 m`. Modes are re-asserted on stdin resume and on resume from suspend. |
| OSC usage | 0/1/2 title, 7 cwd, 8 hyperlinks, 9 and 777 notifications, 10/11/12 color queries, 52 clipboard, 133 semantic prompt, 1337 iTerm2, plus `TAB_STATUS` (21337) and `PROGRAM_STATUS` (7501). OSC 11 is queried to learn the background for the `auto` theme; mode 2031 notifies theme changes. |
| Exit/restore | On teardown it resets SGR-pixel mouse, all mouse modes, extended keys, focus events, theme notify, bracketed paste, shows the cursor, and restores cursor position (`ESC 7`/`ESC 8`). |

### 2.3 What the terminal does versus what Claude Code does

| Behavior | Provided by Claude Code | Provided by the terminal emulator |
| --- | --- | --- |
| Alternate screen, scrollback isolation | Requests 1049 | Implements the second buffer; `Cmd+F`/tmux search cannot see it ([D]) |
| Flicker-free frames | Cell diff, pacing, 2026 wrapping | Honoring 2026; ConPTY coalescing bugs ([D]) |
| Caret | Painted inverse cell; parks the real cursor at the caret | Cursor shape and **blink** only when the native cursor is shown (opt-in) |
| Mouse events | Requests 1000/1002/1003/1006(/1016), parses SGR | Must forward them; iTerm2 per-profile "Enable mouse reporting"; tmux needs `set -g mouse on` |
| Selection | In-app (when capturing): drag, word/line, keyboard extend, highlight color | Native selection only with the bypass modifier (Shift; Option in iTerm2; Fn in Terminal.app) |
| Clipboard | pbcopy, wl-copy/xclip/xsel (+PRIMARY), PowerShell `Set-Clipboard`, tmux buffer, OSC 52 fallback | OSC 52 permission (iTerm2 blocks by default); `Cmd+C` on native selection |
| Links | Emits OSC 8; Cmd/Ctrl-click handled in-app in fullscreen | xterm.js terminals (VS Code) handle links themselves |
| Shift+Enter | Kitty/modifyOtherKeys negotiation; `/terminal-setup` writes terminal config | Whether the key is distinguishable at all |
| Focus in/out | Requests 1004; dims caret, schedules recap when unfocused | Sending `CSI I`/`CSI O`; tmux `focus-events on` |
| Wheel speed | `CLAUDE_CODE_SCROLL_SPEED` (0.25–20), acceleration, JetBrains quirks | Event rate per notch varies by terminal |
| Tab title/progress | OSC 0/2, 9;4, 21337 | Whether they are drawn |

## 3. Feature-by-feature reference and DaVinci comparison

Legend for **Gap**: `—` none, `S` small, `M` medium, `L` large.

### 3.1 Prompt editor

| Feature | Reference behavior | Evidence | DaVinci today | Gap |
| --- | --- | --- | --- | --- |
| Caret drawing | Painted **inverse** cell at the cursor; on an empty prompt the placeholder's first character is inverted and the rest dim. **No blink timer** exists in the bundle; the painted caret is steady. | [B] placeholder/caret renderer; no caret interval found | Painted cell, but **blinks** at ~1 s on the 250 ms tick, solid after movement (`model.rs:3735`, `views/chrome.rs:512`) | S — remove blink by default (keep as a setting) |
| Caret when terminal unfocused | Caret is not painted while the terminal reports focus-out (`terminalFocus` gate) | [B] | No focus events requested | S |
| Real cursor parking | The hidden hardware cursor is moved to the caret's line/column each frame ("cursor declaration"), so IME candidate windows and screen magnifiers track the caret | [B] `caret`/`parkCursor`/`setCursorDeclaration` | Never calls `set_cursor_position`; ratatui leaves the cursor hidden at an arbitrary cell; IME composition appears at the wrong place | M |
| Native cursor mode | Opt-in (`CLAUDE_CODE_NATIVE_CURSOR`, accessibility, screen reader): the terminal's own cursor is shown at the caret instead of the painted cell, so **shape and blink come from the terminal**. DECSCUSR is parsed but never emitted by Claude Code. | [B] | None | S |
| Multiline input | `\`+Enter, Shift+Enter (native in iTerm2/WezTerm/Ghostty/Kitty/Warp/Apple Terminal/Windows Terminal), Option+Enter, Ctrl+J, paste | [D] | Shift+Enter, Alt+Enter, Ctrl+J (`keybindings.rs:128`); `\`+Enter **unverified/absent** | S |
| Soft wrap | Long logical lines wrap onto visual rows; Up/Down move by **visual** row first and reach history only from the first/last visual row | [D] | No soft wrap: each logical line is clipped and scrolled horizontally with `…` (`views/chrome.rs:770`) | M |
| Readline editing | Ctrl+A/E/K/U/W/Y, Alt+Y, Alt+B/F/D, Ctrl+_ undo; Ctrl+W deletes to whitespace while Alt word motions stop at punctuation | [D] | Shared `Editor` has the motions, kill ring and undo (`editor.rs`); Ctrl+W semantics vs Alt+B semantics **unverified** | S |
| History | Per working directory, spans sessions; consecutive duplicates collapsed; draft cleared with Esc Esc or Ctrl+C is recallable with Up | [D] | In-memory `Editor` history for the process only (`editor.rs:119`); Esc Esc saves the draft (`app.rs` `clear_draft_on_double_escape`); not persisted per project | M |
| Reverse search | Fullscreen: Ctrl+R opens a dialog, Up/Down through matches, Ctrl+S cycles scope session/project/all, Enter/Tab places, Esc cancels; matches highlighted | [D] | None in the live shell (legacy chrome has none either) | M |
| Stash | Ctrl+S stashes the draft (text, cursor, pastes, input mode); again on empty restores | [D] | Ctrl+S opens Sessions overlay (`keybindings.rs:119`) | S + keymap decision |
| External editor | Ctrl+G or Ctrl+X Ctrl+E | [D] | Binding declared (`app.editor.external`) but not handled by the live shell | S |
| Shell mode `!` | `!` prompt glyph and pink border, history-based Tab completion, path completion on `/` tokens, Esc/Backspace/Ctrl+U on empty exits, pasted `!` enters mode | [D][F] | Glyph, border color and footer hint present (`views/chrome.rs:540`); history completion and exit keys **unverified** | S |
| Slash/`@`/`:` menus | Floating list above the prompt; hover highlight and click to accept in fullscreen; `:` emoji shortcodes | [D][F] | Layout done per 09-24 plan; wheel steps the list (`runtime.rs:719`); no click, no hover; no emoji shortcodes | S (click) |
| Paste | Bracketed paste; large pastes collapse to `[Pasted text #N]` chips that can be deleted/restored | [D] | Bracketed paste + Windows burst filter (`runtime.rs` `PasteFilter`); paste markers expanded on submit (`model.rs:4087`) | — |
| Queue while running | Enter queues; Up from the first row takes queued messages back; Ctrl+Enter / Ctrl+X Ctrl+S sends now | [D] | Queue exists (`model.rs:4109`), Alt+Up dequeue binding; Up-takes-back and send-now **unverified/absent** | S |

### 3.2 Mouse and clipboard

| Feature | Reference behavior | Evidence | DaVinci today | Gap |
| --- | --- | --- | --- | --- |
| Tracking modes | full (1000+1002+1003+1006), scroll-only (1000+1006), off; SGR-pixel 1016 when available; `CLAUDE_CODE_DISABLE_MOUSE`, `CLAUDE_CODE_DISABLE_MOUSE_CLICKS` | [B][D] | Always 1000+1002+1006 on Unix (`runtime.rs:113`), crossterm capture on Windows; no 1003 hover, no opt-out env | S |
| Drag selection | Anchor/focus kept in **screen-buffer** space scoped to the scroll container; survives scrolling; drag selection persists after release | [B] | Anchor/focus are **raw screen coordinates** (`runtime.rs` `selection_anchor`); scrolling after a drag selects different text; content comes from the last composed lines | M |
| Multi-click | Double-click selects a word with iTerm2-style boundaries (a path is one word; a URL selects whole including scheme); triple-click selects the line; window: **500 ms** and ±1 cell | [B][D] | None in the live shell. The legacy `tui_alt_screen.rs:1194-1340` already implements click counting and word/line granularity and can be ported | S–M |
| Selection highlight | Theme token `selectionBg` (dark `rgb(38,79,120)`, light `rgb(180,213,255)`, ANSI themes `blue`/`cyan`) applied as a **background**, preserving foreground colors | [B] | `Modifier::REVERSED` over the range (`runtime.rs:575`) | S |
| Copy-on-select | On mouse release, copies non-empty selection; default **on**; `/config` toggle "Copy on select"; toast names the path used; first few native-path copies hint "disable auto-copy in /config" | [B][D] | Copies on release via `copy_text` (`runtime.rs:615`); no setting in the live shell (legacy has `copy_on_select`), no toast | S |
| Manual copy | `Ctrl+Shift+C`; `Cmd+C` with kitty protocol; **Ctrl+C copies instead of interrupting when a selection exists** | [D] | None | S |
| Keyboard extend | Shift+←/→/↑/↓ extend; Shift+↑/↓ scroll at edges; Shift+Home/End to line ends | [D] | None | M |
| Which keys clear a selection | Esc keeps it (and does its normal action); PgUp/PgDn, Ctrl+Home/End, modified arrows keep it; any other key clears it; `selection:clear` action | [D] | Selection persists until next press; keys never clear it | S |
| Clipboard backends | macOS `pbcopy`; Linux `wl-copy` (Wayland) or `xclip`/`xsel`, writing **CLIPBOARD and PRIMARY**; Windows/WSL PowerShell `Set-Clipboard` (UTF-8); inside tmux also `tmux load-buffer`, with DCS passthrough for OSC 52; GNU screen DCS chunking; **OSC 52** over SSH; VTE-based terminals get no OSC 52 | [B][D] | `pbcopy`, `clip`, `xclip -selection clipboard` only (`open_browser.rs:47`): no Wayland, no `xsel`, no PRIMARY, no OSC 52 (legacy `tui_alt_screen.rs:554` has OSC 52), `clip.exe` mangles non-ASCII | M |
| Wheel | Per-notch base distance × `CLAUDE_CODE_SCROLL_SPEED`, acceleration on fast spins (`wheelScrollAccelerationEnabled`), JetBrains-specific handling; wheel over a list scrolls that list | [D] | Fixed 3 rows (`views/scrollbar.rs:23`); wheel over completion list steps it | S |
| Scrollbar | Appears on hover beside overflowing list panels; click track, drag thumb, arrows | [D] | Conversation scrollbar with click and drag (`runtime.rs` `route_scroll_mouse`); not in list panels | S |
| Click targets | Click in prompt positions the cursor; click menu rows/options (select, multi-select toggle, submit, "Other" text row); click a `/config` value; click collapsed tool result to expand; click `↑ N more`; Ctrl/Cmd-click URL or file path opens it | [D] | Only scrollbar and graph clicks | L (many small targets, one hit-test framework) |
| Hyperlinks | OSC 8 for URLs, PR badge, file paths; `FORCE_HYPERLINK` | [B][D] | None in the live shell (legacy `markdown.rs` emits OSC 8) | S |

### 3.3 Footer and status UI

| Feature | Reference | Evidence | DaVinci today | Gap |
| --- | --- | --- | --- | --- |
| Permission mode line | `⏵⏵ accept edits on (shift+tab to cycle)`, `⏸ plan mode on`, `⏵⏵ auto mode on`, manual mode, `esc to interrupt` while running | [F] | Done (`views/chrome.rs:118`) | — |
| Agents / background | `· ← N agents`; footer items are **focusable**: `Footer` context with ←/→, ↑/↓ (deselect at top), Enter open, Esc clear, `x` stop or dismiss | [D][F] | Text-only `← N agents` and `N jobs · N agents · N workflows` (`views/chrome.rs:811`); not focusable | M |
| Task checklist | Ctrl+T toggles Claude's to-do list (max five rows) in the status area; state restored on resume | [D] | Ctrl+T is `app.thinking.toggle`; checklist lives in `/tasks`-like sheets and transcript tallies (`views/transcript.rs:860` says `ctrl+t to expand`) | S + keymap decision |
| PR badge | `PR #N` OSC 8 link with colored underline (green approved, yellow pending, red changes requested, gray draft); refreshed after `git push`/`gh pr`; hints when `gh` is missing or unauthenticated | [D] | None | M (needs `gh`/token read; no paid service) |
| Exit/clear hints | `Press Ctrl-C again to exit`, double-press Ctrl+D within 800 ms, `Esc again to clear` | [D][F] | Present (`views/chrome.rs:132`) | — |
| Toasts/notifications | Bottom-right toast stack (copy path, hints); OSC 9/777 notifications when unfocused | [B][D] | Governor notice row above composer; OSC 9;4 progress only (`runtime.rs` `set_progress`) | M |
| Jump to bottom | Floating `Jump to bottom` button with `N new messages`, keyboard hint adapting to platform; dim sticky header showing the last prompt scrolled above the view, clickable | [D] | None | M |
| Context indicator | Status line is user-scriptable; built-in context warning | [D] | Context % at ≥80 % (`views/chrome.rs:174`), context bar above input | — |

### 3.4 Interactive controls and focus

| Feature | Reference | Evidence | DaVinci today | Gap |
| --- | --- | --- | --- | --- |
| Context-scoped keybindings | ~28 contexts (`Global`, `Chat`, `Autocomplete`, `Confirmation`, `Tabs`, `Footer`, `Scroll`, `Transcript`, `HistorySearch`, `Select`, `Settings`, …), `~/.claude/keybindings.json`, chords (`Ctrl+X Ctrl+K`), reserved keys | [D] | Flat action map with JSON overrides (`keybindings.rs`), one input owner per frame (`app.rs` `input_owner`: Modal/Surface/Autocomplete/Composer); no chords | M |
| Default keymap | Ctrl+C interrupt/clear/exit; Ctrl+D delete-forward or double-press exit; Ctrl+L redraw; Ctrl+O transcript viewer; Ctrl+R history; Ctrl+T todos; Ctrl+S stash; Ctrl+B background; Ctrl+G editor; Ctrl+Z suspend; Shift+Tab modes; Alt+P model; Alt+T thinking; Alt+O fast; Esc interrupt/close; Esc Esc clear or rewind | [D] | Differences: Ctrl+T thinking, Ctrl+S sessions, Ctrl+O toggles tool output in place, Ctrl+Alt+* instruments, Ctrl+L/Ctrl+R/Ctrl+B/Ctrl+G/Ctrl+Z not wired in the live shell | M + **user decision** |
| Dialog focus | Tabs: ←/→ on the tab row, ↑/↓ moves between tab row and content; Select: j/k, Ctrl+N/P, PgUp/PgDn, Home/End; Ctrl+C/Ctrl+D twice close a dialog | [D] | Per-sheet handlers; Home/End/PgUp/PgDn inconsistent across sheets (**unverified per sheet**) | S |
| Permission approval | Select with numbered options, Tab adds a comment to Yes/No, Shift+Tab = "allow for session" on file prompts, clicks select | [D][F] | Approval/decision modals (`views/approval_modal.rs`, `views/decision_modal.rs`); no comment field, no click | S–M |
| Transcript viewer | Ctrl+O toggles; `less`-style j/k, g/G, Ctrl+U/D, Space/b, `{`/`}` prompt jumps, `/` search with n/N, `[` dumps the conversation to native scrollback, `v` opens it in `$EDITOR`, `?` help | [D] | Ctrl+O expands tool output in place; legacy `alt_screen_search.rs` has search | M |
| Suspend/redraw | Ctrl+Z suspends and restores modes on `fg`; Ctrl+L repaints keeping state | [D][B] | Neither in the live shell; `Session::clear` exists | S |

### 3.5 Rendering system

| Feature | Reference | DaVinci today (`davinci/runtime.rs`, `app.rs`) | Gap |
| --- | --- | --- | --- |
| Composition | React tree + Yoga layout; only visible messages mounted | `compose_frame` builds every visible line each draw; transcript caches unchanged history (`views/transcript.rs:205`) but `compose_frame` clones the whole `Model` for pickers and dimmed overlays, and `over_suggestions` recomposes a whole frame per wheel event (`runtime.rs:770`) | M (perf) |
| Cell diff | Own double-buffered packed screens | ratatui `Terminal::draw` double buffer and cell diff — equivalent in effect | — |
| Synchronized output | 2026 wrap, probed | **Not emitted** by the live shell (only legacy `tui_runtime.rs:30`); ratatui-core 0.1.2's `draw` does not add it (checked in the fetched crate source); crossterm 0.29 provides `BeginSynchronizedUpdate` | S, high value |
| Frame pacing | 16 ms floor, 4 ms input priority, stdout backpressure | Draw when dirty; a single 250 ms tick drives all animation (`runtime.rs:35`) | S–M |
| Alt screen lifecycle | 1049 enter/exit, re-assert on resume, panic-safe | Enter/leave + panic hook + `reacquire` (`runtime.rs:120-200`) — good | — |
| Mode restore | Resets mouse, extended keys, focus, paste, cursor | Restores mouse, kitty flags, paste, cursor (`runtime.rs:125`) | — |
| Keyboard protocol | kitty `>1u` (disambiguate) + modifyOtherKeys fallback; win32-input-mode | kitty `DISAMBIGUATE_ESCAPE_CODES` when `supports_keyboard_enhancement` | S (modifyOtherKeys fallback for Shift+Enter in xterm/VS Code) |
| Capability probing | DECRQM 2026, XTVERSION, OSC 11, cell-pixel size, kitty graphics; logs results | None besides kitty-keyboard support | M |
| Full-repaint mode | Env switch for ConPTY | None; `Session::clear` forces one full repaint | S |
| Resize | Re-layout; hint on garbage winsize | ratatui autoresize on draw | — |
| Title/progress | OSC 0/2 title, OSC 9;4, tab status | `set_title`, OSC 9;4 when enabled | — |

### 3.6 Visual polish and microinteractions

Colors, spacing, borders, composer rules, completion rows and panel layout are
owned by the 2026-09-24 plan and are mostly in place. Remaining deltas found by
this audit:

| Item | Reference [B] | DaVinci | Gap |
| --- | --- | --- | --- |
| Working glyph cycle | `· ✢ * ✶ ✻ ✽` forward then reversed (12 steps, ends doubled); on `TERM=xterm-ghostty` `✳` replaces `*` | `·✢*✶✻✽` ping-pong over 10 steps at 250 ms (`views/opera.rs:17`) | S |
| Stall color | Spinner color blends toward `error` as a stall intensity rises; above 0.5 it switches to `error` | Already modeled: a quiet turn warms the spinner toward the error ink (`views/opera.rs`, test `a_quiet_turn_warms_the_spinner_toward_the_error_ink`) | — |
| Breathing dot | Cosine pulse (`(1-cos 2πt/T)/2`) for idle/teammate dots, bold above half | `breath()` helper exists (`views/opera.rs:79`) | — |
| Theme set | `dark`, `light`, `dark-daltonized`, `light-daltonized`, `dark-ansi`, `light-ansi`, `auto` (OSC 11 + mode 2031), custom themes | dark/light/vox/vox-classic, `NO_COLOR`, color-depth fallback (`davinci/term.rs`) | M (daltonized + ANSI-only + auto) |
| Selection, user-message, bash-message backgrounds | `selectionBg`, `userMessageBackground` `rgb(55,55,55)`, `bashMessageBackgroundColor` `rgb(65,60,65)` (dark) | Partially in `Cc` | S |
| Spinner frame interval | Driven by a shared animation clock; exact period **unverified** from the bundle | 250 ms | measure from a timed capture before changing |

## 4. Dependencies and constraints

- Rust 1.88.0 pinned, ratatui `=0.30.0`, crossterm `=0.29.0`: both already
  provide what is needed (`BeginSynchronizedUpdate`, `EnableFocusChange`,
  `SetCursorStyle`, `MoveTo`/`Show`/`Hide`, SGR mouse). **No new crate is
  required for any P0/P1 task.** Base64 for OSC 52 is already a dependency.
- Clipboard tools are run as child processes without a shell, as today. No
  daemon, network or paid service.
- PR badge reads `gh` or an existing token only if present; absent means no
  badge (never a fabricated state). Nonessential network is skipped when the
  equivalent of "nonessential traffic off" is set.
- Every new escape sequence is behind a capability or setting so that
  `NO_COLOR`, dumb terminals, Windows conhost, ConPTY and tmux keep working.
- Keep the 09-24 plan's naming rule: no "Claude" strings, DaVinci identity.

## 5. Prioritized implementation plan

Each task: one commit, focused tests, `cargo fmt`, `cargo clippy -p davinci-tui`,
`cargo test -p davinci-tui` (plus `davinci-coding-agent --bin davinci` tests
where the host loop changes). Tests stay offline: ratatui `TestBackend`
buffers, byte-level assertions on emitted sequences through a writer
abstraction, and synthetic `crossterm::event` sequences. No live model calls.

### P0 — flicker, caret, selection and clipboard correctness

**T1. Synchronized frames and paced drawing.**
Files: `davinci/runtime.rs`, `davinci-coding-agent/src/davinci_interactive.rs`.
Wrap each `terminal.draw` in `BeginSynchronizedUpdate`/`EndSynchronizedUpdate`
when enabled. Enable from a startup DECRQM probe (`CSI ? 2026 $ p`, 100 ms
timeout, answered `1|2|3` → on); default off under tmux unless it answers; env
override `DAVINCI_SYNC_OUTPUT=0|1`. Add a pacer: at most one frame per 16 ms,
4 ms within 50 ms after a key event; animation stays on its own tick.
Tests: emitted bytes contain `?2026h` … `?2026l` around a frame when enabled and
nothing when disabled; pacer unit tests with an injected clock.
Terminal compatibility: tmux ≤3.6 and Apple Terminal ignore or are not asked.

**T2. Caret parity and IME parking.**
Files: `views/chrome.rs`, `model.rs`, `runtime.rs`, `app.rs`.
Steady painted caret by default (setting `caretBlink`, default off, to keep
today's behavior available). After each draw, `MoveTo` the hidden cursor to the
caret cell computed by `composer()` (return it from `compose_frame`). Request
focus events (`EnableFocusChange`); on `FocusLost` do not paint the caret, on
`FocusGained` repaint. Optional native cursor (`DAVINCI_NATIVE_CURSOR=1` or
accessibility setting): `Show` the terminal cursor at the caret instead of
painting, leaving shape/blink to the terminal.
Tests: caret cell has inverse style regardless of tick; composed frame reports
caret position for ASCII, CJK and combining input; focus-lost frame has no
caret; native mode paints no caret cell.

**T3. Content-anchored selection with theme highlight.**
Files: `runtime.rs` (selection state), `app.rs` (`ComposedFrame` gains a row →
transcript-line map), `theme.rs` (`selection_bg`).
Store selection endpoints in transcript coordinates when inside the
conversation, so wheel/PgUp keep the same text selected; clamp to the
composer/footer otherwise. Paint with `selection_bg` as background, keeping
foregrounds; `NO_COLOR` falls back to reversed. Exclude scrollbar and
decorative gutters (`NoSelect` equivalent) from copied text.
Tests: select, scroll 3 rows, selection still covers the same words; copied
text excludes scrollbar column; highlight style per theme.

**T4. Multi-click and keyboard selection.**
Port click counting (500 ms, ±1 cell) and word/line granularity from
`tui_alt_screen.rs:1194-1340`; word boundaries treat path and URL characters as
word characters. Add Shift+arrows/Home/End extend, edge scrolling on
Shift+↑/↓, the key-clearing rules from section 3.2, `Ctrl+Shift+C` copy, and
Ctrl+C copies (not interrupts) while a selection exists.
Tests: double-click on `src/utils/foo.ts` selects the whole path; on a URL
selects scheme through end; triple-click selects the row; each key class keeps
or clears the selection.

**T5. Clipboard backend chain and copy feedback.**
Files: `open_browser.rs` (or a new `clipboard.rs` in `davinci-tui`), `runtime.rs`.
Order: SSH session → OSC 52 (wrapped for tmux/screen); macOS `pbcopy`;
Wayland `wl-copy` (+ `--primary`); X11 `xclip` then `xsel` for CLIPBOARD and
PRIMARY; Windows/WSL `powershell -NoProfile -Command Set-Clipboard` fed UTF-8
via stdin (keep `clip` as last resort); inside tmux also `tmux load-buffer -`.
Return which path was used; show a short toast ("copied 42 chars · wl-copy").
Wire the existing `copy_on_select` setting into `/config` for the live shell.
Tests: backend selection is a pure function of env (`SSH_TTY`, `TMUX`,
`WAYLAND_DISPLAY`, `DISPLAY`, OS) and tool availability; OSC 52 payload and
tmux/screen wrapping bytes; dry-run path (`PI_COPY_DRY_RUN`) still honored.

**T6. Composer soft wrap and visual-row navigation.**
Files: `views/chrome.rs` (`composer`, `composer_view`), `editor.rs`
(`set_layout_width` already exists), `app.rs`.
Wrap logical lines at the composer width with grapheme/width awareness;
Up/Down move by visual row and fall through to history only on the first/last
visual row; keep the 8-row cap with internal scroll. Click in the prompt
places the cursor (needs T7's hit map).
Tests: long single-line prompt renders on N rows without `…`; Up on row 2 of a
wrapped line stays in the prompt; Up on row 1 recalls history; widths at 20,
80, 200 columns never overrun.

### P1 — mouse targets, scrolling, keymap and history

**T7. Hit-test framework and click targets.** Record clickable regions while
composing (`ComposedFrame.targets: Vec<(Rect, Target)>`). Targets: completion
rows (hover via 1003 only while a list is open), select/approval options,
multi-select toggles and submit, settings values, collapsed tool results,
`↑ N more` rows, composer text. Env `DAVINCI_DISABLE_MOUSE`,
`DAVINCI_DISABLE_MOUSE_CLICKS` mirror the reference modes (off / wheel only).

**T8. Hyperlinks and modifier-click.** Emit OSC 8 for URLs and file paths in
transcript cells (ratatui cells cannot carry OSC 8; write a post-pass that
wraps the affected cell runs, or render link text through a dedicated
`RawAnsi`-style span supported by the runtime). Ctrl-click (Cmd where
reported) opens via existing `open_browser`. Never link UNC paths.

**T9. Auto-follow, jump-to-bottom and sticky prompt header.** Pause follow when
scrolled up; floating `Jump to bottom · N new messages` row with the platform
key hint; dim one-row header of the last prompt above the view, clickable;
PgUp/PgDn half page, Ctrl+Home/End top/bottom; `autoScroll` setting; wheel
speed env `DAVINCI_SCROLL_SPEED` (0.25–20) and acceleration.

**T10. Keymap alignment (needs user decision before merge).** Proposed
reference defaults with DaVinci features moved, not removed:
Ctrl+T → todo checklist (thinking toggle to Alt+T), Ctrl+S → stash (sessions
stays on `/resume` and Ctrl+Alt+S), Ctrl+R → history search, Ctrl+G and
Ctrl+X Ctrl+E → external editor, Ctrl+B → background running task,
Ctrl+L → redraw, Ctrl+Z → suspend with mode re-assert on resume,
Ctrl+Enter / Ctrl+X Ctrl+S → send queued now, Ctrl+D delete-forward or
double-press exit within 800 ms, Esc Esc on empty → rewind. Add chord support
to `keybindings.rs`. User overrides in `keybindings.json` keep winning.
Migration note in `docs/ui/terminal-rebuild.md`.

**T11. Persistent per-project history and Ctrl+R dialog.** Append-only JSONL
per working directory under the agent dir, deduplicating consecutive repeats,
bounded size; dialog with scope cycling (session/project/all) and match
highlighting; pasted-content chips recalled with their content.

**T12. Transcript viewer.** Ctrl+O opens a read-only full transcript surface
with the pager keys from section 3.4, `/` search with n/N (reuse
`alt_screen_search.rs` matching), `{`/`}` prompt jumps, `[` writes the
expanded transcript to the main screen (leave alt screen, print, wait for
Esc/q, re-enter), `v` opens it in `$VISUAL`/`$EDITOR`. In-place tool-output
expansion moves to click (T7) and stays available on a separate key.

### P2 — footer state, capability awareness, polish

**T13. Focusable footer.** `Footer` focus owner: ↓ from an empty prompt enters
the footer row; ←/→ items, Enter opens (agents, jobs, workflows sheets), Esc
leaves, `x` stops/dismisses. Ctrl+T checklist band (max five rows).

**T14. PR badge.** Background `gh pr view --json` (or token-authenticated
GitHub REST through the existing HTTP client only if already present; no new
dependency), refreshed after `git push`/`gh pr` tool calls; OSC 8 link,
underline color by review state; hint text when `gh` is missing.

**T15. Capability probes and themes.** XTVERSION and OSC 11 queries at
startup (bounded timeouts, skipped on non-TTY/conhost); `auto` theme from the
background luminance and mode 2031 updates; add daltonized and ANSI-only
variants of `Theme::cc()`; modifyOtherKeys fallback when kitty keyboard is
absent so Shift+Enter works in xterm and VS Code.

**T16. Spinner and microinteraction parity.** 12-step glyph ping-pong, Ghostty
glyph set, stall color blend, measured frame period (capture first), toast
stack with timed dismissal.

**T17. Render cost.** Remove per-frame `Model` clones in `compose_frame`
(borrow a view struct), cache the composed frame for mouse hit tests instead
of recomposing per wheel event, and add a frame-time debug counter.
`DAVINCI_FULL_REPAINT=1` for ConPTY hosts; one-time tmux mouse/focus hints.

### P3 — not recommended now

**T18. Classic (main-screen) renderer.** Large, duplicates the conversation
pipeline, and conflicts with DaVinci's pinned composer and graph surfaces.
Revisit only if users ask for native scrollback; T12's `[` covers the main
need. Screen-reader mode can reuse T2's native cursor instead.

## 5a. Implementation status (branch `claude/dazzling-shannon-st9ndv`)

Decisions taken on "go ahead" without explicit answers: the recommended
defaults in section 8 (steady caret with a `caretBlink` setting; keymap left
for T10; no classic renderer).

| Task | Status | Notes |
| --- | --- | --- |
| T1 | Done, narrowed | Every frame is wrapped in mode 2026 (`DAVINCI_SYNC_OUTPUT=0` opts out). No startup DECRQM probe: terminals without the mode ignore it, and a probe reply would arrive through crossterm's input reader as stray keys. `DAVINCI_FULL_REPAINT=1` added. No new pacer: the host loops already draw at most once per 40 ms poll during a turn and immediately after a key. |
| T2 | Done | Steady painted caret; `caretBlink` setting; focus events (1004) hide it in a background window; the hidden cursor is parked on the caret each frame; `showHardwareCursor` / `DAVINCI_NATIVE_CURSOR=1` show the terminal's own cursor instead. |
| T3 | Done | `davinci/selection.rs`: selections started in the conversation are held in conversation rows, survive scrolling, copy rows scrolled out of view, and are painted with `Cc::selection_bg` (reversed under `NO_COLOR`). |
| T4 | Done | Double-click word (paths and URLs whole), triple-click row, word/row drag, drag past the edge scrolls, Shift+arrows/Home/End extend, Ctrl+Shift+C copies, Ctrl+C copies instead of interrupting, Esc/PgUp/PgDn/modified motions keep the selection, other keys clear it. |
| T5 | Done | `clipboard.rs`: OSC 52 over SSH (tmux/screen wrapped, skipped for VTE and non-TTY), `pbcopy`, `wl-copy`, `xclip`, `xsel` (+ PRIMARY), PowerShell `Set-Clipboard` before `clip` on Windows/WSL, tmux buffer. Footer toast names the path. `fullscreenCopyOnSelect` now reaches the live shell. `/copy` reports a failed copy. |
| T13 | Partly done | Down from an empty prompt focuses the footer's background items (jobs, agents, workflows); ←/→ choose, Enter opens `/jobs` `/agents` `/workflows`, Esc/Up back, any other key leaves. Not done: `x` to stop an item, the Ctrl+T checklist band. |
| T14 | Done | `gh pr view --json` on the dresser thread (once a minute per branch; `DAVINCI_DISABLE_PR_BADGE` opts out); footer shows `PR #n` (merged/closed/draft), inked by review state, OSC 8-linked. No hint when `gh` is missing. |
| T15 | Partly done | Themes `auto` (from `COLORFGBG`), `ansi`, and a `-daltonized` suffix. Not done: XTVERSION/OSC 11 queries and modifyOtherKeys (replies would arrive as stray keys through crossterm). |
| T16 | Partly done | Toasts queue instead of overwriting one another. Not done: glyph ping-pong and Ghostty set (needs a measured capture). |
| T17 | Partly done | Pointer hits are tested against the last drawn frame (`Model::hit_cache`) instead of composing a frame per click or wheel notch; `DAVINCI_FRAME_STATS=1` shows a moving-average frame time in the corner. Not done: removing the `Model` clone while an instrument is open (needs a view struct), the cached frame for wheel scrolling, one-time tmux hints. |
| T6 | Done | The composer word-wraps at the window width using the shared editor's own visual-row map, so drawing and Up/Down agree. |
| T7 | Done, narrowed | `ComposedFrame.targets` + `Model::hit_cache`: jump row, completion rows, history-search rows, prompt header, composer text (places the caret), options of an open question (a tool approval only moves focus), settings rows (focus + cycle), folded tool results (open the output). `DAVINCI_DISABLE_MOUSE` / `DAVINCI_DISABLE_MOUSE_CLICKS`. No multi-select exists in this shell, so no toggles; `↑ N more` has no such row here. Other instruments' rows are not clickable. |
| T8 | Done | `links.rs` finds URLs and existing paths per row; after each frame the runtime reprints those cells inside OSC 8 (`DAVINCI_DISABLE_HYPERLINKS` opts out). Ctrl/Cmd-click opens web links via `open_browser`; a `file://` link opens its folder, never the file (the OS opener would run an executable or script that model output merely names). Windows drive paths link. `//` and `\\` paths are never linked. |
| T9 | Done, narrowed | Jump-to-bottom row with new-message count (clickable), sticky prompt header (clickable), Ctrl+Home/End, `DAVINCI_SCROLL_SPEED`, `autoScroll` setting (off pins the view while output streams). No wheel acceleration. |
| T10 | Done, narrowed | Ctrl+O transcript viewer (Alt+O expands tool output in place), Ctrl+R history, Ctrl+L repaint, Ctrl+Z suspend (unix), Ctrl+D twice within 800 ms to exit. Left as they were because the feature behind the reference key does not exist here: Ctrl+T (thinking toggle, no checklist yet), Ctrl+S (sessions, no stash), Ctrl+B, Ctrl+Enter, chords (Ctrl+X Ctrl+E; the external editor is not wired in this shell). User overrides in `keybindings.json` still win. |
| T11 | Mostly done | Per-project JSONL history (bounded by entries and bytes), loaded at startup; Ctrl+R search with Tab cycling session / project / all projects, first-word highlight, click or Enter to take. Pasted content is recalled as its expanded text, not as a chip. |
| T12 | Done | Ctrl+O viewer over a snapshot with every tool call expanded: j/k, space/b, d/u, g/G, `/` with n/N, `{`/`}` prompts, `[` to scrollback, `v` to `$VISUAL`/`$EDITOR`, q/esc. |

## 6. Recommended order and sizing

| Order | Tasks | Rough size | Why first |
| --- | --- | --- | --- |
| 1 | T1, T2 | small | Flicker and caret are felt on every keystroke; low risk |
| 2 | T3, T4, T5 | medium | Selection/copy is the most visible fullscreen behavior and today copies wrong text after scrolling |
| 3 | T6 | medium | Long prompts are currently clipped |
| 4 | T7, T9 | medium | Unlocks clicks everywhere and scroll ergonomics |
| 5 | T10 (after decision), T11, T12 | medium–large | Keymap change is user-visible; history/search depend on it |
| 6 | T13–T17 | medium | Polish and capability awareness |

## 7. Validation plan

- Unit/buffer tests per task as listed; existing "never overruns the window",
  `NO_COLOR`, Unicode caret and focus-routing tests must stay green.
- Byte-level tests for every new escape sequence and its disable path.
- Manual terminal matrix before claiming compatibility (not yet run): Windows
  Terminal, conhost, VS Code terminal, iTerm2, Ghostty, kitty, WezTerm,
  GNOME Terminal (VTE), tmux 3.4/3.6, SSH. Each row: flicker during streaming,
  wheel, drag/double/triple click, copy path reported by the toast, IME
  candidate position, focus caret, Shift+Enter.
- Reference comparison: a new capture phase for interaction (mouse and focus
  sequences) needs a real terminal or a PTY harness that forwards SGR mouse;
  `pyte` replay cannot validate this layer.

## 8. Open decisions for the user

1. **Keymap (T10):** adopt the reference defaults and move DaVinci's
   Ctrl+T (thinking), Ctrl+S (sessions) and Ctrl+Alt instrument keys, or keep
   DaVinci's and document the differences?
2. **Caret blink (T2):** reference is steady; default DaVinci to steady with a
   setting for blink, or keep blinking?
3. **Classic renderer (T18):** confirm it stays out of scope.
