//! Terminal themes. The default dark theme is the da Vinci palette described
//! in the davinci TUI design spec: copper carries state, verdigris carries
//! location (git, paths), and every state also has a glyph so `NO_COLOR`
//! still reads. Custom theme JSON files without a `palette` keep the legacy
//! 16-color rendering.

use serde::{Deserialize, Serialize};

/// State glyph vocabulary (design spec §4). Color reinforces, never replaces.
pub mod glyphs {
    pub const DONE: &str = "✓";
    pub const ACTIVE: &str = "◉";
    pub const QUEUED: &str = "○";
    pub const SKIPPED: &str = "◌";
    pub const FAILED: &str = "×";
    pub const ATTENTION: &str = "!";
    pub const DELTA: &str = "Δ";
    pub const READ: &str = "↳";
    pub const SEARCH: &str = "⌕";
    pub const AGENT: &str = "◆";
    pub const PROMPT: &str = "›";
    pub const TICK: &str = "·";
    pub const SPINNER_FRAMES: [&str; 4] = ["◜", "◝", "◞", "◟"];
    pub const METER_FILLED: &str = "━";
    pub const METER_TIP: &str = "╸";
    pub const METER_EMPTY: &str = "─";
}

/// Truecolor ramp for a theme. Optional: legacy themes render with basic ANSI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub surface: String,
    pub border: String,
    pub text: String,
    pub muted: String,
    pub primary: String,
    pub secondary: String,
    pub success: String,
    pub warning: String,
    pub error: String,
    /// Dimmed strokes: keybind hints, inert glyphs, rules.
    pub dim: String,
}

impl Palette {
    /// Design spec §2 truecolor values.
    pub fn da_vinci() -> Self {
        Self {
            surface: "#101719".into(),
            border: "#453A27".into(),
            text: "#DDD5C4".into(),
            muted: "#80796D".into(),
            primary: "#D58A32".into(),
            secondary: "#52A89C".into(),
            success: "#74A879".into(),
            warning: "#D5A047".into(),
            error: "#C4593F".into(),
            dim: "#5d564c".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub accent: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<Palette>,
}

impl Default for Theme {
    fn default() -> Self {
        builtin_themes().into_iter().next().expect("builtin theme")
    }
}

/// `NO_COLOR` (https://no-color.org) drops every color code; state still
/// reads through glyphs. Read per call so tests can toggle it.
pub fn no_color() -> bool {
    std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty())
}

/// Serializes tests that flip the `NO_COLOR` process environment.
#[cfg(test)]
pub(crate) static NO_COLOR_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn truecolor(hex: &str) -> Option<(u8, u8, u8)> {
    let hex = hex.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    Some((
        u8::from_str_radix(&hex[0..2], 16).ok()?,
        u8::from_str_radix(&hex[2..4], 16).ok()?,
        u8::from_str_radix(&hex[4..6], 16).ok()?,
    ))
}

impl Theme {
    fn paint(&self, hex: &str, text: &str, bold: bool) -> String {
        if no_color() {
            return if bold {
                format!("\x1b[1m{text}\x1b[22m")
            } else {
                text.to_string()
            };
        }
        match truecolor(hex) {
            Some((r, g, b)) if bold => {
                format!("\x1b[1;38;2;{r};{g};{b}m{text}\x1b[22;39m")
            }
            Some((r, g, b)) => format!("\x1b[38;2;{r};{g};{b}m{text}\x1b[39m"),
            None => text.to_string(),
        }
    }

