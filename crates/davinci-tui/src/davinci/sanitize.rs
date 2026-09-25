//! Sanitize untrusted text before it reaches the terminal.
//!
//! Model, tool, extension, and session text is display data, never terminal
//! control data. Preserve newlines and tabs, strip escape sequences and other
//! control characters, and make carriage-return progress output match what a
//! terminal would leave visible.

use std::borrow::Cow;

pub fn terminal_safe(text: &str) -> Cow<'_, str> {
    if !text
        .chars()
        .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
    {
        return Cow::Borrowed(text);
    }

    let stripped = crate::ansi::strip_terminal_sequences(text);
    let mut out = String::with_capacity(stripped.len());
    for (index, line) in stripped.split('\n').enumerate() {
        if index > 0 {
            out.push('\n');
        }
        let line = line.strip_suffix('\r').unwrap_or(line);
        let visible = line.rsplit('\r').next().unwrap_or(line);
        out.extend(visible.chars().filter(|ch| !ch.is_control() || *ch == '\t'));
    }
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_and_controls_are_removed() {
        assert_eq!(terminal_safe("a\x1b]52;c;SGVsbG8=\x07b"), "ab");
        assert_eq!(terminal_safe("x\x1b[2Jy"), "xy");
        assert_eq!(terminal_safe("bell\x07"), "bell");
        assert_eq!(terminal_safe("tab\tkept"), "tab\tkept");
    }

    #[test]
    fn carriage_return_keeps_what_the_terminal_would_show() {
        assert_eq!(terminal_safe("10%\r50%\r100%"), "100%");
        assert_eq!(terminal_safe("line\r\n"), "line\n");
    }

    #[test]
    fn plain_text_is_not_copied() {
        assert!(matches!(terminal_safe("plain text"), Cow::Borrowed(_)));
    }
}
