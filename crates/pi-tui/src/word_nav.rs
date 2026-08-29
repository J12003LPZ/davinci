//! Word navigation matching TypeScript `packages/tui/src/word-navigation.ts`.

/// TypeScript `PUNCTUATION_REGEX` from `packages/tui/src/utils.ts`.
const ASCII_PUNCTUATION: &[char] = &[
    '(', ')', '{', '}', '[', ']', '<', '>', '.', ',', ';', ':', '\'', '"', '!', '?', '+', '-', '=',
    '*', '/', '\\', '|', '&', '%', '^', '$', '#', '@', '~', '`',
];

#[derive(Debug, Clone)]
struct Segment {
    start: usize,
    end: usize,
    is_word_like: bool,
}

impl Segment {
    fn text<'a>(&self, src: &'a str) -> &'a str {
        &src[self.start..self.end]
    }
}

fn is_ascii_punctuation(ch: char) -> bool {
    ASCII_PUNCTUATION.contains(&ch)
}

fn is_cjk_letter(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}'
    )
}

fn is_unicode_punctuation(ch: char) -> bool {
    matches!(ch, '\u{3000}'..='\u{303F}' | '\u{FF00}'..='\u{FFEF}') && !ch.is_alphanumeric()
        || matches!(
            ch,
            '。' | '，' | '、' | '；' | '：' | '！' | '？' | '「' | '」'
        )
}

fn segment_words(text: &str) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        if ch.is_whitespace() {
            let mut end = start + ch.len_utf8();
            while let Some((_, next)) = chars.peek() {
                if next.is_whitespace() {
                    end += next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            segments.push(Segment {
                start,
                end,
                is_word_like: false,
            });
            continue;
        }
        if is_ascii_punctuation(ch) {
            let mut end = start + ch.len_utf8();
            while let Some((_, next)) = chars.peek() {
                if is_ascii_punctuation(*next) {
                    end += next.len_utf8();
                    chars.next();
                } else {
                    break;
                }
            }
            segments.push(Segment {
                start,
                end,
                is_word_like: false,
            });
            continue;
        }
        if is_unicode_punctuation(ch) {
            segments.push(Segment {
                start,
                end: start + ch.len_utf8(),
                is_word_like: false,
            });
            continue;
        }
        if is_cjk_letter(ch) {
            let mut end = start + ch.len_utf8();
            if let Some((_, next)) = chars.peek() {
                if is_cjk_letter(*next) {
                    end += next.len_utf8();
                    chars.next();
                }
            }
            segments.push(Segment {
                start,
                end,
                is_word_like: true,
            });
            continue;
        }
        let mut end = start + ch.len_utf8();
        while let Some((_, next)) = chars.peek() {
            if next.is_whitespace()
                || is_ascii_punctuation(*next)
                || is_unicode_punctuation(*next)
                || is_cjk_letter(*next)
            {
                break;
            }
            end += next.len_utf8();
            chars.next();
        }
        segments.push(Segment {
            start,
            end,
            is_word_like: true,
        });
    }
    segments
}

fn is_whitespace_segment(text: &str) -> bool {
    !text.is_empty() && text.chars().all(char::is_whitespace)
}

fn last_punctuation_end(segment: &str) -> Option<usize> {
    segment
        .char_indices()
        .filter(|(_, ch)| is_ascii_punctuation(*ch))
        .last()
        .map(|(i, ch)| i + ch.len_utf8())
}

fn first_punctuation_index(segment: &str) -> Option<usize> {
    segment
        .char_indices()
        .find(|(_, ch)| is_ascii_punctuation(*ch))
        .map(|(i, _)| i)
}

/// TypeScript `findWordBackward`.
pub fn find_word_backward(text: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let mut segments = segment_words(before);
    let mut new_cursor = cursor;
    while let Some(last) = segments.last() {
        if is_whitespace_segment(last.text(before)) {
            new_cursor = last.start;
            segments.pop();
        } else {
            break;
        }
    }
    let Some(last) = segments.last() else {
        return new_cursor;
    };
    if last.is_word_like {
        let segment = last.text(before);
        if let Some(end) = last_punctuation_end(segment) {
            new_cursor -= segment.len() - end;
        } else {
            new_cursor = last.start;
        }
    } else {
        while let Some(last) = segments.last() {
            if last.is_word_like || is_whitespace_segment(last.text(before)) {
                break;
            }
            new_cursor = last.start;
            segments.pop();
        }
    }
    new_cursor
}

