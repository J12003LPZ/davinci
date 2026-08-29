use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::render::wrap_text;

pub const DEFAULT_CODE_BLOCK_INDENT: &str = "  ";

pub fn render_markdown(source: &str, width: usize) -> Vec<String> {
    render_markdown_with(source, width, DEFAULT_CODE_BLOCK_INDENT)
}

/// TS `Markdown` `theme.codeBlockIndent` (default two spaces).
pub fn render_markdown_with(source: &str, width: usize, code_block_indent: &str) -> Vec<String> {
    let parser = Parser::new(source);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut in_code = false;
    let mut code = String::new();
    let mut code_lang = String::new();
    for event in parser {
        match event {
            Event::Start(Tag::Heading { .. }) => current.push_str("# "),
            Event::End(TagEnd::Heading(_)) => flush(&mut current, &mut lines, width),
            Event::Start(Tag::Item) => current.push_str("- "),
            Event::End(TagEnd::Item) => flush(&mut current, &mut lines, width),
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut current, &mut lines, width);
                in_code = true;
                code.clear();
                code_lang = match kind {
                    CodeBlockKind::Fenced(lang) => lang.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
            }
            Event::End(TagEnd::CodeBlock) => {
                lines.push(format!("```{code_lang}"));
                let body = code.strip_suffix('\n').unwrap_or(&code);
                for line in body.split('\n') {
                    lines.push(format!("{code_block_indent}{line}"));
                }
                lines.push("```".into());
                lines.push(String::new());
                in_code = false;
                code.clear();
                code_lang.clear();
            }
            Event::Text(text) if in_code => code.push_str(&text),
            Event::Text(text) => current.push_str(&text),
            Event::Code(inline) => {
                current.push('`');
                current.push_str(&inline);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_block_indent_matches_ts_default() {
        let source = "```rust\nfn main() {}\n```";
        let default_lines = render_markdown(source, 80);
        assert!(default_lines.iter().any(|line| line == "```rust"));
        assert!(default_lines.iter().any(|line| line == "  fn main() {}"));
        let custom = render_markdown_with(source, 80, "\t");
        assert!(custom.iter().any(|line| line == "\tfn main() {}"));
    }
}
