use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub accent: String,
}

impl Theme {
    pub fn fg(&self, role: &str, text: &str) -> String {
        match role {
            "accent" => format!("\x1b[36m{text}\x1b[39m"),
            "muted" | "dim" => format!("\x1b[2m{text}\x1b[22m"),
            "warning" => format!("\x1b[33m{text}\x1b[39m"),
            "success" => format!("\x1b[32m{text}\x1b[39m"),
            "error" => format!("\x1b[31m{text}\x1b[39m"),
            "customMessageLabel" => format!("\x1b[35m{text}\x1b[39m"),
            "customMessageText" => text.to_string(),
            "borderMuted" | "borderAccent" => format!("\x1b[2m{text}\x1b[22m"),
            _ => text.to_string(),
        }
    }

    pub fn bold(&self, text: &str) -> String {
        format!("\x1b[1m{text}\x1b[22m")
    }

    pub fn bg(&self, role: &str, text: &str) -> String {
        match role {
            "customMessageBg" => format!("\x1b[45m{text}\x1b[49m"),
            "selectedBg" => format!("\x1b[7m{text}\x1b[27m"),
            _ => text.to_string(),
        }
    }
}

pub fn builtin_themes() -> Vec<Theme> {
    vec![
        Theme {
            name: "dark".into(),
            background: "#1e1e1e".into(),
            foreground: "#e6e6e6".into(),
            accent: "#7aa2f7".into(),
        },
        Theme {
            name: "light".into(),
            background: "#f8f8f8".into(),
            foreground: "#1e1e1e".into(),
            accent: "#2e6da4".into(),
        },
        Theme {
            name: "pi".into(),
            background: "#16161e".into(),
            foreground: "#c0caf5".into(),
            accent: "#7dcfff".into(),
        },
    ]
}
