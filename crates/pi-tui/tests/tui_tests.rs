use pi_tui::component::{Component, Container, Text};
use pi_tui::keybindings::{KeyId, KeybindingsManager};

#[test]
fn test_tui_components() {
    let mut container = Container::new();
    container.add(Box::new(Text::new("Line 1\nLine 2")));
    container.add(Box::new(Text::new("Line 3")));

    let lines = container.render(80);
    assert_eq!(lines, vec!["Line 1", "Line 2", "Line 3"]);
}

#[test]
fn test_keybindings() {
    let mgr = KeybindingsManager::new();
    assert_eq!(mgr.get_action(&KeyId::Ctrl('c')), Some(&"quit".to_string()));
    assert_eq!(
        mgr.get_action(&KeyId::Ctrl('p')),
        Some(&"cycle_model".to_string())
    );
}
