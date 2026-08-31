//! The davinci terminal UI, built on ratatui.
//!
//! Specification: `docs/ui/design.md`. Mockups: `docs/ui/Pi TUI Mockups.dc.html`
//! (screens `1a`–`1h`, `2a`–`2c`). Reference implementation, in Elixir /
//! Ratatouille: `docs/ui/davinci_tui/`.
//!
//! The terminal is a notebook, not a dashboard. One panel at a time, color is
//! never the only signal, and nothing animates that the user is reading.

pub mod app;
pub mod fixtures;
pub mod model;
pub mod runtime;
pub mod term;
pub mod theme;
pub mod ui;
pub mod views;

use model::Model;
use theme::Theme;

/// Build a model for this terminal: negotiated color depth, `NO_COLOR`, and
/// `--no-animation` all resolved once, here.
pub fn boot(args: &[String], width: u16, height: u16) -> Model {
    let theme = Theme::da_vinci(term::detect_color_depth(), term::no_color());
    Model::new(theme, width, height, term::animate(args))
}
