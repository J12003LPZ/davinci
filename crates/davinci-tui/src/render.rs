use std::any::Any;
use std::collections::BTreeMap;
use std::rc::Rc;

use unicode_width::UnicodeWidthStr;

pub trait AsAny {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

impl<T: Any> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

/// Dense or sparse line buffer matching JS `string[]` holes used by huge transcripts.
#[derive(Clone)]
pub struct RenderedLines {
    inner: Rc<RenderedLinesInner>,
}

enum RenderedLinesInner {
    Dense(Vec<String>),
    Sparse {
        count: usize,
        lines: BTreeMap<usize, String>,
    },
}

impl RenderedLines {
    pub fn dense(lines: Vec<String>) -> Self {
        Self {
            inner: Rc::new(RenderedLinesInner::Dense(lines)),
        }
    }

    pub fn sparse(count: usize, lines: BTreeMap<usize, String>) -> Self {
        Self {
            inner: Rc::new(RenderedLinesInner::Sparse { count, lines }),
        }
    }

    pub fn empty() -> Self {
        Self::dense(Vec::new())
    }

    pub fn len(&self) -> usize {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => lines.len(),
            RenderedLinesInner::Sparse { count, .. } => *count,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => lines.get(index).map(String::as_str),
            RenderedLinesInner::Sparse { lines, .. } => lines.get(&index).map(String::as_str),
        }
    }

    pub fn defined(&self) -> Vec<(usize, &str)> {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => lines
                .iter()
                .enumerate()
                .map(|(index, line)| (index, line.as_str()))
                .collect(),
            RenderedLinesInner::Sparse { lines, .. } => lines
                .iter()
                .map(|(index, line)| (*index, line.as_str()))
                .collect(),
        }
    }

    pub fn max_visible_width(&self) -> usize {
        self.defined()
            .into_iter()
            .map(|(_, line)| visible_width(line))
            .max()
            .unwrap_or(0)
    }

    pub fn find_containing(&self, needle: &str) -> Option<usize> {
        self.defined()
            .into_iter()
            .find(|(_, line)| line.contains(needle))
            .map(|(index, _)| index)
    }

    pub fn to_dense_vec(&self) -> Option<Vec<String>> {
        match self.inner.as_ref() {
            RenderedLinesInner::Dense(lines) => Some(lines.clone()),
            RenderedLinesInner::Sparse { .. } => None,
        }
    }
}

impl Default for RenderedLines {
    fn default() -> Self {
        Self::dense(Vec::new())
    }
}

pub trait Component: AsAny {
    fn render(&self, width: usize) -> Vec<String>;
    fn rendered_lines(&self, width: usize) -> RenderedLines {
        RenderedLines::dense(self.render(width))
    }
    fn handle_input(&mut self, _data: &str) {}
    fn invalidate(&mut self);
    fn wants_key_release(&self) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct Text {
    pub value: String,
}

impl Component for Text {
    fn render(&self, width: usize) -> Vec<String> {
        wrap_text(&self.value, width)
    }

    fn invalidate(&mut self) {}
}

/// Non-allocating huge transcript source (JS sparse `string[]`).
pub struct SparseLines {
    pub count: usize,
    pub lines: BTreeMap<usize, String>,
}

impl SparseLines {
    pub fn new(count: usize) -> Self {
        Self {
            count,
            lines: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, index: usize, line: impl Into<String>) {
        self.lines.insert(index, line.into());
    }
}

impl Component for SparseLines {
    fn render(&self, _width: usize) -> Vec<String> {
        Vec::new()
    }

    fn rendered_lines(&self, _width: usize) -> RenderedLines {
        RenderedLines::sparse(self.count, self.lines.clone())
    }

    fn invalidate(&mut self) {}
}

pub fn visible_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// TS `visibleWidth`: strip ANSI/OSC/APC then measure columns.
pub fn visible_width_stripped(text: &str) -> usize {
    // Most lines carry no escape sequences; skip the stripped copy for them.
    if !text.contains('\x1b') {
        return visible_width(text);
    }
    visible_width(&strip_terminal_sequences(text))
}

pub fn strip_terminal_sequences(text: &str) -> String {
    if !text.contains('\x1b') {
        return text.to_string();
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == 0x1b {
            if i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                i += 2;
                while i < bytes.len() {
                    let b = bytes[i];
                    i += 1;
                    if (0x40..=0x7e).contains(&b) {
                        break;
                    }
                }
                continue;
            }
            if i + 1 < bytes.len()
                && (bytes[i + 1] == b']' || bytes[i + 1] == b'_' || bytes[i + 1] == b'^')
            {
                i += 2;
                while i < bytes.len() {
                    if bytes[i] == 0x07 {
                        i += 1;
                        break;
                    }
                    if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
                        i += 2;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for raw in text.split('\n') {
        if raw.is_empty() {
            lines.push(String::new());
            continue;
        }
        let mut current = String::new();
        // Running accumulator: recomputing `visible_width(&current)` per word
        // made wrapping quadratic in line length.
        let mut current_width = 0usize;
        for word in raw.split(' ') {
            let word_width = visible_width(word);
            if current.is_empty() {
                current = word.to_string();
                current_width = word_width;
                continue;
            }
            if current_width + 1 + word_width <= width {
                current.push(' ');
                current.push_str(word);
                current_width += 1 + word_width;
            } else {
                lines.push(current);
                current = word.to_string();
                current_width = word_width;
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    lines
}
