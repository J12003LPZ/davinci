use pulldown_cmark::{Event, Parser, Tag, TagEnd};

use crate::render::wrap_text;

pub fn render_markdown(source: &str, width: usize) -> Vec<String> {
    let parser = Parser::new(source);
    let mut lines = Vec::new();
    let mut current = String::new();
    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => current.push_str("# "),
            Event::End(TagEnd::Heading(_)) => flush(&mut current, &mut lines, width),
            Event::Start(Tag::Item) => current.push_str("- "),
            Event::End(TagEnd::Item) => flush(&mut current, &mut lines, width),
            Event::Text(text) => current.push_str(&text),
            Event::Code(code) => {
                current.push('`');
                current.push_str(&code);
                current.push('`');
            }
            Event::SoftBreak | Event::HardBreak => flush(&mut current, &mut lines, width),
            Event::End(TagEnd::Paragraph) => flush(&mut current, &mut lines, width),
            _ => {}
        }
    }
    if !current.is_empty() {
        flush(&mut current, &mut lines, width);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn flush(current: &mut String, lines: &mut Vec<String>, width: usize) {
    if current.is_empty() {
        return;
    }
    lines.extend(wrap_text(current, width));
    current.clear();
}