    pub fn fg(&self, role: &str, text: &str) -> String {
        if let Some(palette) = &self.palette {
            let hex = match role {
                "accent" | "primary" | "copper" => &palette.primary,
                "secondary" | "verdigris" | "mdLink" | "path" => &palette.secondary,
                "text" | "customMessageText" => &palette.text,
                "muted" => &palette.muted,
                "dim" | "borderMuted" => &palette.dim,
                "border" => &palette.border,
                "success" => &palette.success,
                "warning" => &palette.warning,
                "error" => &palette.error,
                "borderAccent" | "customMessageLabel" => &palette.primary,
                "mdHeading" => return self.paint(&palette.text, text, true),
                // Returning the escape directly would step around `paint`,
                // which is where the NO_COLOR check lives — the one role that
                // could still emit colour with colour turned off.
                "searchMatchText" if no_color() => return text.to_string(),
                "searchMatchText" => return format!("\x1b[30m{text}\x1b[39m"),
                _ => return text.to_string(),
            };
            return self.paint(hex, text, false);
        }
        if no_color() {
            return text.to_string();
        }
        match role {
            "accent" | "primary" => format!("\x1b[36m{text}\x1b[39m"),
            "secondary" => format!("\x1b[36m{text}\x1b[39m"),
            "mdHeading" => format!("\x1b[1m{text}\x1b[22m"),
            "mdLink" => format!("\x1b[4;36m{text}\x1b[39;24m"),
            "muted" | "dim" | "border" => format!("\x1b[2m{text}\x1b[22m"),
            "warning" => format!("\x1b[33m{text}\x1b[39m"),
            "success" => format!("\x1b[32m{text}\x1b[39m"),
            "error" => format!("\x1b[31m{text}\x1b[39m"),
            "customMessageLabel" => format!("\x1b[35m{text}\x1b[39m"),
            "customMessageText" => text.to_string(),
            "searchMatchText" => format!("\x1b[30m{text}\x1b[39m"),
            "borderMuted" | "borderAccent" => format!("\x1b[2m{text}\x1b[22m"),
            _ => text.to_string(),
        }
    }

    pub fn bold(&self, text: &str) -> String {
        format!("\x1b[1m{text}\x1b[22m")
    }

    pub fn underline(&self, text: &str) -> String {
        format!("\x1b[4m{text}\x1b[24m")
    }

    pub fn inverse(&self, text: &str) -> String {
        format!("\x1b[7m{text}\x1b[27m")
    }

    pub fn bg(&self, role: &str, text: &str) -> String {
        if no_color() {
            return match role {
                "selectedBg" => format!("\x1b[7m{text}\x1b[27m"),
                _ => text.to_string(),
            };
        }
        match role {
            "customMessageBg" => format!("\x1b[45m{text}\x1b[49m"),
            "selectedBg" => format!("\x1b[7m{text}\x1b[27m"),
            "searchMatchBg" => format!("\x1b[43m{text}\x1b[49m"),
            _ => text.to_string(),
        }
    }

    /// Instrument panel (design spec §3): a framed block whose label is
    /// notched into the top rule, uppercase and letter-spaced.
    ///
    /// ```text
    /// ╭─ MENSURA · TOKEN GOVERNOR ────────╮
    /// │ rows …                            │
    /// ╰───────────────────────────────────╯
    /// ```
    pub fn panel(
        &self,
        label: &str,
        sublabel: Option<&str>,
        accent_role: &str,
        body: &[String],
        width: usize,
    ) -> Vec<String> {
        let width = width.clamp(20, 120);
        let inner = width.saturating_sub(2);
        let mut title_plain = format!(" {} ", label.to_uppercase());
        let mut title = format!(" {} ", self.fg(accent_role, &label.to_uppercase()));
        if let Some(sub) = sublabel.filter(|value| !value.is_empty()) {
            title_plain = format!(" {} · {} ", label.to_uppercase(), sub.to_uppercase());
            title = format!(
                " {} {} {} ",
                self.fg(accent_role, &label.to_uppercase()),
                self.fg("border", "·"),
                self.fg("muted", &sub.to_uppercase())
            );
        }
        let title_width = crate::render::visible_width(&title_plain);
        let rest = inner.saturating_sub(title_width + 1);
        let mut lines = vec![format!(
            "{}{}{}{}",
            self.fg("border", "╭─"),
            title,
            self.fg("border", &"─".repeat(rest)),
            self.fg("border", "╮")
        )];
        for row in body {
            let row_width = crate::render::visible_width_stripped(row);
            let pad = inner.saturating_sub(row_width + 2);
            lines.push(format!(
                "{} {row}{}{}",
                self.fg("border", "│"),
                " ".repeat(pad + 1),
                self.fg("border", "│")
            ));
        }
        lines.push(format!(
            "{}{}{}",
            self.fg("border", "╰"),
            self.fg("border", &"─".repeat(inner)),
            self.fg("border", "╯")
        ));
        lines
    }

