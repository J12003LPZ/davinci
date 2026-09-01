//! Shared input ownership and interaction routing primitives.
//!
//! Renderers may look different, but they should agree on who owns a key. The
//! runtime handles emergency controls first; then the most specific active
//! interaction layer wins. Global application shortcuts are a fallback chosen
//! by the caller after the active owner declines them.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::{key_to_bytes, Editor, Keybindings};

/// The active UI layer that gets first refusal on ordinary input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputOwner {
    Modal,
    Autocomplete,
    Surface,
    Composer,
}

/// Resolve focus from most-specific to least-specific.
pub(crate) fn input_owner(modal: bool, autocomplete: bool, surface: bool) -> InputOwner {
    if modal {
        InputOwner::Modal
    } else if autocomplete {
        InputOwner::Autocomplete
    } else if surface {
        InputOwner::Surface
    } else {
        InputOwner::Composer
    }
}

/// Convert a crossterm key event into the same canonical byte spelling used by
/// [`Keybindings`]. This lets alternate renderers reuse the regular TUI's
/// configurable actions instead of matching `KeyCode`s independently.
pub fn key_event_bytes(key: &KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);

    match key.code {
        KeyCode::Char(ch) if ctrl || alt => {
            let codepoint = ch.to_ascii_lowercase() as u32;
            let bits = crate::keys::key_modifier_bits(ctrl, alt, shift);
            Some(format!("\x1b[{codepoint};{}u", bits + 1))
        }
        KeyCode::Char(ch) => Some(ch.to_string()),
        KeyCode::Enter if alt => Some("\x1b\r".into()),
        KeyCode::Enter if shift => Some("\n".into()),
        KeyCode::Enter => Some("\r".into()),
        KeyCode::Tab if shift => Some("\x1b[Z".into()),
        KeyCode::Tab => Some("\t".into()),
        KeyCode::BackTab => Some("\x1b[Z".into()),
        KeyCode::Esc => Some("\x1b".into()),
        KeyCode::Backspace if ctrl => Some(key_to_bytes("ctrl+backspace")),
        KeyCode::Backspace if alt => Some(key_to_bytes("alt+backspace")),
        KeyCode::Backspace => Some(key_to_bytes("backspace")),
        KeyCode::Delete if alt => Some(key_to_bytes("alt+delete")),
        KeyCode::Delete => Some(key_to_bytes("delete")),
        KeyCode::Left => Some(direction_bytes("left", key.modifiers)),
        KeyCode::Right => Some(direction_bytes("right", key.modifiers)),
        KeyCode::Up => Some(direction_bytes("up", key.modifiers)),
        KeyCode::Down => Some(direction_bytes("down", key.modifiers)),
        KeyCode::Home if ctrl => Some(key_to_bytes("ctrl+home")),
        KeyCode::Home => Some(key_to_bytes("home")),
        KeyCode::End if ctrl => Some(key_to_bytes("ctrl+end")),
        KeyCode::End => Some(key_to_bytes("end")),
        KeyCode::PageUp if ctrl => Some(key_to_bytes("ctrl+pageUp")),
        KeyCode::PageUp => Some(key_to_bytes("pageUp")),
        KeyCode::PageDown if ctrl => Some(key_to_bytes("ctrl+pageDown")),
        KeyCode::PageDown => Some(key_to_bytes("pageDown")),
        _ => None,
    }
}

fn direction_bytes(name: &str, modifiers: KeyModifiers) -> String {
    if modifiers.contains(KeyModifiers::CONTROL) {
        key_to_bytes(&format!("ctrl+{name}"))
    } else if modifiers.contains(KeyModifiers::ALT) {
        key_to_bytes(&format!("alt+{name}"))
    } else {
        key_to_bytes(name)
    }
}

/// Apply one configured editor action to the shared editor implementation.
///
/// This is intentionally renderer-agnostic: both the regular TUI and alternate
/// renderers can use the same cursor, history, kill-ring, undo and jump
/// semantics while drawing the editor differently.
pub(crate) fn apply_editor_key(editor: &mut Editor, keybindings: &Keybindings, data: &str) -> bool {
    if editor.jump_mode().is_some()
        && (keybindings.matches(data, "tui.editor.jumpForward")
            || keybindings.matches(data, "tui.editor.jumpBackward"))
    {
        editor.cancel_jump();
        return true;
    }
    if keybindings.matches(data, "tui.editor.historyPrevious") {
        editor.navigate_history(-1);
        return true;
    }
    if keybindings.matches(data, "tui.editor.historyNext") {
        editor.navigate_history(1);
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorUp") {
        editor.cursor_up();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorDown") {
        editor.cursor_down();
        return true;
    }
    if keybindings.matches(data, "tui.editor.pageUp") {
        editor.page_up();
        return true;
    }
    if keybindings.matches(data, "tui.editor.pageDown") {
        editor.page_down();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorWordLeft") {
        editor.move_word_backwards();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorWordRight") {
        editor.move_word_forwards();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorLeft") {
        editor.move_left();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorRight") {
        editor.move_right();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorLineStart") {
        editor.move_line_start();
        return true;
    }
    if keybindings.matches(data, "tui.editor.cursorLineEnd") {
        editor.move_line_end();
        return true;
    }
    if keybindings.matches(data, "tui.editor.deleteWordBackward") {
        editor.delete_word_backwards();
        return true;
    }
    if keybindings.matches(data, "tui.editor.deleteWordForward") {
        editor.delete_word_forwards();
        return true;
    }
    if keybindings.matches(data, "tui.editor.deleteCharForward") && !editor.buffer.is_empty() {
        editor.delete_forward();
        return true;
    }
    if keybindings.matches(data, "tui.editor.deleteCharBackward") {
        editor.backspace();
        return true;
    }
    if keybindings.matches(data, "tui.editor.deleteToLineStart") {
        editor.delete_to_line_start();
        return true;
    }
    if keybindings.matches(data, "tui.editor.deleteToLineEnd") {
        editor.delete_to_line_end();
        return true;
    }
    if keybindings.matches(data, "tui.editor.yank") {
        editor.yank();
        return true;
    }
    if keybindings.matches(data, "tui.editor.yankPop") {
        editor.yank_pop();
        return true;
    }
    if keybindings.matches(data, "tui.editor.undo") {
        editor.undo();
        return true;
    }
    if keybindings.matches(data, "tui.editor.jumpForward") {
        editor.begin_jump_forward();
        return true;
    }
    if keybindings.matches(data, "tui.editor.jumpBackward") {
        editor.begin_jump_backward();
        return true;
    }
    false
}
