# Phase 6 — `pi-tui`

## Goal

Port the TUI component contract used by interactive coding-agent.

## TypeScript sources

- `vendor/pi/packages/tui/src/tui.ts`
- `vendor/pi/packages/tui/src/components/`

## Deliverables

- `Component` trait: `render(width) -> Vec<String>`.
- `Container` vertical composition.
- `Editor` multiline input with cursor marker `\x1b_pi:c\x07`.
- Key parse for Enter / Ctrl-C / printable.
- Snapshot tests at fixed widths.

## Done when

`cargo test -p pi-tui` snapshot tests pass.
