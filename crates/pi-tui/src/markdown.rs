use pulldown_cmark::{CodeBlockKind, Event, Parser, Tag, TagEnd};

use crate::render::wrap_text;

pub const DEFAULT_CODE_BLOCK_INDENT: &str = "  ";

/// OSC 8 hyperlink matching TS `hyperlink()` in `terminal-image.ts`.
pub fn osc8_hyperlink(text: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

/// TS `getCapabilities().hyperlinks` plus `PI_HYPERLINKS` / `PI_TERMINAL_HYPERLINKS`.
/// Unknown terminals default off (TS `detectCapabilities`).
pub fn hyperlinks_enabled() -> bool {
    for key in ["PI_HYPERLINKS", "PI_TERMINAL_HYPERLINKS"] {
        if let Ok(value) = std::env::var(key) {
            let value = value.to_ascii_lowercase();
            if matches!(value.as_str(), "1" | "true" | "yes") {
                return true;
            }
            if matches!(value.as_str(), "0" | "false" | "no") {
                return false;
            }
        }
    }
    false
}

pub fn format_markdown_link(text: &str, href: &str) -> String {
    if hyperlinks_enabled() {
        return osc8_hyperlink(text, href);
    }
    let href_for_comparison = href.strip_prefix("mailto:").unwrap_or(href);
    if text == href || text == href_for_comparison {
        text.to_string()
    } else {
        format!("{text} ({href})")
    }
}

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
    let mut link_href: Option<String> = None;
    let mut link_text = String::new();
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
            Event::Start(Tag::Link { dest_url, .. }) => {
                link_href = Some(dest_url.to_string());
                link_text.clear();
            }
            Event::End(TagEnd::Link) => {
                if let Some(href) = link_href.take() {
                    let text = if link_text.is_empty() {
                        href.clone()
                    } else {
                        std::mem::take(&mut link_text)
                    };
                    current.push_str(&format_markdown_link(&text, &href));
                }
            }
            Event::Text(text) if in_code => code.push_str(&text),
            Event::Text(text) if link_href.is_some() => link_text.push_str(&text),
            Event::Text(text) => current.push_str(&text),
            Event::Code(inline) if link_href.is_some() => link_text.push_str(&inline),
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
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn code_block_indent_matches_ts_default() {
        let source = "```rust\nfn main() {}\n```";
        let default_lines = render_markdown(source, 80);
        assert!(default_lines.iter().any(|line| line == "```rust"));
        assert!(default_lines.iter().any(|line| line == "  fn main() {}"));
        let custom = render_markdown_with(source, 80, "\t");
        assert!(custom.iter().any(|line| line == "\tfn main() {}"));
    }

    #[test]
    fn links_use_osc8_when_enabled_and_fallback_href() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("PI_HYPERLINKS");
        std::env::remove_var("PI_TERMINAL_HYPERLINKS");
        let fallback = render_markdown("[docs](https://pi.dev)", 80);
        assert!(fallback
            .iter()
            .any(|line| line.contains("docs (https://pi.dev)")));
        let same = render_markdown("<https://pi.dev>", 80);
        assert!(same.iter().any(|line| line.contains("https://pi.dev")));
        assert!(!same.iter().any(|line| line.contains(" (https://pi.dev)")));
        std::env::set_var("PI_HYPERLINKS", "1");
        let linked = render_markdown("[docs](https://pi.dev)", 80);
        std::env::remove_var("PI_HYPERLINKS");
        assert!(linked
            .iter()
            .any(|line| line.contains("\x1b]8;;https://pi.dev\x1b\\")));
        assert_eq!(
            osc8_hyperlink("docs", "https://pi.dev"),
            "\x1b]8;;https://pi.dev\x1b\\docs\x1b]8;;\x1b\\"
        );
    }
}
