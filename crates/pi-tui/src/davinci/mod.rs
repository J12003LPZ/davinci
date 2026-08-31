//! The davinci terminal UI, built on ratatui.
//!
//! Specification: `docs/ui/design.md`. Mockups: `docs/ui/Pi TUI Mockups.dc.html`
//! (screens `1a`–`1h`, `2a`–`2c`). Reference implementation, in Elixir /
//! Ratatouille: `docs/ui/davinci_tui/`.
//!
//! The terminal is a notebook, not a dashboard. One panel at a time, color is
//! never the only signal, and nothing animates that the user is reading.

pub mod term;
pub mod theme;
pub mod ui;
