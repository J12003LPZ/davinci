//! Untrusted recognition text to plain editor insertion (plan section 4.5).

pub const MAX_TEXT_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TextError {
    #[error("Voice text exceeds the 32 KiB limit")]
    TooLong,
    #[error("Invalid editor cursor")]
    InvalidCursor,
}

pub fn normalize(raw: &str) -> Result<String, TextError> {
    if raw.len() > MAX_TEXT_BYTES {
        return Err(TextError::TooLong);
    }
    let lines = raw.replace("\r\n", "\n").replace('\r', "\n");
    let text: String = lines
        .chars()
        .filter_map(|ch| match ch {
            '\t' => Some(' '),
            '\n' => Some('\n'),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect();
    Ok(text.trim().to_owned())
}

pub fn insertion(raw: &str, draft: &str, cursor: usize) -> Result<String, TextError> {
    if !draft.is_char_boundary(cursor) {
        return Err(TextError::InvalidCursor);
    }
    let text = normalize(raw)?;
    if text.is_empty() {
        return Ok(text);
    }
    let word = |ch: Option<char>| ch.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
    let left = word(draft[..cursor].chars().next_back()) && word(text.chars().next());
    let right = word(text.chars().next_back()) && word(draft[cursor..].chars().next());
    Ok(format!(
        "{}{}{}",
        if left { " " } else { "" },
        text,
        if right { " " } else { "" }
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_and_line_endings_are_plain_text() {
        assert_eq!(
            normalize(" \u{1b}hello\tworld\r\nnext\rline\u{85} ").unwrap(),
            "hello world\nnext\nline"
        );
        assert_eq!(
            normalize("👩\u{200d}💻 e\u{301} 中文").unwrap(),
            "👩\u{200d}💻 e\u{301} 中文"
        );
    }

    #[test]
    fn spacing_is_independent_at_each_live_boundary() {
        assert_eq!(insertion("word", "abcXYZ", 3).unwrap(), " word ");
        assert_eq!(insertion("中文", "甲乙", 3).unwrap(), "中文");
        assert_eq!(insertion("word", "(,", 1).unwrap(), "word");
        assert_eq!(insertion("  ", "ab", 1).unwrap(), "");
        assert!(insertion("word", "甲", 1).is_err());
    }

    #[test]
    fn over_limit_is_an_error_without_truncation() {
        assert!(normalize(&"x".repeat(32 * 1024 + 1)).is_err());
        assert!(normalize(&"x".repeat(32 * 1024)).is_ok());
    }
}
