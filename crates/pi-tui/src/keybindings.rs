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
            .any(|key| sequence_matches_key(data, key))
    }

    pub fn bindings(&self) -> impl Iterator<Item = (&str, &[String])> {
        self.bindings
            .iter()
            .map(|(action, keys)| (action.as_str(), keys.as_slice()))
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
        ("tui.editor.cursorUp", &["up"]),
        ("tui.editor.cursorDown", &["down"]),
        ("tui.editor.historyPrevious", &[]),
        ("tui.editor.historyNext", &[]),
        ("tui.editor.pageUp", &["pageUp", "ctrl+pageUp"]),
        ("tui.editor.pageDown", &["pageDown", "ctrl+pageDown"]),
        ("tui.editor.cursorLeft", &["left", "ctrl+b"]),
        ("tui.editor.cursorRight", &["right", "ctrl+f"]),
        (
            "tui.editor.cursorWordLeft",
            &["alt+left", "ctrl+left", "alt+b"],
        ),
        (
            "tui.editor.cursorWordRight",
            &["alt+right", "ctrl+right", "alt+f"],
        ),
        (
            "tui.editor.cursorLineStart",
            &["home", "ctrl+home", "ctrl+a"],
        ),
        ("tui.editor.cursorLineEnd", &["end", "ctrl+end", "ctrl+e"]),
        ("tui.editor.deleteCharBackward", &["backspace"]),
        ("tui.editor.deleteCharForward", &["delete", "ctrl+d"]),
        (
            "tui.editor.deleteWordBackward",
            &["ctrl+w", "alt+backspace"],
        ),
        ("tui.editor.deleteWordForward", &["alt+d", "alt+delete"]),
        ("tui.editor.deleteToLineStart", &["ctrl+u"]),
        ("tui.editor.deleteToLineEnd", &["ctrl+k"]),
        ("tui.editor.yank", &["ctrl+y"]),
        ("tui.editor.yankPop", &["alt+y"]),
        ("tui.editor.undo", &["ctrl+-", "\x1b[45;5u"]),
        ("tui.editor.jumpForward", &["ctrl+]"]),
        ("tui.editor.jumpBackward", &["ctrl+alt+]"]),
        ("tui.altScreen.pageUp", &["pageUp"]),
        ("tui.altScreen.pageDown", &["pageDown"]),
        ("tui.altScreen.halfPageUp", &[]),
        ("tui.altScreen.halfPageDown", &[]),
        ("tui.altScreen.lineUp", &[]),
        ("tui.altScreen.lineDown", &[]),
        (
            "tui.altScreen.previousPrompt",
            &["ctrl+shift+up", "ctrl+up"],
        ),
        (
            "tui.altScreen.nextPrompt",
            &["ctrl+shift+down", "ctrl+down"],
        ),
        ("tui.altScreen.search", &["ctrl+shift+f"]),
        ("tui.altScreen.searchNext", &["enter", "ctrl+g"]),
        (
            "tui.altScreen.searchPrevious",
            &["shift+enter", "ctrl+shift+g"],
        ),
        ("tui.altScreen.searchClose", &["escape"]),
        ("tui.altScreen.top", &["home"]),
        ("tui.altScreen.bottom", &["end"]),
    ]
}

pub fn key_to_bytes(key: &str) -> String {
    match key {
        "ctrl+a" => "\x01".into(),
        "ctrl+b" => "\x02".into(),
        "ctrl+c" => "\x03".into(),
        "ctrl+d" => "\x04".into(),
        "ctrl+e" => "\x05".into(),
        "ctrl+f" => "\x06".into(),
        "ctrl+g" => "\x07".into(),
        "ctrl+k" => "\x0b".into(),
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
        "ctrl+w" => "\x17".into(),
        "ctrl+x" => "\x18".into(),
        "ctrl+y" => "\x19".into(),
        "ctrl+z" => "\x1a".into(),
        "ctrl+-" => "\x1f".into(),
        "ctrl+]" => "\x1d".into(),
        "ctrl+alt+]" => "\x1b\x1d".into(),
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
        "alt+b" => "\x1bb".into(),
        "alt+f" => "\x1bf".into(),
        "alt+d" => "\x1bd".into(),
        "alt+y" => "\x1by".into(),
        "alt+backspace" => "\x1b\x7f".into(),
        "alt+delete" => "\x1b[3;3~".into(),
        "left" => "\x1b[D".into(),
        "right" => "\x1b[C".into(),
        "home" => "\x1b[H".into(),
        "end" => "\x1b[F".into(),
        "delete" => "\x1b[3~".into(),
        "backspace" => "\x7f".into(),
        "ctrl+home" => "\x1b[1;5H".into(),
        "ctrl+end" => "\x1b[1;5F".into(),
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
        "pageUp" => "\x1b[5~".into(),
        "pageDown" => "\x1b[6~".into(),
        "ctrl+pageUp" => "\x1b[5;5~".into(),
        "ctrl+pageDown" => "\x1b[6;5~".into(),
        "shift+enter" => "\n".into(),
        "ctrl+shift+f" => "\x1b[102;6u".into(),
        "ctrl+shift+g" => "\x1b[103;6u".into(),
        "ctrl+shift+up" => "\x1b[1;6A".into(),
        "ctrl+shift+down" => "\x1b[1;6B".into(),
        "ctrl+up" => "\x1b[1;5A".into(),
        "ctrl+down" => "\x1b[1;5B".into(),
        other => other.to_string(),
    }
}

