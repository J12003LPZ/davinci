//! Expandable built-in startup header matching TS `builtInHeader`.

use crate::keybindings::Keybindings;
use crate::loaded_resources::ExpandableText;
use crate::themes::Theme;

pub fn format_key_text(key: &str, capitalize: bool) -> String {
    key.split('/')
        .map(|combo| {
            combo
                .split('+')
                .map(|part| format_key_part(part, capitalize))
                .collect::<Vec<_>>()
                .join("+")
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub fn key_text(bindings: &Keybindings, action: &str) -> String {
    format_key_text(&bindings.keys_for(action).join("/"), false)
}

pub fn key_hint(theme: &Theme, bindings: &Keybindings, action: &str, description: &str) -> String {
    format!(
        "{}{}",
        theme.fg("dim", &key_text(bindings, action)),
        theme.fg("muted", &format!(" {description}"))
    )
}

pub fn raw_key_hint(theme: &Theme, key: &str, description: &str) -> String {
    format!(
        "{}{}",
        theme.fg("dim", &format_key_text(key, false)),
        theme.fg("muted", &format!(" {description}"))
    )
}

pub fn build_startup_header(
    theme: &Theme,
    app_name: &str,
    version: &str,
    bindings: &Keybindings,
    expanded: bool,
) -> ExpandableText {
    let logo = format!(
        "{}{}",
        theme.bold(&theme.fg("accent", app_name)),
        theme.fg("dim", &format!(" v{version}"))
    );
    let compact_instructions = [
        key_hint(theme, bindings, "app.interrupt", "interrupt"),
        raw_key_hint(
            theme,
            &format!(
                "{}/{}",
                key_text(bindings, "app.clear"),
                key_text(bindings, "app.exit")
            ),
            "clear/exit",
        ),
        raw_key_hint(theme, "/", "commands"),
        raw_key_hint(theme, "!", "bash"),
        key_hint(theme, bindings, "app.tools.expand", "more"),
    ]
    .join(&theme.fg("muted", " · "));
    let compact_onboarding = theme.fg(
        "dim",
        &format!(
            "Press {} to show full startup help and loaded resources.",
            key_text(bindings, "app.tools.expand")
        ),
    );
    let onboarding = theme.fg(
        "dim",
        "Pi can explain its own features and look up its docs. Ask it how to use or extend Pi.",
    );
    let expanded_instructions = [
        key_hint(theme, bindings, "app.interrupt", "to interrupt"),
        key_hint(theme, bindings, "app.clear", "to clear"),
        raw_key_hint(
            theme,
            &format!("{} twice", key_text(bindings, "app.clear")),
            "to exit",
        ),
        key_hint(theme, bindings, "app.exit", "to exit (empty)"),
        key_hint(theme, bindings, "app.suspend", "to suspend"),
        key_hint(
            theme,
            bindings,
            "tui.editor.deleteToLineEnd",
            "to delete to end",
        ),
        key_hint(
            theme,
            bindings,
            "app.thinking.cycle",
            "to cycle thinking level",
        ),
        raw_key_hint(
            theme,
            &format!(
                "{}/{}",
                key_text(bindings, "app.model.cycleForward"),
                key_text(bindings, "app.model.cycleBackward")
            ),
            "to cycle models",
        ),
        key_hint(theme, bindings, "app.model.select", "to select model"),
        key_hint(theme, bindings, "app.tools.expand", "to expand tools"),
        key_hint(theme, bindings, "app.thinking.toggle", "to expand thinking"),
        key_hint(
            theme,
            bindings,
            "app.editor.external",
            "for external editor",
        ),
        raw_key_hint(theme, "/", "for commands"),
        raw_key_hint(theme, "!", "to run bash"),
        raw_key_hint(theme, "!!", "to run bash (no context)"),
        key_hint(
            theme,
            bindings,
            "app.message.followUp",
            "to queue follow-up",
        ),
        key_hint(
            theme,
            bindings,
            "app.message.dequeue",
            "to edit all queued messages",
        ),
        key_hint(
            theme,
            bindings,
            "app.clipboard.pasteImage",
            "to paste image (with text fallback)",
        ),
        raw_key_hint(theme, "drop files", "to attach"),
    ]
    .join("\n");
    ExpandableText {
        collapsed: format!("{logo}\n{compact_instructions}\n{compact_onboarding}\n\n{onboarding}"),
        expanded_text: format!("{logo}\n{expanded_instructions}\n\n{onboarding}"),
        expanded,
    }
}

fn format_key_part(part: &str, capitalize: bool) -> String {
    let display = if cfg!(target_os = "macos") && part.eq_ignore_ascii_case("alt") {
        "option"
    } else {
        part
    };
    if capitalize {
        let mut chars = display.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
            None => String::new(),
        }
    } else {
        display.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::builtin_themes;

    #[test]
    fn compact_header_mentions_expand_and_expanded_lists_hints() {
        let theme = builtin_themes()[0].clone();
        let bindings = Keybindings::defaults();
        let mut header = build_startup_header(&theme, "pi", "0.84.4", &bindings, false);
        let compact = header.current();
        assert!(compact.contains("pi"));
        assert!(compact.contains("v0.84.4"));
        assert!(compact.contains("ctrl+o"));
        assert!(compact.contains("full startup help and loaded resources"));
        assert!(!compact.contains("to cycle thinking level"));
        header.set_expanded(true);
        let expanded = header.current();
        assert!(expanded.contains("to cycle thinking level"));
        assert!(expanded.contains("to expand tools"));
        assert!(expanded.contains("drop files"));
    }
}