    /// Proportion meter: `━━━━╸────` (design spec: meters, not bare numbers).
    pub fn meter(&self, used: u64, total: u64, cells: usize) -> String {
        let cells = cells.max(2);
        let ratio = if total == 0 {
            0.0
        } else {
            (used as f64 / total as f64).clamp(0.0, 1.0)
        };
        let filled = (ratio * cells as f64).round() as usize;
        let filled = filled.min(cells);
        let mut bar = String::new();
        if filled > 0 {
            bar.push_str(&glyphs::METER_FILLED.repeat(filled.saturating_sub(1)));
            bar.push_str(glyphs::METER_TIP);
        }
        let empty = cells - filled;
        let rest = glyphs::METER_EMPTY.repeat(empty);
        format!("{}{}", self.fg("primary", &bar), self.fg("border", &rest))
    }
}

pub fn builtin_themes() -> Vec<Theme> {
    vec![
        Theme {
            name: "dark".into(),
            background: "#0B1011".into(),
            foreground: "#DDD5C4".into(),
            accent: "#D58A32".into(),
            palette: Some(Palette::da_vinci()),
        },
        Theme {
            name: "light".into(),
            background: "#f8f8f8".into(),
            foreground: "#1e1e1e".into(),
            accent: "#2e6da4".into(),
            palette: None,
        },
        Theme {
            name: "pi".into(),
            background: "#16161e".into(),
            foreground: "#c0caf5".into(),
            accent: "#7dcfff".into(),
            palette: None,
        },
    ]
}

pub fn load_themes_from_dir(dir: &std::path::Path) -> Vec<Theme> {
    let mut themes = builtin_themes();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return themes;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(theme) = serde_json::from_str::<Theme>(&raw) {
            if let Some(existing) = themes.iter_mut().find(|item| item.name == theme.name) {
                *existing = theme;
            } else {
                themes.push(theme);
            }
        }
    }
    themes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_themes_from_dir_overlays_json() {
        let dir = tempfile::tempdir().expect("temp");
        std::fs::write(
            dir.path().join("custom.json"),
            r##"{"name":"custom","background":"#000","foreground":"#fff","accent":"#f80"}"##,
        )
        .expect("write");
        std::fs::write(
            dir.path().join("dark.json"),
            r##"{"name":"dark","background":"#111111","foreground":"#eeeeee","accent":"#abcdef"}"##,
        )
        .expect("write");
        let themes = load_themes_from_dir(dir.path());
        assert!(themes.iter().any(|theme| theme.name == "custom"));
        let dark = themes
            .iter()
            .find(|theme| theme.name == "dark")
            .expect("dark");
        assert_eq!(dark.background, "#111111");
        // A JSON overlay without a palette drops back to legacy rendering.
        assert!(dark.palette.is_none());
    }

    #[test]
    fn da_vinci_palette_paints_truecolor_roles() {
        let _guard = NO_COLOR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("NO_COLOR");
        let theme = builtin_themes().into_iter().next().unwrap();
        assert!(theme.palette.is_some());
        let copper = theme.fg("primary", "x");
        assert!(copper.contains("38;2;213;138;50"), "{copper:?}");
        let verdigris = theme.fg("secondary", "x");
        assert!(verdigris.contains("38;2;82;168;156"), "{verdigris:?}");
        // Legacy role names stay mapped.
        assert!(theme.fg("accent", "x").contains("38;2;213;138;50"));
        assert!(theme.fg("muted", "x").contains("38;2;128;121;109"));
    }

    #[test]
    fn no_color_strips_codes_and_meter_scales() {
        let _guard = NO_COLOR_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("NO_COLOR", "1");
        let theme = builtin_themes().into_iter().next().unwrap();
        assert_eq!(theme.fg("primary", "plain"), "plain");
        assert_eq!(theme.fg("error", "plain"), "plain");
        // Every role, not most of them: `searchMatchText` used to return its
        // escape before the check that strips colour.
        for role in [
            "accent",
            "primary",
            "copper",
            "secondary",
            "verdigris",
            "mdLink",
            "path",
            "text",
            "customMessageText",
            "muted",
            "dim",
            "borderMuted",
            "border",
            "success",
            "warning",
            "error",
            "borderAccent",
            "customMessageLabel",
            "mdHeading",
            "searchMatchText",
        ] {
            let painted = theme.fg(role, "plain");
            assert!(
                !painted.contains("\x1b[3"),
                "{role} still emits colour under NO_COLOR: {painted:?}"
            );
        }
        std::env::remove_var("NO_COLOR");
        let meter = theme.meter(47, 200, 12);
        let stripped = crate::ansi::strip_terminal_sequences(&meter);
        assert_eq!(stripped.chars().count(), 12, "{stripped:?}");
        assert!(stripped.contains('╸'));
        let empty = crate::ansi::strip_terminal_sequences(&theme.meter(0, 200, 12));
        assert!(!empty.contains('━'));
    }
}