/// Match a key id against legacy, Kitty CSI-u, and `key_to_bytes` encodings.
pub fn sequence_matches_key(data: &str, key: &str) -> bool {
    if key_to_bytes(key) == data {
        return true;
    }
    let parsed = crate::keys::parse_key(key);
    match parsed.name.as_str() {
        "home" if !parsed.ctrl && !parsed.alt && !parsed.shift => {
            matches!(data, "\x1b[H" | "\x1bOH" | "\x1b[1~" | "\x1b[7~")
                || kitty_matches(data, "home", 0)
        }
        "end" if !parsed.ctrl && !parsed.alt && !parsed.shift => {
            matches!(data, "\x1b[F" | "\x1bOF" | "\x1b[4~" | "\x1b[8~")
                || kitty_matches(data, "end", 0)
        }
        "pageup" if !parsed.ctrl && !parsed.alt && !parsed.shift => {
            matches!(data, "\x1b[5~" | "\x1b[[5~") || kitty_matches(data, "pageup", 0)
        }
        "pagedown" if !parsed.ctrl && !parsed.alt && !parsed.shift => {
            matches!(data, "\x1b[6~" | "\x1b[[6~") || kitty_matches(data, "pagedown", 0)
        }
        "up" => {
            let bits = crate::keys::key_modifier_bits(parsed.ctrl, parsed.alt, parsed.shift);
            kitty_matches(data, "up", bits)
                || (bits == 4 && data == "\x1b[1;5A")
                || (bits == 5 && data == "\x1b[1;6A")
        }
        "down" => {
            let bits = crate::keys::key_modifier_bits(parsed.ctrl, parsed.alt, parsed.shift);
            kitty_matches(data, "down", bits)
                || (bits == 4 && data == "\x1b[1;5B")
                || (bits == 5 && data == "\x1b[1;6B")
        }
        name if name.len() == 1 => {
            let bits = crate::keys::key_modifier_bits(parsed.ctrl, parsed.alt, parsed.shift);
            let Some(code) = name.chars().next().map(|ch| ch as u32) else {
                return false;
            };
            kitty_code_matches(data, code, bits)
        }
        _ => false,
    }
}

fn kitty_matches(data: &str, name: &str, bits: u32) -> bool {
    let Some(code) = crate::keys::kitty_functional_codepoint(name) else {
        return false;
    };
    kitty_code_matches(data, code, bits)
}

fn kitty_code_matches(data: &str, code: u32, bits: u32) -> bool {
    crate::keys::parse_kitty_csi_u(data)
        .is_some_and(|(found, found_bits, _release)| found == code && found_bits == bits)
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
        assert!(Keybindings::defaults().matches("\x1b[1;5D", "tui.editor.cursorWordLeft"));
        assert!(Keybindings::defaults().matches("\x17", "tui.editor.deleteWordBackward"));
        assert!(Keybindings::defaults().matches("\x7f", "tui.editor.deleteCharBackward"));
        assert!(Keybindings::defaults().matches("\x19", "tui.editor.yank"));
        assert!(Keybindings::defaults().matches("\x1by", "tui.editor.yankPop"));
        assert!(Keybindings::defaults().matches("\x15", "tui.editor.deleteToLineStart"));
        assert!(Keybindings::defaults().matches("\x0b", "tui.editor.deleteToLineEnd"));
        assert!(Keybindings::defaults().matches("\x1d", "tui.editor.jumpForward"));
        assert!(Keybindings::defaults().matches("\x1b\x1d", "tui.editor.jumpBackward"));
        assert!(Keybindings::defaults().matches("\x1f", "tui.editor.undo"));
        assert!(Keybindings::defaults().matches("\x1b[45;5u", "tui.editor.undo"));
        assert!(Keybindings::defaults().matches("\x1b[A", "tui.editor.cursorUp"));
        assert!(Keybindings::defaults().matches("\x1b[B", "tui.editor.cursorDown"));
        assert!(Keybindings::defaults()
            .keys_for("tui.editor.historyPrevious")
            .is_empty());
        assert!(Keybindings::defaults().matches("\x1b[5~", "tui.editor.pageUp"));
        assert!(Keybindings::defaults().matches("\x1b[6~", "tui.editor.pageDown"));
        let migrated = Keybindings::from_json(r#"{"clear":"ctrl+u"}"#);
        assert!(migrated.matches("\x15", "app.clear"));
    }
}
