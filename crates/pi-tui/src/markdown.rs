use crate::component::{wrap, Component};
use crate::latex::{render_latex, RenderLatexOptions};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

#[derive(Debug, Clone)]
pub struct Markdown {
    pub source: String,
}

impl Markdown {
    pub fn new(source: impl Into<String>) -> Self {
        Self {
            source: source.into(),
        }
    }
}

fn expand_latex(source: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = source.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' && i + 1 < chars.len() && chars[i + 1] == '$' {
            if let Some(end) = find_delim(&chars, i + 2, &['$', '$']) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str(
                    &render_latex(inner.trim(), RenderLatexOptions { display: true })
                        .unwrap_or_else(|| chars[i..=end + 1].iter().collect()),
                );
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '[' {
            if let Some(end) = find_delim(&chars, i + 2, &['\\', ']']) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str(
                    &render_latex(inner.trim(), RenderLatexOptions { display: true })
                        .unwrap_or_else(|| chars[i..=end + 1].iter().collect()),
                );
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '\\' && i + 1 < chars.len() && chars[i + 1] == '(' {
            if let Some(end) = find_delim(&chars, i + 2, &['\\', ')']) {
                let inner: String = chars[i + 2..end].iter().collect();
                out.push_str(
                    &render_latex(inner.trim(), RenderLatexOptions { display: false })
                        .unwrap_or_else(|| chars[i..=end + 1].iter().collect()),
                );
                i = end + 2;
                continue;
            }
        }
        if chars[i] == '$' {
            if let Some(end) = find_single_dollar(&chars, i + 1) {
                let inner: String = chars[i + 1..end].iter().collect();
                if !inner.is_empty() && !inner.contains('\n') {
                    out.push_str(
                        &render_latex(&inner, RenderLatexOptions { display: false })
                            .unwrap_or_else(|| chars[i..=end].iter().collect()),
                    );
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn find_delim(chars: &[char], start: usize, delim: &[char]) -> Option<usize> {
    let mut i = start;
    while i + delim.len() <= chars.len() {
        if chars[i..i + delim.len()] == *delim {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_single_dollar(chars: &[char], start: usize) -> Option<usize> {
    (start..chars.len()).find(|&i| chars[i] == '$')
}

impl Component for Markdown {
    fn render(&self, width: usize) -> Vec<String> {
        let source = expand_latex(&self.source);
        let mut out = String::new();
        for event in Parser::new(&source) {
            match event {
                Event::Start(Tag::Heading { .. }) => out.push_str("\n# "),
                Event::End(TagEnd::Heading(_)) => out.push('\n'),
                Event::Start(Tag::Item) => out.push_str("- "),
                Event::End(TagEnd::Item) => out.push('\n'),
                Event::Start(Tag::CodeBlock(_)) => out.push('\n'),
                Event::End(TagEnd::CodeBlock) => out.push('\n'),
                Event::Text(t) | Event::Code(t) => out.push_str(&t),
                Event::SoftBreak | Event::HardBreak => out.push('\n'),
                _ => {}
            }
        }
        wrap(out.trim(), width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_heading_snapshot() {
        let md = Markdown::new("# Title\n\nHello **world**");
        let lines = md.render(40);
        assert!(lines.iter().any(|l| l.contains("Title")));
        assert!(lines.iter().any(|l| l.contains("Hello")));
    }

    #[test]
    fn markdown_expands_inline_and_display_latex() {
        let md = Markdown::new("area $A = \\pi r^2$ and $$\\sum_{i=0}^n x_i$$");
        let text = md.render(80).join("\n");
        assert!(text.contains("π"), "{text}");
        assert!(text.contains("A") && text.contains("r"), "{text}");
    }
}
