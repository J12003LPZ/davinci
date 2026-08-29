use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub accent: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            name: "dark".into(),
            background: "#1e1e1e".into(),
            foreground: "#e6e6e6".into(),
            accent: "#7aa2f7".into(),
        }
    }
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
        match role {
            "customMessageBg" => format!("\x1b[45m{text}\x1b[49m"),
            "selectedBg" => format!("\x1b[7m{text}\x1b[27m"),
            "searchMatchBg" => format!("\x1b[43m{text}\x1b[49m"),
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
    }
}