/// TypeScript `findWordForward`.
pub fn find_word_forward(text: &str, cursor: usize) -> usize {
    if cursor >= text.len() {
        return text.len();
    }
    let after = &text[cursor..];
    let segments = segment_words(after);
    let mut iter = segments.into_iter();
    let mut new_cursor = cursor;
    let mut next = iter.next();
    while let Some(seg) = next.as_ref() {
        if is_whitespace_segment(seg.text(after)) {
            new_cursor += seg.end - seg.start;
            next = iter.next();
        } else {
            break;
        }
    }
    let Some(seg) = next else {
        return new_cursor;
    };
    if seg.is_word_like {
        let segment = seg.text(after);
        new_cursor += first_punctuation_index(segment).unwrap_or(segment.len());
    } else {
        new_cursor += seg.end - seg.start;
        for more in iter {
            if more.is_word_like || is_whitespace_segment(more.text(after)) {
                break;
            }
            new_cursor += more.end - more.start;
        }
    }
    new_cursor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, chars: usize) -> usize {
        text.chars().take(chars).map(|c| c.len_utf8()).sum()
    }

    #[test]
    fn find_word_matches_typescript_fixtures() {
        let text = "hello world";
        assert_eq!(find_word_backward(text, 11), 6);
        assert_eq!(find_word_backward(text, 6), 0);
        assert_eq!(find_word_forward(text, 0), 5);
        assert_eq!(find_word_forward(text, 5), 11);

        let text = "foo.bar";
        assert_eq!(find_word_backward(text, 7), 4);
        assert_eq!(find_word_backward(text, 4), 3);
        assert_eq!(find_word_backward(text, 3), 0);
        assert_eq!(find_word_forward(text, 0), 3);
        assert_eq!(find_word_forward(text, 3), 4);
        assert_eq!(find_word_forward(text, 4), 7);

        let text = "foo:bar";
        assert_eq!(find_word_backward(text, 7), 4);
        assert_eq!(find_word_backward(text, 4), 3);
        assert_eq!(find_word_backward(text, 3), 0);

        let text = "path/to/file";
        assert_eq!(find_word_backward(text, 12), 8);
        assert_eq!(find_word_backward(text, 8), 7);
        assert_eq!(find_word_backward(text, 7), 5);
        assert_eq!(find_word_backward(text, 5), 4);
        assert_eq!(find_word_backward(text, 4), 0);
        assert_eq!(find_word_forward(text, 0), 4);
        assert_eq!(find_word_forward(text, 4), 5);
        assert_eq!(find_word_forward(text, 5), 7);
        assert_eq!(find_word_forward(text, 7), 8);
        assert_eq!(find_word_forward(text, 8), 12);

        let text = "foo...bar";
        assert_eq!(find_word_backward(text, 9), 6);
        assert_eq!(find_word_backward(text, 6), 3);
        assert_eq!(find_word_backward(text, 3), 0);
        assert_eq!(find_word_forward(text, 0), 3);
        assert_eq!(find_word_forward(text, 3), 6);
        assert_eq!(find_word_forward(text, 6), 9);

        let text = "  hello  ";
        assert_eq!(find_word_backward(text, 9), 2);
        assert_eq!(find_word_backward(text, 2), 0);
        assert_eq!(find_word_forward(text, 0), 7);
        assert_eq!(find_word_forward(text, 7), 9);

        let text = "你好世界 test";
        assert_eq!(
            find_word_backward(text, at(text, text.chars().count())),
            at(text, 5)
        );
        assert_eq!(find_word_backward(text, at(text, 5)), at(text, 2));
        assert_eq!(find_word_backward(text, at(text, 2)), 0);
    }
}
