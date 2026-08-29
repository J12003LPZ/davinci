use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyId {
    Char(char),
    Enter,
    Backspace,
    Tab,
    Esc,
    Up,
    Down,
    Left,
    Right,
    Ctrl(char),
    Alt(char),
}

pub struct KeybindingsManager {
    bindings: HashMap<KeyId, String>,
}

impl KeybindingsManager {
    pub fn new() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert(KeyId::Ctrl('c'), "quit".to_string());
        bindings.insert(KeyId::Ctrl('p'), "cycle_model".to_string());
        bindings.insert(KeyId::Ctrl('t'), "cycle_thinking".to_string());
        Self { bindings }
    }

    pub fn get_action(&self, key: &KeyId) -> Option<&String> {
        self.bindings.get(key)
    }
}

impl Default for KeybindingsManager {
    fn default() -> Self {
        Self::new()
    }
}
