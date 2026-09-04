# davinci-tui

`davinci-tui` is the terminal user interface engine for Davinci. Built on top of [Ratatui](https://github.com/ratatui/ratatui) and Crossterm, it delivers a high-fidelity, responsive, and visually consistent terminal interface.

---

## Design System & Architecture

The TUI implements the design system documented in [`docs/ui/design.md`](../../docs/ui/design.md) across two primary canvases:
1. **Transcript Mockups (`1a`–`2c`)**: Message turn streams, tool call accordions, streaming markdown output, thinking indicators, and error callouts.
2. **Instruments & Command Sheets (`3a`–`6d`)**: Modal sheets sharing the unified `SheetChrome` descriptor (`views/sheet.rs`):
   - Header facts (context, repository, session)
   - Status third (interactive tables, lists, or selections)
   - Hint row (keyboard shortcuts and navigation hints)
   - Composer & Echo row

### Tenets & Visual Rules
- **One Panel at a Time**: Modals take center focus with clear ESC/Enter exit chords.
- **Glyph Redundancy**: Every status indicator includes a descriptive glyph so the interface remains fully readable under `NO_COLOR`.
- **Single Source of Truth for Color**: All color values are defined in [`src/davinci/theme.rs`](src/davinci/theme.rs).
- **Prose & Typography**: Text wraps strictly at 74 columns via ICU word segmenters.
- **Meters & Gauges**: Numeric stats (tokens, context %, memory) are presented as calibrated gauges with clear units and caps.

---

## Sheet & View Inventory

| View ID | Command / Screen | File |
| :--- | :--- | :--- |
| `1a`–`2c` | Main transcript & interaction loop | `src/davinci/views/stream.rs`, `turn.rs` |
| `3a` | `/model` selection sheet | `src/davinci/views/model.rs` |
| `3b` | `/settings` toggle sheet | `src/davinci/views/settings.rs` |
| `3c` | `/thinking` budget sheet | `src/davinci/views/thinking.rs` |
| `3d` | `/login` authentication sheet | `src/davinci/views/login.rs` |
| `4a` | `/hotkeys` help sheet | `src/davinci/views/keys.rs` |
| `4b` | `/resume` session picker | `src/davinci/views/resume.rs` |
| `4c` | `/tree` session history tree | `src/davinci/views/tree.rs` |
| `5a` | `/graph` multi-worker graph runner | `src/davinci/views/graph_run.rs` |
| `5b` | `memory-status` vector memory meter | `src/davinci/views/mensura.rs` |
| `5c` | `governor-status` token governor meter | `src/davinci/views/governor.rs` |
| `5d` | `sec-report` security scan findings | `src/davinci/views/securitas.rs` |
| `6a` | `/diff` git change review inspector | `src/davinci/views/diff.rs` |
| `6b` | `/permissions` rule editor sheet | `src/davinci/views/ask.rs` |
| `6c` | `/mcp` server & tool inspector | `src/davinci/views/mcp.rs` |

---

## Testing & Headless Inspection

```bash
# Run unit tests
cargo test -p davinci-tui

# Dump all visual screen mockups for visual regression audit
cargo test -p davinci-tui dump_every_screen_for_the_mockup_audit -- --ignored --nocapture
```
