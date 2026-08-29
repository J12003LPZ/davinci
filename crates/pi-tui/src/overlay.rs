use crate::image::KITTY_IMAGE_PREFIX;
use crate::render::{visible_width, Component};

const SEGMENT_RESET: &str = "\x1b[0m\x1b]8;;\x07";
const ITERM_PREFIX: &str = "\x1b]1337;";

/// Overlay host matching TS `OverlayHandle` / overlay stack.
pub struct Overlay {
    pub title: String,
    child: Box<dyn Component>,
}

impl Overlay {
    pub fn new(title: impl Into<String>, child: Box<dyn Component>) -> Self {
        Self {
            title: title.into(),
            child,
        }
    }
}

impl Component for Overlay {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(2);
        let mut lines = vec![format!("┌{}┐", "─".repeat(inner))];
        let title = format!(" {} ", self.title);
        lines.push(format!("│{title:<inner$}│"));
        for line in self.child.render(inner.saturating_sub(2)) {
            let mut padded = format!("│ {line}");
            while visible_width(&padded) < width.saturating_sub(1) {
                padded.push(' ');
            }
            padded.push('│');
            lines.push(padded);
        }
        lines.push(format!("└{}┘", "─".repeat(inner)));
        lines
    }

    fn handle_input(&mut self, data: &str) {
        self.child.handle_input(data);
    }

    fn invalidate(&mut self) {
        self.child.invalidate();
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayMargin {
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
    pub left: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayOptions {
    pub width: Option<SizeValue>,
    pub min_width: Option<usize>,
    pub max_height: Option<SizeValue>,
    pub anchor: OverlayAnchor,
    pub offset_x: i32,
    pub offset_y: i32,
    pub row: Option<SizeValue>,
    pub col: Option<SizeValue>,
    pub margin: OverlayMargin,
}

impl Default for OverlayOptions {
    fn default() -> Self {
        Self {
            width: None,
            min_width: None,
            max_height: None,
            anchor: OverlayAnchor::Center,
            offset_x: 0,
            offset_y: 0,
            row: None,
            col: None,
            margin: OverlayMargin {
                top: 0,
                right: 0,
                bottom: 0,
                left: 0,
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    LeftCenter,
    Center,
    RightCenter,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl OverlayAnchor {
    pub fn parse(value: &str) -> Self {
        match value {
            "top-left" => Self::TopLeft,
            "top-center" => Self::TopCenter,
            "top-right" => Self::TopRight,
            "left-center" => Self::LeftCenter,
            "right-center" => Self::RightCenter,
            "bottom-left" => Self::BottomLeft,
            "bottom-center" => Self::BottomCenter,
            "bottom-right" => Self::BottomRight,
            _ => Self::Center,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SizeValue {
    Absolute(usize),
    Percent(f64),
}

impl SizeValue {
    pub fn parse(value: &serde_json::Value) -> Option<Self> {
        if let Some(number) = value.as_u64() {
            return Some(Self::Absolute(number as usize));
        }
        if let Some(number) = value.as_f64() {
            return Some(Self::Absolute(number.max(0.0) as usize));
        }
        let text = value.as_str()?;
        parse_size_text(text)
    }

    fn resolve(&self, reference: usize) -> usize {
        match self {
            Self::Absolute(value) => *value,
            Self::Percent(percent) => ((reference as f64) * percent / 100.0).floor() as usize,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayLayout {
    pub width: usize,
    pub row: usize,
    pub col: usize,
    pub max_height: Option<usize>,
}

pub fn overlay_options_from_json(value: &serde_json::Value) -> OverlayOptions {
    OverlayOptions {
        width: value.get("width").and_then(SizeValue::parse),
        min_width: value
            .get("minWidth")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize),
        max_height: value.get("maxHeight").and_then(SizeValue::parse),
        anchor: value
            .get("anchor")
            .and_then(serde_json::Value::as_str)
            .map(OverlayAnchor::parse)
            .unwrap_or(OverlayAnchor::Center),
        offset_x: value
            .get("offsetX")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32,
        offset_y: value
            .get("offsetY")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or(0) as i32,
        row: value.get("row").and_then(SizeValue::parse),
        col: value.get("col").and_then(SizeValue::parse),
        margin: parse_margin(value.get("margin")),
    }
}

pub fn resolve_overlay_layout(
    options: &OverlayOptions,
    overlay_height: usize,
    term_width: usize,
    term_height: usize,
) -> OverlayLayout {
    let margin_top = options.margin.top;
    let margin_right = options.margin.right;
    let margin_bottom = options.margin.bottom;
    let margin_left = options.margin.left;
    let avail_width = term_width.saturating_sub(margin_left + margin_right).max(1);
    let avail_height = term_height
        .saturating_sub(margin_top + margin_bottom)
        .max(1);
    let mut width = options
        .width
        .as_ref()
        .map(|value| value.resolve(term_width))
        .unwrap_or_else(|| 80.min(avail_width));
    if let Some(min_width) = options.min_width {
        width = width.max(min_width);
    }
    width = width.clamp(1, avail_width);
    let mut max_height = options
        .max_height
        .as_ref()
        .map(|value| value.resolve(term_height));
    if let Some(height) = max_height.as_mut() {
        *height = (*height).clamp(1, avail_height);
    }
    let effective_height = max_height
        .map(|height| overlay_height.min(height))
        .unwrap_or(overlay_height);
    let mut row = match &options.row {
        Some(SizeValue::Percent(percent)) => {
            let max_row = avail_height.saturating_sub(effective_height);
            margin_top + ((max_row as f64) * percent / 100.0).floor() as usize
        }
        Some(SizeValue::Absolute(value)) => *value,
        None => resolve_anchor_row(options.anchor, effective_height, avail_height, margin_top),
    };
    let mut col = match &options.col {
        Some(SizeValue::Percent(percent)) => {
            let max_col = avail_width.saturating_sub(width);
            margin_left + ((max_col as f64) * percent / 100.0).floor() as usize
        }
        Some(SizeValue::Absolute(value)) => *value,
        None => resolve_anchor_col(options.anchor, width, avail_width, margin_left),
    };
    row = add_offset(row, options.offset_y);
    col = add_offset(col, options.offset_x);
    let max_row = term_height.saturating_sub(margin_bottom + effective_height);
    let max_col = term_width.saturating_sub(margin_right + width);
    row = row.clamp(margin_top, max_row);
    col = col.clamp(margin_left, max_col);
    OverlayLayout {
        width,
        row,
        col,
        max_height,
    }
}

pub fn resolve_anchor_row(
    anchor: OverlayAnchor,
    height: usize,
    avail_height: usize,
    margin_top: usize,
) -> usize {
    match anchor {
        OverlayAnchor::TopLeft | OverlayAnchor::TopCenter | OverlayAnchor::TopRight => margin_top,
        OverlayAnchor::BottomLeft | OverlayAnchor::BottomCenter | OverlayAnchor::BottomRight => {
            margin_top + avail_height.saturating_sub(height)
        }
        OverlayAnchor::LeftCenter | OverlayAnchor::Center | OverlayAnchor::RightCenter => {
            margin_top + avail_height.saturating_sub(height) / 2
        }
    }
}

pub fn resolve_anchor_col(
    anchor: OverlayAnchor,
    width: usize,
    avail_width: usize,
    margin_left: usize,
) -> usize {
    match anchor {
        OverlayAnchor::TopLeft | OverlayAnchor::LeftCenter | OverlayAnchor::BottomLeft => {
            margin_left
        }
        OverlayAnchor::TopRight | OverlayAnchor::RightCenter | OverlayAnchor::BottomRight => {
            margin_left + avail_width.saturating_sub(width)
        }
        OverlayAnchor::TopCenter | OverlayAnchor::Center | OverlayAnchor::BottomCenter => {
            margin_left + avail_width.saturating_sub(width) / 2
        }
    }
}

pub fn composite_overlay_lines(
    base: &[String],
    overlay: &[String],
    options: &OverlayOptions,
    term_width: usize,
    term_height: usize,
) -> Vec<String> {
    let layout = resolve_overlay_layout(options, overlay.len(), term_width, term_height);
    let overlay_lines: Vec<String> = if let Some(max_height) = layout.max_height {
        overlay.iter().take(max_height).cloned().collect()
    } else {
        overlay.to_vec()
    };
    let needed = layout.row + overlay_lines.len();
    let mut result: Vec<String> = base.to_vec();
    while result.len() < needed.max(term_height.min(base.len().max(needed))) {
        result.push(String::new());
    }
    if result.len() < needed {
        result.resize(needed, String::new());
    }
    for (index, line) in overlay_lines.iter().enumerate() {
        let row = layout.row + index;
        if row >= result.len() {
            result.resize(row + 1, String::new());
        }
        result[row] = composite_tui_line(&result[row], line, layout.col, layout.width, term_width);
    }
    result
}

/// Composite overlay content into a terminal line at a fixed column.
pub fn composite_tui_line(
    base_line: &str,
    overlay_line: &str,
    start_col: usize,
    overlay_width: usize,
    total_width: usize,
) -> String {
    if is_image_line(base_line) {
        return base_line.to_string();
    }
    let after_start = start_col + overlay_width;
    let base = extract_segments(
        base_line,
        start_col,
        after_start,
        total_width.saturating_sub(after_start),
    );
    let overlay = slice_with_width(overlay_line, 0, overlay_width);
    let before_pad = start_col.saturating_sub(base.before_width);
    let overlay_pad = overlay_width.saturating_sub(overlay.width);
    let actual_before = start_col.max(base.before_width);
    let actual_overlay = overlay_width.max(overlay.width);
    let after_target = total_width
        .saturating_sub(actual_before)
        .saturating_sub(actual_overlay);
    let after_pad = after_target.saturating_sub(base.after_width);
    let result = format!(
        "{}{}{SEGMENT_RESET}{}{}{SEGMENT_RESET}{}{}",
        base.before,
        " ".repeat(before_pad),
        overlay.text,
        " ".repeat(overlay_pad),
        base.after,
        " ".repeat(after_pad)
    );
    if visible_width_plain(&result) <= total_width {
        result
    } else {
        slice_by_column(&result, 0, total_width)
    }
}

fn is_image_line(line: &str) -> bool {
    line.contains(KITTY_IMAGE_PREFIX) || line.contains(ITERM_PREFIX)
}

struct Segments {
    before: String,
    before_width: usize,
    after: String,
    after_width: usize,
}

fn extract_segments(line: &str, start: usize, end: usize, _after_target: usize) -> Segments {
    let chars = visible_chars(line);
    let mut before = String::new();
    let mut before_width = 0;
    let mut after = String::new();
    let mut after_width = 0;
    let mut col = 0;
    for (ch, width) in chars {
        let next = col + width;
        if next <= start {
            before.push_str(&ch);
            before_width += width;
        } else if col >= end {
            after.push_str(&ch);
            after_width += width;
        }
        col = next;
    }
    Segments {
        before,
        before_width,
        after,
        after_width,
    }
}

struct Sliced {
    text: String,
    width: usize,
}

fn slice_with_width(line: &str, start: usize, width: usize) -> Sliced {
    let text = slice_by_column(line, start, width);
    Sliced {
        width: visible_width_plain(&text),
        text,
    }
}

fn slice_by_column(line: &str, start: usize, width: usize) -> String {
    let chars = visible_chars(line);
    let mut out = String::new();
    let mut col = 0;
    let end = start + width;
    for (ch, ch_width) in chars {
        let next = col + ch_width;
        if next > start && col < end {
            if col < start || next > end {
                out.push_str(&" ".repeat(ch_width.min(end.saturating_sub(start.max(col)))));
            } else {
                out.push_str(&ch);
            }
        }
        col = next;
        if col >= end {
            break;
        }
    }
    out
}

fn visible_chars(line: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            let mut seq = String::from(ch);
            if let Some(next) = chars.next() {
                seq.push(next);
                if next == '[' {
                    for item in chars.by_ref() {
                        seq.push(item);
                        if item.is_ascii_alphabetic() {
                            break;
                        }
                    }
                } else if next == ']' {
                    for item in chars.by_ref() {
                        seq.push(item);
                        if item == '\u{7}' {
                            break;
                        }
                    }
                } else if next == '_' {
                    for item in chars.by_ref() {
                        seq.push(item);
                        if item == '\\' {
                            break;
                        }
                    }
                }
            }
            out.push((seq, 0));
            continue;
        }
        let width = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        out.push((ch.to_string(), width));
    }
    out
}

fn visible_width_plain(line: &str) -> usize {
    visible_chars(line)
        .into_iter()
        .map(|(_, width)| width)
        .sum()
}

fn parse_size_text(text: &str) -> Option<SizeValue> {
    if let Some(stripped) = text.strip_suffix('%') {
        return stripped.parse().ok().map(SizeValue::Percent);
    }
    text.parse().ok().map(SizeValue::Absolute)
}

fn parse_margin(value: Option<&serde_json::Value>) -> OverlayMargin {
    match value {
        Some(serde_json::Value::Number(number)) => {
            let n = number.as_u64().unwrap_or(0) as usize;
            OverlayMargin {
                top: n,
                right: n,
                bottom: n,
                left: n,
            }
        }
        Some(serde_json::Value::Object(map)) => OverlayMargin {
            top: map
                .get("top")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
            right: map
                .get("right")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
            bottom: map
                .get("bottom")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
            left: map
                .get("left")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0) as usize,
        },
        _ => OverlayMargin {
            top: 0,
            right: 0,
            bottom: 0,
            left: 0,
        },
    }
}

fn add_offset(value: usize, offset: i32) -> usize {
    if offset >= 0 {
        value.saturating_add(offset as usize)
    } else {
        value.saturating_sub((-offset) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composites_overlay_inside_wide_grapheme() {
        let out = composite_tui_line("abcd让EFGH", "│XX│", 5, 4, 20);
        assert!(!out.contains('让'));
        assert_eq!(visible_width_plain(&out), 20);
        assert!(slice_by_column(&out, 5, 4).contains("│XX│"));
    }

    #[test]
    fn composites_overlay_at_wide_grapheme_boundary() {
        let out = composite_tui_line("abcd让EFGH", "│XX│", 4, 4, 20);
        assert!(!out.contains('让'));
        assert_eq!(visible_width_plain(&out), 20);
        assert!(slice_by_column(&out, 4, 4).contains("│XX│"));
    }

    #[test]
    fn centers_overlay_by_default() {
        let layout = resolve_overlay_layout(&OverlayOptions::default(), 3, 80, 24);
        assert_eq!(layout.col, 0);
        let options = OverlayOptions {
            anchor: OverlayAnchor::TopRight,
            width: Some(SizeValue::Absolute(10)),
            ..OverlayOptions::default()
        };
        let layout = resolve_overlay_layout(&options, 2, 40, 10);
        assert_eq!(layout.col, 30);
        assert_eq!(layout.row, 0);
    }
}
