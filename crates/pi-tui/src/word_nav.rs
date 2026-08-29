//! Word cursor movement matching TS `word-navigation.ts`.

/// ASCII punctuation used as intra-word boundaries (`PUNCTUATION_REGEX`).
const PUNCTUATION: &[char] = &[
    '(', ')', '{', '}', '[', ']', '<', '>', '.', ',', ';', ':', '\'', '"', '!', '?', '+', '-', '=',
    '*', '/', '\\', '|', '&', '%', '^', '$', '#', '@', '~', '`',
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordSegment {
    pub text: String,
    pub is_word_like: bool,
    pub is_atomic: bool,
}

pub fn is_whitespace_char(text: &str) -> bool {
    !text.is_empty() && text.chars().all(char::is_whitespace)
}

pub fn default_word_segments(text: &str) -> Vec<WordSegment> {
    let mut segments = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch.is_whitespace() {
            let mut text = ch.to_string();
            while chars.peek().is_some_and(|next| next.is_whitespace()) {
                text.push(chars.next().expect("peeked"));
            }
            segments.push(WordSegment {
                text,
                is_word_like: false,
                is_atomic: false,
            });
            continue;
        }
        if is_cjk(ch) {
            segments.push(WordSegment {
                text: ch.to_string(),
                is_word_like: true,
                is_atomic: false,
            });
            continue;
        }
        if is_word_char(ch) {
            let mut text = ch.to_string();
            while chars.peek().is_some_and(|next| is_word_char(*next)) {
                text.push(chars.next().expect("peeked"));
            }
            segments.push(WordSegment {
                text,
                is_word_like: true,
                is_atomic: false,
            });
            continue;
        }
        let mut text = ch.to_string();
        while chars
            .peek()
            .is_some_and(|next| !next.is_whitespace() && !is_word_char(*next) && !is_cjk(*next))
        {
            text.push(chars.next().expect("peeked"));
        }
        segments.push(WordSegment {
            text,
            is_word_like: false,
            is_atomic: false,
        });
    }
    segments
}

fn is_word_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3040..=0x30FF | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F
    )
}

fn last_punctuation_end(segment: &str) -> Option<usize> {
    let mut last = None;
    for (index, ch) in segment.char_indices() {
        if PUNCTUATION.contains(&ch) {
            last = Some(index + ch.len_utf8());
        }
    }
    last
}

fn first_punctuation_start(segment: &str) -> Option<usize> {
    segment
        .char_indices()
        .find(|(_, ch)| PUNCTUATION.contains(ch))
        .map(|(index, _)| index)
}

pub fn find_word_backward(text: &str, cursor: usize, segments: &[WordSegment]) -> usize {
    if cursor == 0 {
        return 0;
    }
    let cursor = cursor.min(text.len());
    let mut segments = segments.to_vec();
    let mut new_cursor = cursor;
    while segments
        .last()
        .is_some_and(|segment| !segment.is_atomic && is_whitespace_char(&segment.text))
    {
        new_cursor -= segments
            .pop()
            .map(|segment| segment.text.len())
            .unwrap_or(0);
    }
    let Some(last) = segments.last() else {
        return new_cursor;
    };
    if last.is_atomic {
        new_cursor -= last.text.len();
    } else if last.is_word_like {
        if let Some(end) = last_punctuation_end(&last.text) {
            new_cursor -= last.text.len() - end;
        } else {
            new_cursor -= last.text.len();
        }
    } else {
        while segments
            .last()
            .is_some_and(|segment| !segment.is_word_like && !is_whitespace_char(&segment.text))
        {
            new_cursor -= segments
                .pop()
                .map(|segment| segment.text.len())
                .unwrap_or(0);
        }
    }
    new_cursor
}

pub fn find_word_forward(text: &str, cursor: usize, segments: &[WordSegment]) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    let mut segments = segments.iter().cloned().peekable();
    let mut new_cursor = cursor;
    while segments
        .peek()
        .is_some_and(|segment| !segment.is_atomic && is_whitespace_char(&segment.text))
    {
        new_cursor += segments
            .next()
            .map(|segment| segment.text.len())
            .unwrap_or(0);
    }
    let Some(next) = segments.peek().cloned() else {
        return new_cursor;
    };
    if next.is_atomic {
        new_cursor += next.text.len();
    } else if next.is_word_like {
        new_cursor += first_punctuation_start(&next.text).unwrap_or(next.text.len());
    } else {
        while segments
            .peek()
            .is_some_and(|segment| !segment.is_word_like && !is_whitespace_char(&segment.text))
        {
            new_cursor += segments
                .next()
                .map(|segment| segment.text.len())
                .unwrap_or(0);
        }
    }
    new_cursor
}

