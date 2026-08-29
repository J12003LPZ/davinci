use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub background: String,
    pub foreground: String,
    pub accent: String,
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
