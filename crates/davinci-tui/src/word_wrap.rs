//! TS `wordWrapLine` from `packages/tui/src/components/editor.ts`.

use unicode_segmentation::UnicodeSegmentation;

use crate::render::visible_width;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChunk {
    pub text: String,
    pub start_index: usize,
    pub end_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub index: usize,
}

pub fn grapheme_segments(line: &str) -> Vec<Segment> {
    let mut segs = Vec::new();
    let mut index = 0;
    for grapheme in line.graphemes(true) {
        segs.push(Segment {
            text: grapheme.to_string(),
            index,
        });
        index += grapheme.len();
    }
    segs
}

pub fn is_paste_marker(segment: &str) -> bool {
    parse_paste_marker(segment).is_some()
}

pub fn parse_paste_marker(marker: &str) -> Option<usize> {
    let inner = marker.strip_prefix("[paste #")?.strip_suffix(']')?;
    if let Some((id, rest)) = inner.split_once(' ') {
        let id = id.parse().ok()?;
        if let Some(count) = rest
            .strip_prefix('+')
            .and_then(|rest| rest.strip_suffix(" lines"))
        {
            count.parse::<usize>().ok()?;
            return Some(id);
        }
        if let Some(count) = rest.strip_suffix(" chars") {
            count.parse::<usize>().ok()?;
            return Some(id);
        }
        return None;
    }
    inner.parse().ok()
}

pub fn word_wrap_line(
    line: &str,
    max_width: usize,
    pre_segmented: Option<&[Segment]>,
) -> Vec<TextChunk> {
    if line.is_empty() || max_width == 0 {
        return vec![TextChunk {
            text: String::new(),
            start_index: 0,
            end_index: 0,
        }];
    }
    if visible_width(line) <= max_width {
        return vec![TextChunk {
            text: line.to_string(),
            start_index: 0,
            end_index: line.len(),
        }];
    }

    let owned = pre_segmented.map(|s| s.to_vec());
    let default_segs = if owned.is_none() {
        Some(grapheme_segments(line))
    } else {
        None
    };
    let segments: &[Segment] = owned
        .as_deref()
        .unwrap_or_else(|| default_segs.as_deref().unwrap());

    let mut chunks = Vec::new();
    let mut current_width = 0usize;
    let mut chunk_start = 0usize;
    let mut wrap_opp_index: isize = -1;
    let mut wrap_opp_width = 0usize;

    let mut i = 0;
    while i < segments.len() {
        let seg = &segments[i];
        let grapheme = seg.text.as_str();
        let g_width = visible_width(grapheme);
        let char_index = seg.index;
        let is_ws = !is_paste_marker(grapheme) && is_whitespace_char(grapheme);

        if current_width + g_width > max_width {
            if wrap_opp_index >= 0 && current_width - wrap_opp_width + g_width <= max_width {
                let wrap_at = wrap_opp_index as usize;
                chunks.push(TextChunk {
                    text: line[chunk_start..wrap_at].to_string(),
                    start_index: chunk_start,
                    end_index: wrap_at,
                });
                chunk_start = wrap_at;
                current_width -= wrap_opp_width;
            } else if chunk_start < char_index {
                chunks.push(TextChunk {
                    text: line[chunk_start..char_index].to_string(),
                    start_index: chunk_start,
                    end_index: char_index,
                });
                chunk_start = char_index;
                current_width = 0;
            }
            wrap_opp_index = -1;
        }

        if g_width > max_width {
            let sub = word_wrap_line(grapheme, max_width, None);
            for sc in sub.iter().take(sub.len().saturating_sub(1)) {
                chunks.push(TextChunk {
                    text: sc.text.clone(),
                    start_index: char_index + sc.start_index,
                    end_index: char_index + sc.end_index,
                });
            }
            if let Some(last) = sub.last() {
                chunk_start = char_index + last.start_index;
                current_width = visible_width(&last.text);
            }
            wrap_opp_index = -1;
            i += 1;
            continue;
        }

        current_width += g_width;

        let next = segments.get(i + 1);
        if is_ws && next.is_some_and(|n| is_paste_marker(&n.text) || !is_whitespace_char(&n.text)) {
            wrap_opp_index = next.unwrap().index as isize;
            wrap_opp_width = current_width;
        } else if !is_ws && next.is_some_and(|n| !is_whitespace_char(&n.text)) {
            let next = next.unwrap();
            let is_cjk = !is_paste_marker(grapheme) && is_cjk_break(grapheme);
            let next_is_cjk = !is_paste_marker(&next.text) && is_cjk_break(&next.text);
            if is_cjk || next_is_cjk {
                wrap_opp_index = next.index as isize;
                wrap_opp_width = current_width;
            }
        }
        i += 1;
    }

    chunks.push(TextChunk {
        text: line[chunk_start..].to_string(),
        start_index: chunk_start,
        end_index: line.len(),
    });
    chunks
}

fn is_whitespace_char(text: &str) -> bool {
    !text.is_empty() && text.chars().all(char::is_whitespace)
}

fn is_cjk_break(text: &str) -> bool {
    text.chars().next().is_some_and(|ch| {
        matches!(
            ch as u32,
            0x3040..=0x30FF
                | 0x3100..=0x312F
                | 0x3400..=0x4DBF
                | 0x4E00..=0x9FFF
                | 0xAC00..=0xD7AF
                | 0xF900..=0xFAFF
                | 0x20000..=0x2FA1F
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_at_word_boundaries_and_breaks_long_words() {
        let chunks = word_wrap_line(
            "Hello world this is a test of word wrapping functionality",
            20,
            None,
        );
        assert!(chunks.iter().all(|c| visible_width(&c.text) <= 20));
        assert!(!chunks[0].text.ends_with('-'));
        assert!(!chunks.iter().skip(1).any(|c| c.text.starts_with(' ')));

        let url = word_wrap_line(
            "Check https://example.com/very/long/path/that/exceeds/width here",
            29,
            None,
        );
        assert!(url.iter().all(|c| visible_width(&c.text) <= 29));
        assert!(url.iter().any(|c| c.text.contains("https://")));
    }

    #[test]
    fn wraps_cjk_and_emoji_without_overflow() {
        let cjk = word_wrap_line("日本語テスト", 10, None);
        assert_eq!(cjk[0].text, "日本語テス");
        assert_eq!(cjk[1].text, "ト");

        let emoji = word_wrap_line("✅✅✅✅✅✅", 10, None);
        assert_eq!(emoji[0].text, "✅✅✅✅✅");
        assert_eq!(emoji[1].text, "✅");

        let mixed = word_wrap_line("0123456789✅", 10, None);
        assert!(mixed.iter().all(|c| visible_width(&c.text) <= 10));
        assert_eq!(mixed[0].text, "0123456789");
        assert_eq!(mixed[1].text, "✅");
    }

    #[test]
    fn splits_oversized_atomic_paste_marker() {
        let marker = "[paste #1 +47 lines]";
        let segs = [Segment {
            text: marker.to_string(),
            index: 0,
        }];
        let chunks = word_wrap_line(marker, 8, Some(&segs));
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| visible_width(&c.text) <= 8));
        assert_eq!(
            chunks.iter().map(|c| c.text.as_str()).collect::<String>(),
            marker
        );
    }
}