pub fn find_word_backward_default(text: &str, cursor: usize) -> usize {
    let before = &text[..cursor.min(text.len())];
    find_word_backward(text, cursor, &default_word_segments(before))
}

pub fn find_word_forward_default(text: &str, cursor: usize) -> usize {
    let after = &text[cursor.min(text.len())..];
    cursor + find_word_forward(after, 0, &default_word_segments(after))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backward_matches_ts_ascii_fixtures() {
        assert_eq!(find_word_backward_default("hello world", 11), 6);
        assert_eq!(find_word_backward_default("hello world", 6), 0);
        assert_eq!(find_word_backward_default("foo.bar", 7), 4);
        assert_eq!(find_word_backward_default("foo.bar", 4), 3);
        assert_eq!(find_word_backward_default("foo.bar", 3), 0);
        assert_eq!(find_word_backward_default("foo:bar", 7), 4);
        assert_eq!(find_word_backward_default("path/to/file", 12), 8);
        assert_eq!(find_word_backward_default("path/to/file", 8), 7);
        assert_eq!(find_word_backward_default("path/to/file", 7), 5);
        assert_eq!(find_word_backward_default("path/to/file", 5), 4);
        assert_eq!(find_word_backward_default("path/to/file", 4), 0);
        assert_eq!(find_word_backward_default("  hello  ", 9), 2);
        assert_eq!(find_word_backward_default("  hello  ", 2), 0);
        assert_eq!(find_word_backward_default("foo...bar", 9), 6);
        assert_eq!(find_word_backward_default("foo...bar", 6), 3);
        assert_eq!(find_word_backward_default("foo...bar", 3), 0);
        assert_eq!(find_word_backward_default("hello", 0), 0);
    }

    #[test]
    fn forward_matches_ts_ascii_fixtures() {
        assert_eq!(find_word_forward_default("hello world", 0), 5);
        assert_eq!(find_word_forward_default("hello world", 5), 11);
        assert_eq!(find_word_forward_default("foo.bar", 0), 3);
        assert_eq!(find_word_forward_default("foo.bar", 3), 4);
        assert_eq!(find_word_forward_default("foo.bar", 4), 7);
        assert_eq!(find_word_forward_default("foo:bar", 0), 3);
        assert_eq!(find_word_forward_default("path/to/file", 0), 4);
        assert_eq!(find_word_forward_default("path/to/file", 4), 5);
        assert_eq!(find_word_forward_default("path/to/file", 5), 7);
        assert_eq!(find_word_forward_default("path/to/file", 7), 8);
        assert_eq!(find_word_forward_default("path/to/file", 8), 12);
        assert_eq!(find_word_forward_default("  hello  ", 0), 7);
        assert_eq!(find_word_forward_default("  hello  ", 7), 9);
        assert_eq!(find_word_forward_default("foo...bar", 0), 3);
        assert_eq!(find_word_forward_default("foo...bar", 3), 6);
        assert_eq!(find_word_forward_default("foo...bar", 6), 9);
        assert_eq!(find_word_forward_default("hello", 5), 5);
    }

    #[test]
    fn cjk_walks_to_boundaries() {
        let text = "你好世界 test";
        let mut pos = text.len();
        while pos > 0 {
            let next = find_word_backward_default(text, pos);
            assert!(next < pos);
            pos = next;
        }
        pos = 0;
        while pos < text.len() {
            let next = find_word_forward_default(text, pos);
            assert!(next > pos);
            pos = next;
        }
        assert_eq!(pos, text.len());
    }

    #[test]
    fn atomic_segments_skip_as_one_unit() {
        let marker = "[paste #1 +5 lines]";
        let text = format!("hello {marker} world");
        let full = vec![
            WordSegment {
                text: "hello".into(),
                is_word_like: true,
                is_atomic: false,
            },
            WordSegment {
                text: " ".into(),
                is_word_like: false,
                is_atomic: false,
            },
            WordSegment {
                text: marker.into(),
                is_word_like: true,
                is_atomic: true,
            },
            WordSegment {
                text: " ".into(),
                is_word_like: false,
                is_atomic: false,
            },
            WordSegment {
                text: "world".into(),
                is_word_like: true,
                is_atomic: false,
            },
        ];
        assert_eq!(find_word_backward(&text, text.len(), &full), 26);
        let before_marker = &full[..4];
        assert_eq!(find_word_backward(&text, 26, before_marker), 6);
        let after = vec![
            WordSegment {
                text: marker.into(),
                is_word_like: true,
                is_atomic: true,
            },
            WordSegment {
                text: " ".into(),
                is_word_like: false,
                is_atomic: false,
            },
            WordSegment {
                text: "world".into(),
                is_word_like: true,
                is_atomic: false,
            },
        ];
        assert_eq!(find_word_forward(&text[6..], 0, &after), marker.len());
    }
}
