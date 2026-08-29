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
            let action = migrate_keybinding_name(action);
            let parsed = match keys {
                serde_json::Value::String(key) => vec![key.clone()],
                serde_json::Value::Array(items) => items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect(),
                serde_json::Value::Null => Vec::new(),
                _ => continue,
            };
            bindings.bindings.insert(action, parsed);
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

fn migrate_keybinding_name(name: &str) -> String {
    match name {
        "interrupt" => "app.interrupt",
        "clear" => "app.clear",
        "exit" => "app.exit",
        "submit" => "tui.input.submit",
        "tab" => "tui.input.tab",
        other => other,
    }
    .to_string()
}

fn default_pairs() -> &'static [(&'static str, &'static [&'static str])] {
    &[
        ("app.interrupt", &["escape"]),
        ("app.clear", &["ctrl+c"]),
        ("app.exit", &["ctrl+d"]),
        ("app.suspend", &["ctrl+z"]),
        ("app.thinking.cycle", &["shift+tab"]),
        ("app.model.cycleForward", &["ctrl+p"]),
        ("app.model.cycleBackward", &["shift+ctrl+p"]),
        ("app.model.select", &["ctrl+l"]),
        ("app.tools.expand", &["ctrl+o"]),
        ("app.thinking.toggle", &["ctrl+t"]),
        ("app.editor.external", &["ctrl+g"]),
        ("app.message.copy", &["ctrl+x"]),
        ("app.message.followUp", &["alt+enter"]),
        ("app.message.dequeue", &["alt+up"]),
        ("app.clipboard.pasteImage", &["ctrl+v"]),
        ("app.session.new", &[]),
        ("app.session.tree", &[]),
        ("app.session.fork", &[]),
        ("app.session.resume", &[]),
        ("app.session.togglePath", &["ctrl+p"]),
        ("app.session.toggleSort", &["ctrl+s"]),
        ("app.session.rename", &["ctrl+r"]),
        ("app.session.toggleNamedFilter", &["ctrl+n"]),
        ("app.session.delete", &["ctrl+d"]),
        ("app.session.deleteNoninvasive", &["ctrl+backspace"]),
        ("app.tree.foldOrUp", &["ctrl+left", "alt+left"]),
        ("app.tree.unfoldOrDown", &["ctrl+right", "alt+right"]),
        ("app.tree.editLabel", &["shift+l"]),
        ("app.tree.toggleLabelTimestamp", &["shift+t"]),
        ("app.tree.filter.default", &["ctrl+d"]),
        ("app.tree.filter.noTools", &["ctrl+t"]),
        ("app.tree.filter.userOnly", &["ctrl+u"]),
        ("app.tree.filter.labeledOnly", &["ctrl+l"]),
        ("app.tree.filter.all", &["ctrl+a"]),
        ("app.tree.filter.cycleForward", &["ctrl+o"]),
        ("app.tree.filter.cycleBackward", &["shift+ctrl+o"]),
        ("app.models.save", &["ctrl+s"]),
        ("app.models.enableAll", &["ctrl+a"]),
        ("app.models.clearAll", &["ctrl+x"]),
        ("app.models.toggleProvider", &["ctrl+p"]),
        ("app.models.reorderUp", &["alt+up"]),
        ("app.models.reorderDown", &["alt+down"]),
        ("tui.input.submit", &["enter"]),
        ("tui.input.newLine", &["shift+enter"]),
        ("tui.input.tab", &["tab"]),
        ("tui.select.up", &["up"]),
        ("tui.select.down", &["down"]),
        ("tui.select.confirm", &["enter"]),
        ("tui.select.cancel", &["escape"]),
    ]
}

pub fn key_to_bytes(key: &str) -> String {
    match key {
        "ctrl+a" => "\x01".into(),
        "ctrl+c" => "\x03".into(),
        "ctrl+d" => "\x04".into(),
        "ctrl+e" => "\x05".into(),
        "ctrl+f" => "\x06".into(),
        "ctrl+g" => "\x07".into(),
        "ctrl+l" => "\x0c".into(),
        "ctrl+n" => "\x0e".into(),
        "ctrl+o" => "\x0f".into(),
        "ctrl+p" => "\x10".into(),
        "ctrl+q" => "\x11".into(),
        "ctrl+r" => "\x12".into(),
        "ctrl+s" => "\x13".into(),
        "ctrl+t" => "\x14".into(),
        "ctrl+u" => "\x15".into(),
        "ctrl+v" => "\x16".into(),
        "ctrl+x" => "\x18".into(),
        "ctrl+z" => "\x1a".into(),
        "ctrl+backspace" => "\x1b[3;5~".into(),
        "shift+tab" => "\x1b[Z".into(),
        "shift+ctrl+p" => "\x1b[80;6u".into(),
        "shift+ctrl+o" => "\x1b[79;6u".into(),
        "alt+enter" => "\x1b\r".into(),
        "alt+up" => "\x1b[1;3A".into(),
        "alt+down" => "\x1b[1;3B".into(),
        "alt+p" => "\x1bp".into(),
        "alt+q" => "\x1bq".into(),
        "alt+v" => "\x1bv".into(),
        "ctrl+left" => "\x1b[1;5D".into(),
        "alt+left" => "\x1b[1;3D".into(),
        "ctrl+right" => "\x1b[1;5C".into(),
        "alt+right" => "\x1b[1;3C".into(),
        "shift+l" => "L".into(),
        "shift+t" => "T".into(),
        "escape" => "\x1b".into(),
        "enter" => "\r".into(),
        "tab" => "\t".into(),
        "up" => "\x1b[A".into(),
        "down" => "\x1b[B".into(),
        "shift+enter" => "\n".into(),
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
        assert!(Keybindings::defaults().matches("\x0c", "app.model.select"));
        assert!(Keybindings::defaults().matches("\x0f", "app.tools.expand"));
        assert!(Keybindings::defaults().matches("\x1b[Z", "app.thinking.cycle"));
        let migrated = Keybindings::from_json(r#"{"clear":"ctrl+u"}"#);
        assert!(migrated.matches("\x15", "app.clear"));
    }
}
