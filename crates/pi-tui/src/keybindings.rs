//! App keybindings matching TS `keybindings.ts` + `keybindings.json`.

use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybindings {
    bindings: HashMap<String, Vec<String>>,
}

impl Default for Keybindings {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Keybindings {
    pub fn defaults() -> Self {
        let mut bindings = HashMap::new();
        for (action, keys) in default_pairs() {
            bindings.insert(
                action.to_string(),
                keys.iter().map(|key| (*key).to_string()).collect(),
            );
        }
        Self { bindings }
    }

    pub fn load(agent_dir: &Path) -> Self {
        let path = agent_dir.join("keybindings.json");
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::defaults();
        };
        Self::from_json(&raw)
    }

    pub fn from_json(raw: &str) -> Self {
        let mut bindings = Self::defaults();
        let Ok(value) = serde_json::from_str::<serde_json::Value>(raw) else {
            return bindings;
        };
        let Some(object) = value.as_object() else {
            return bindings;
        };
        for (action, keys) in object {
            let parsed = match keys {
                serde_json::Value::String(key) => vec![key.clone()],
                serde_json::Value::Array(items) => items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect(),
                serde_json::Value::Null => Vec::new(),
                _ => continue,
            };
            bindings.bindings.insert(action.clone(), parsed);
        }
        bindings
    }

    pub fn keys_for(&self, action: &str) -> &[String] {
        self.bindings.get(action).map(Vec::as_slice).unwrap_or(&[])
    }

    pub fn matches(&self, data: &str, action: &str) -> bool {
        self.keys_for(action)
            .iter()
            .any(|key| key_to_bytes(key) == data)
    }
}

fn default_pairs() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("app.editor.external", &["ctrl+g"]),
        ("app.message.copy", &["ctrl+x"]),
        ("app.message.followUp", &["alt+enter"]),
        ("app.message.dequeue", &["alt+up"]),
        ("app.clipboard.pasteImage", &["ctrl+v"]),
        ("app.session.togglePath", &["ctrl+p"]),
        ("app.session.toggleSort", &["ctrl+s"]),
        ("app.session.rename", &["ctrl+r"]),
        ("app.tree.foldOrUp", &["ctrl+left", "alt+left"]),
        ("app.tree.unfoldOrDown", &["ctrl+right", "alt+right"]),
        ("app.tree.editLabel", &["shift+l"]),
        ("app.tree.toggleLabelTimestamp", &["shift+t"]),
    ]
}

pub fn key_to_bytes(key: &str) -> String {
    match key {
        "ctrl+e" => "\x05".into(),
        "ctrl+g" => "\x07".into(),
        "ctrl+v" => "\x16".into(),
        "ctrl+x" => "\x18".into(),
        "ctrl+q" => "\x11".into(),
        "ctrl+p" => "\x10".into(),
        "ctrl+s" => "\x13".into(),
        "ctrl+r" => "\x12".into(),
        "alt+enter" => "\x1b\r".into(),
        "alt+up" => "\x1b[1;3A".into(),
        "alt+q" => "\x1bq".into(),
        "alt+v" => "\x1bv".into(),
        "ctrl+left" => "\x1b[1;5D".into(),
        "alt+left" => "\x1b[1;3D".into(),
        "ctrl+right" => "\x1b[1;5C".into(),
        "alt+right" => "\x1b[1;3C".into(),
        "shift+l" => "L".into(),
        "shift+t" => "T".into(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_user_overrides_from_json() {
        let bindings = Keybindings::from_json(
            r#"{"app.editor.external":"ctrl+e","app.message.followUp":["ctrl+q"]}"#,
        );
        assert!(bindings.matches("\x05", "app.editor.external"));
        assert!(bindings.matches("\x11", "app.message.followUp"));
        assert!(!bindings.matches("\x07", "app.editor.external"));
        assert!(Keybindings::defaults().matches("\x07", "app.editor.external"));
        assert!(Keybindings::defaults().matches("\x1b\r", "app.message.followUp"));
        assert!(Keybindings::defaults().matches("\x16", "app.clipboard.pasteImage"));
    }
}
