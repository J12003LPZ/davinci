use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Theme {
    pub name: String,
    pub accent: String,
    pub background: String,
    pub foreground: String,
    pub error: String,
    pub warning: String,
}

impl Theme {
    pub fn default_dark() -> Self {
        Self {
            name: "dark".into(),
            accent: "#7aa2f7".into(),
            background: "#1a1b26".into(),
            foreground: "#c0caf5".into(),
            error: "#f7768e".into(),
            warning: "#e0af68".into(),
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }
}
