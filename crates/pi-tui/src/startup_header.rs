//! Startup header: the davinci identity mark (design spec §10) plus the
//! expandable keybind help the TS `builtInHeader` provided.

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

/// Extra facts shown under the wordmark at startup.
#[derive(Debug, Clone, Default)]
pub struct StartupInfo {
    pub cwd: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub session_restored: bool,
}

/// The identity mark: a line-drawn portrait after the Mona Lisa, built from
/// the same box-drawing set as the UI, the smile as the only copper stroke.
fn emblem_lines(theme: &Theme) -> Vec<String> {
    let frame = |text: &str| theme.fg("muted", text);
    let face = |text: &str| theme.fg("text", text);
    let copper = |text: &str| theme.fg("primary", text);
    let base = |text: &str| theme.fg("border", text);
    vec![
        frame("       ·─────────·"),
        frame("     ╱             ╲"),
        format!("{}{}{}", frame("    ╱  "), face("╭───────╮"), frame("  ╲")),
        format!("{}{}{}", frame("   │  "), face("╱ ·     · ╲"), frame("  │")),
        format!("{}{}{}", frame("   │  "), face("│    ╷    │"), frame("  │")),
        format!(
            "{}{}{}{}{}",
            frame("   │  "),
            face("╲  "),
            copper("╰───╯"),
            face("  ╱"),
            frame("  │")
        ),
        format!("{}{}{}", frame("    ╲  "), face("╰───────╯"), frame("  ╱")),
        frame("     ╲             ╱"),
        format!(
            "{}{}{}",
            base("   ·────────"),
            copper("┬"),
            base("────────·")
        ),
    ]
}

pub fn build_startup_header(
    theme: &Theme,
    app_name: &str,
    version: &str,
    bindings: &Keybindings,
    expanded: bool,
) -> ExpandableText {
    build_startup_header_with(
        theme,
        app_name,
        version,
        bindings,
        expanded,
        &StartupInfo::default(),
    )
}

pub fn build_startup_header_with(
    theme: &Theme,
    app_name: &str,
    version: &str,
    bindings: &Keybindings,
    expanded: bool,
    info: &StartupInfo,
) -> ExpandableText {
    let mut identity = emblem_lines(theme);
    identity.push(String::new());
    let wordmark: String = app_name
        .to_uppercase()
        .chars()
        .flat_map(|ch| [ch, ' '])
        .collect();
    identity.push(format!(
        "   {}{}",
        theme.bold(&theme.fg("text", wordmark.trim_end())),
        theme.fg("dim", &format!("  v{version}"))
    ));
    identity.push(format!(
        "   {}",
        theme.fg("primary", "macchina dell'intelletto")
    ));
    let mut facts = Vec::new();
    if let Some(cwd) = info.cwd.as_deref().filter(|value| !value.is_empty()) {
        facts.push(theme.fg("muted", cwd));
    }
    let mut second = String::new();
    if let Some(branch) = info.branch.as_deref().filter(|value| !value.is_empty()) {
        second.push_str(&theme.fg("secondary", branch));
    }
    if let Some(model) = info.model.as_deref().filter(|value| !value.is_empty()) {
        if !second.is_empty() {
            second.push_str(&theme.fg("border", " · "));
        }
        second.push_str(&theme.fg("muted", model));
    }
    if !second.is_empty() {
        facts.push(second);
    }
    if info.session_restored {
        facts.push(format!(
            "{} {}",
            theme.fg("success", "✓"),
            theme.fg("muted", "session restored · memoria intacta")
        ));
    }
    if !facts.is_empty() {
        identity.push(String::new());
        for fact in facts {
            identity.push(format!("   {fact}"));
        }
    }
    let logo = identity.join("\n");

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
    .join(&theme.fg("border", " · "));
    let compact_onboarding = theme.fg(
        "dim",
        &format!(
            "Press {} to show full startup help and loaded resources.",
            key_text(bindings, "app.tools.expand")
        ),
    );
    let onboarding = theme.fg(
        "dim",
        "A machine for thought, built in Rust. Ask it how to use or extend itself.",
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
        collapsed: format!("{logo}\n\n{compact_instructions}\n{compact_onboarding}"),
        expanded_text: format!("{logo}\n\n{expanded_instructions}\n\n{onboarding}"),
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
        assert!(compact.contains("P I"));
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

    #[test]
    fn identity_mark_and_facts_render() {
        let _guard = crate::themes::NO_COLOR_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        std::env::set_var("NO_COLOR", "1");
        let theme = builtin_themes()[0].clone();
        let bindings = Keybindings::defaults();
        let header = build_startup_header_with(
            &theme,
            "davinci",
            "0.84.4",
            &bindings,
            false,
            &StartupInfo {
                cwd: Some("C:\\dev\\davinci-rust".into()),
                branch: Some("main".into()),
                model: Some("openai-codex/gpt-5.6-sol".into()),
                session_restored: true,
            },
        );
        let text = header.current();
        std::env::remove_var("NO_COLOR");
        assert!(text.contains("D A V I N C I"), "{text}");
        assert!(text.contains("macchina dell'intelletto"));
        assert!(text.contains("╰───╯"), "smile stroke present");
        assert!(text.contains("main · openai-codex/gpt-5.6-sol"));
        assert!(text.contains("session restored"));
    }
}
