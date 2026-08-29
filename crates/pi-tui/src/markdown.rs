use crate::component::{wrap, Component};
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

impl Component for Markdown {
    fn render(&self, width: usize) -> Vec<String> {
        let mut out = String::new();
        for event in Parser::new(&self.source) {
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
}
