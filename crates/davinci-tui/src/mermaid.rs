//! Mermaid markdown transformer matching TS `mermaid.ts` + grok-mermaid flowchart subset.

use crate::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MermaidMode {
    Off,
    Final,
    Streaming,
}

impl MermaidMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "off" => Self::Off,
            "final" => Self::Final,
            _ => Self::Streaming,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Final => "final",
            Self::Streaming => "streaming",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MermaidSpanClass {
    Border,
    Text,
    Edge,
    EdgeLabel,
    Title,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidSpan {
    pub cls: MermaidSpanClass,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidArt {
    pub plain: Vec<String>,
    pub styled: Vec<Vec<MermaidSpan>>,
    pub width: usize,
    pub warnings: Vec<String>,
}

pub struct MermaidContext<'a> {
    pub available_width: usize,
    pub is_streaming: bool,
    pub message_type: &'a str,
    pub mode: MermaidMode,
}

pub trait MermaidTheme {
    fn fg(&self, color: &str, text: &str) -> String;
    fn bold(&self, text: &str) -> String;
}

impl MermaidTheme for Theme {
    fn fg(&self, color: &str, text: &str) -> String {
        Theme::fg(self, color, text)
    }

    fn bold(&self, text: &str) -> String {
        Theme::bold(self, text)
    }
}

pub fn code_span(line: &str) -> String {
    let content = if line.is_empty() {
        "\u{00a0}".to_string()
    } else {
        line.to_string()
    };
    let longest = content
        .chars()
        .collect::<Vec<_>>()
        .windows(1)
        .fold(0usize, |max, _| max);
    let mut run = 0usize;
    let mut best = 0usize;
    for ch in content.chars() {
        if ch == '`' {
            run += 1;
            best = best.max(run);
        } else {
            run = 0;
        }
    }
    let _ = longest;
    let fence = "`".repeat(best + 1);
    let padding = if content.starts_with('`') || content.ends_with('`') {
        " "
    } else {
        ""
    };
    format!("{fence}{padding}{content}{padding}{fence}")
}

fn style_span(span: &MermaidSpan, theme: &dyn MermaidTheme) -> String {
    match span.cls {
        MermaidSpanClass::Border => theme.fg("borderMuted", &span.text),
        MermaidSpanClass::Text => theme.fg("text", &span.text),
        MermaidSpanClass::Edge => theme.fg("accent", &span.text),
        MermaidSpanClass::EdgeLabel => theme.fg("muted", &span.text),
        MermaidSpanClass::Title => theme.fg("accent", &theme.bold(&span.text)),
        MermaidSpanClass::None => span.text.clone(),
    }
}

fn themed_lines(art: &MermaidArt, theme: &dyn MermaidTheme) -> Vec<String> {
    art.styled
        .iter()
        .map(|row| {
            row.iter()
                .map(|span| style_span(span, theme))
                .collect::<String>()
        })
        .collect()
}

#[derive(Debug, Clone)]
struct Node {
    id: String,
    label: String,
}

pub fn render_mermaid(source: &str) -> Option<MermaidArt> {
    let trimmed = source.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut lines = trimmed.lines();
    let header = lines.next()?.trim();
    let mut parts = header.split_whitespace();
    let kind = parts.next()?.to_ascii_lowercase();
    if kind != "flowchart" && kind != "graph" {
        return None;
    }
    let direction = parts.next().unwrap_or("TD").to_ascii_uppercase();
    let mut nodes: Vec<Node> = Vec::new();
    let mut edges: Vec<(String, String)> = Vec::new();
    let mut warnings = Vec::new();
    for raw in lines {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        parse_statement(line, &mut nodes, &mut edges, &mut warnings);
    }
    if nodes.is_empty() {
        return None;
    }
    Some(layout(&nodes, &edges, &direction, warnings))
}

fn upsert_node(nodes: &mut Vec<Node>, id: &str, label: Option<&str>) {
    if let Some(existing) = nodes.iter_mut().find(|node| node.id == id) {
        if let Some(label) = label {
            existing.label = label.to_string();
        }
        return;
    }
    nodes.push(Node {
        id: id.to_string(),
        label: label.unwrap_or(id).to_string(),
    });
}

fn parse_node_token(token: &str) -> Option<(String, Option<String>)> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if let Some(start) = token.find('[') {
        if let Some(end) = token.rfind(']') {
            let id = token[..start].trim();
            if id.is_empty() {
                return None;
            }
            let label = token[start + 1..end].to_string();
            return Some((id.to_string(), Some(label)));
        }
    }
    if let Some(start) = token.find('(') {
        if let Some(end) = token.rfind(')') {
            let id = token[..start].trim();
            if id.is_empty() {
                return None;
            }
            let label = token[start + 1..end].to_string();
            return Some((id.to_string(), Some(label)));
        }
    }
    if token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        return Some((token.to_string(), None));
    }
    None
}

fn parse_statement(
    line: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<(String, String)>,
    warnings: &mut Vec<String>,
) {
    if let Some(class_at) = line.find(":::") {
        let before = line[..class_at].trim();
        let rest = line[class_at..].to_string();
        warnings.push(format!("dropped, expected a link: \"{rest}\""));
        if let Some((id, label)) = parse_node_token(before) {
            upsert_node(nodes, &id, label.as_deref());
        }
        return;
    }
    if let Some((left, right)) = line.split_once("-->") {
        let left = left.trim().trim_end_matches('|').trim();
        let right = right.trim().trim_start_matches('|').trim();
        let right = right.split('|').next().unwrap_or(right).trim();
        if let (Some((from_id, from_label)), Some((to_id, to_label))) =
            (parse_node_token(left), parse_node_token(right))
        {
            upsert_node(nodes, &from_id, from_label.as_deref());
            upsert_node(nodes, &to_id, to_label.as_deref());
            edges.push((from_id, to_id));
            return;
        }
    }
    if let Some((id, label)) = parse_node_token(line) {
        upsert_node(nodes, &id, label.as_deref());
    }
}

fn span(cls: MermaidSpanClass, text: impl Into<String>) -> MermaidSpan {
    MermaidSpan {
        cls,
        text: text.into(),
    }
}

fn box_parts(label: &str) -> (String, String, String) {
    let inner = format!(" {label} ");
    let bar = "─".repeat(inner.chars().count());
    (format!("┌{bar}┐"), format!("│{inner}│"), format!("└{bar}┘"))
}

fn layout(
    nodes: &[Node],
    edges: &[(String, String)],
    direction: &str,
    warnings: Vec<String>,
) -> MermaidArt {
    let order = order_nodes(nodes, edges);
    let lr = direction == "LR" || direction == "RL";
    if lr {
        layout_lr(&order, edges, warnings)
    } else {
        layout_td(&order, edges, warnings)
    }
}

fn order_nodes(nodes: &[Node], edges: &[(String, String)]) -> Vec<Node> {
    let mut ordered = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if let Some((from, _)) = edges.first() {
        if let Some(node) = nodes.iter().find(|node| node.id == *from) {
            ordered.push(node.clone());
            seen.insert(node.id.clone());
        }
    }
    for (from, to) in edges {
        for id in [from, to] {
            if seen.insert(id.clone()) {
                if let Some(node) = nodes.iter().find(|node| node.id == *id) {
                    ordered.push(node.clone());
                }
            }
        }
    }
    for node in nodes {
        if seen.insert(node.id.clone()) {
            ordered.push(node.clone());
        }
    }
    ordered
}

fn layout_lr(nodes: &[Node], _edges: &[(String, String)], warnings: Vec<String>) -> MermaidArt {
    let boxes: Vec<(String, String, String)> =
        nodes.iter().map(|node| box_parts(&node.label)).collect();
    if boxes.is_empty() {
        return MermaidArt {
            plain: Vec::new(),
            styled: Vec::new(),
            width: 0,
            warnings,
        };
    }
    let arrow = "───▶";
    let gap = "    ";
    let mut top = String::new();
    let mut mid = String::new();
    let mut bot = String::new();
    let mut top_spans = Vec::new();
    let mut mid_spans = Vec::new();
    let mut bot_spans = Vec::new();
    for (index, (t, _m, b)) in boxes.iter().enumerate() {
        if index > 0 {
            top.push_str(gap);
            top_spans.push(span(MermaidSpanClass::None, gap));
            mid.push_str(arrow);
            mid_spans.push(span(MermaidSpanClass::Edge, arrow));
            bot.push_str(gap);
            bot_spans.push(span(MermaidSpanClass::None, gap));
        }
        top.push_str(t);
        top_spans.push(span(MermaidSpanClass::Border, t.clone()));
        let label = &nodes[index].label;
        let connect_right = index + 1 < boxes.len();
        let left = "│";
        let right = if connect_right { "├" } else { "│" };
        let inner = format!(" {label} ");
        mid.push_str(left);
        mid.push_str(&inner);
        mid.push_str(right);
        mid_spans.push(span(MermaidSpanClass::Border, left));
        mid_spans.push(span(MermaidSpanClass::Text, inner));
        mid_spans.push(span(MermaidSpanClass::Border, right));
        bot.push_str(b);
        bot_spans.push(span(MermaidSpanClass::Border, b.clone()));
    }
    let plain = vec![top.clone(), mid.clone(), bot.clone()];
    let width = plain
        .iter()
        .map(|line| crate::render::visible_width(line))
        .max()
        .unwrap_or(0);
    MermaidArt {
        plain,
        styled: vec![top_spans, mid_spans, bot_spans],
        width,
        warnings,
    }
}

fn layout_td(nodes: &[Node], _edges: &[(String, String)], warnings: Vec<String>) -> MermaidArt {
    let mut plain = Vec::new();
    let mut styled = Vec::new();
    for (index, node) in nodes.iter().enumerate() {
        let (t, m, b) = box_parts(&node.label);
        plain.push(t.clone());
        plain.push(m.clone());
        plain.push(b.clone());
        styled.push(vec![span(MermaidSpanClass::Border, t)]);
        styled.push(vec![
            span(MermaidSpanClass::Border, "│"),
            span(MermaidSpanClass::Text, format!(" {} ", node.label)),
            span(MermaidSpanClass::Border, "│"),
        ]);
        styled.push(vec![span(MermaidSpanClass::Border, b)]);
        if index + 1 < nodes.len() {
            plain.push("  ▼".into());
            styled.push(vec![span(MermaidSpanClass::Edge, "  ▼")]);
        }
    }
    let width = plain
        .iter()
        .map(|line| crate::render::visible_width(line))
        .max()
        .unwrap_or(0);
    MermaidArt {
        plain,
        styled,
        width,
        warnings,
    }
}

struct FenceToken {
    raw: String,
    mermaid: Option<String>,
}

fn lex_fences(markdown: &str) -> Vec<FenceToken> {
    let mut tokens = Vec::new();
    let bytes = markdown.as_bytes();
    let mut i = 0;
    let mut raw = String::new();
    while i < bytes.len() {
        if markdown[i..].starts_with("```") {
            let after = &markdown[i + 3..];
            let (info, rest) = after.split_once('\n').unwrap_or((after, ""));
            let lang = info.split_whitespace().next().unwrap_or("");
            if lang.eq_ignore_ascii_case("mermaid") {
                if !raw.is_empty() {
                    tokens.push(FenceToken {
                        raw: std::mem::take(&mut raw),
                        mermaid: None,
                    });
                }
                if let Some(end) = rest.find("```") {
                    let body = &rest[..end];
                    let after_fence = &rest[end + 3..];
                    let consumed = if after_fence.starts_with('\n') { 1 } else { 0 };
                    let raw_token = format!("```{info}\n{body}```{}", &after_fence[..consumed]);
                    tokens.push(FenceToken {
                        raw: raw_token,
                        mermaid: Some(body.to_string()),
                    });
                    i = markdown.len() - rest.len() + end + 3 + consumed;
                    continue;
                }
                let raw_token = format!("```{info}\n{rest}");
                tokens.push(FenceToken {
                    raw: raw_token,
                    mermaid: Some(rest.to_string()),
                });
                break;
            }
        }
        raw.push(bytes[i] as char);
        i += 1;
    }
    if !raw.is_empty() {
        tokens.push(FenceToken { raw, mermaid: None });
    }
    tokens
}

pub fn transform_mermaid(
    markdown: &str,
    ctx: MermaidContext<'_>,
    theme: Option<&dyn MermaidTheme>,
) -> String {
    if ctx.mode == MermaidMode::Off
        || ctx.message_type == "assistant-thinking"
        || (ctx.is_streaming && ctx.mode != MermaidMode::Streaming)
    {
        return markdown.to_string();
    }
    let mut out = String::new();
    for token in lex_fences(markdown) {
        let Some(source) = token.mermaid else {
            out.push_str(&token.raw);
            continue;
        };
        let Some(art) = render_mermaid(&source) else {
            out.push_str(&token.raw);
            continue;
        };
        if art.width > ctx.available_width {
            out.push_str(&token.raw);
            continue;
        }
        if !ctx.is_streaming && !art.warnings.is_empty() {
            let suffix = if art.warnings.len() > 1 {
                format!(" (+{} more)", art.warnings.len() - 1)
            } else {
                String::new()
            };
            let warning = format!("Mermaid diagram not rendered: {}{suffix}", art.warnings[0]);
            let styled = match theme {
                Some(theme) => theme.fg("warning", &warning),
                None => warning,
            };
            out.push_str(&token.raw);
            out.push('\n');
            out.push_str(&code_span(&styled));
            out.push_str("  \n");
            continue;
        }
        let lines = match theme {
            Some(theme) => themed_lines(&art, theme),
            None => art.plain,
        };
        out.push_str(
            &lines
                .iter()
                .map(|line| code_span(line))
                .collect::<Vec<_>>()
                .join("  \n"),
        );
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TagTheme;

    impl MermaidTheme for TagTheme {
        fn fg(&self, color: &str, text: &str) -> String {
            format!("<{color}>{text}</{color}>")
        }
        fn bold(&self, text: &str) -> String {
            format!("<bold>{text}</bold>")
        }
    }

    fn transform(markdown: &str, ctx: MermaidContext<'_>) -> String {
        transform_mermaid(markdown, ctx, None)
    }

    fn default_ctx<'a>(width: usize, streaming: bool, mode: MermaidMode) -> MermaidContext<'a> {
        MermaidContext {
            available_width: width,
            is_streaming: streaming,
            message_type: "assistant",
            mode,
        }
    }

    #[test]
    fn replaces_mermaid_blocks_with_unicode_diagrams() {
        let markdown = "Before\n\n```mermaid\nflowchart LR\n  A[Start] --> B[Done]\n```\nAfter";
        let rendered = transform(markdown, default_ctx(100, false, MermaidMode::Streaming));
        assert!(rendered.contains("Before"));
        assert!(rendered.contains("┌───────┐"));
        assert!(rendered.contains("│ Start ├───▶│ Done │"));
        assert!(rendered.contains("└───────┘    └──────┘`\nAfter"));
        assert!(!rendered.contains("```mermaid"));
        assert!(rendered.contains("After"));
    }

    #[test]
    fn leaves_unsupported_and_oversized_unchanged() {
        let unsupported = "```mermaid\npie\n  title Pets\n  \"Dogs\" : 4\n```";
        let oversized = "```mermaid\nflowchart LR\n  A[Start] --> B[Done]\n```";
        assert_eq!(
            transform(unsupported, default_ctx(100, false, MermaidMode::Streaming)),
            unsupported
        );
        assert_eq!(
            transform(oversized, default_ctx(10, false, MermaidMode::Streaming)),
            oversized
        );
    }

    #[test]
    fn maps_semantic_spans_through_theme() {
        let rendered = transform_mermaid(
            "```mermaid\nflowchart LR\n  A --> B\n```",
            default_ctx(100, false, MermaidMode::Streaming),
            Some(&TagTheme),
        );
        assert!(rendered.contains("<borderMuted>"));
        assert!(rendered.contains("<accent>"));
    }

    #[test]
    fn renders_incomplete_blocks_during_streaming() {
        let partial = "```mermaid\nflowchart LR\n  A --> B";
        let rendered = transform(partial, default_ctx(100, true, MermaidMode::Streaming));
        assert!(rendered.contains("───▶"));
    }

    #[test]
    fn falls_back_to_code_block_with_warning_after_streaming() {
        let markdown = "```mermaid\nflowchart LR\n  A[Foo]:::highlight --> B[Bar]\n```";
        let final_out = transform(markdown, default_ctx(100, false, MermaidMode::Streaming));
        let followed = transform(
            &format!("{markdown}\nFollowing text"),
            default_ctx(100, false, MermaidMode::Streaming),
        );
        let streaming = transform(markdown, default_ctx(100, true, MermaidMode::Streaming));
        assert!(final_out.contains(markdown));
        assert!(final_out.contains("```\n`Mermaid diagram not rendered"));
        assert!(final_out.contains("dropped, expected a link: \":::highlight --> B[Bar]\""));
        assert!(!final_out.contains("more)"));
        assert!(followed.contains("  \nFollowing text"));
        assert!(!streaming.contains("Mermaid diagram not rendered"));
        assert!(!streaming.contains("```mermaid"));
        assert!(streaming.contains("│ Foo │"));
    }

    #[test]
    fn summarizes_additional_partial_render_warnings() {
        let markdown =
            "```mermaid\nflowchart LR\n  A[Foo]:::highlight --> B[Bar]\n  C[Baz]:::other --> D[Qux]\n```";
        let rendered = transform(markdown, default_ctx(100, false, MermaidMode::Streaming));
        assert!(rendered.contains(markdown));
        assert!(rendered.contains("dropped, expected a link: \":::highlight --> B[Bar]\""));
        assert!(rendered.contains("(+1 more)"));
        assert!(!rendered.contains("dropped, expected a link: \":::other --> D[Qux]\""));
    }

    #[test]
    fn respects_modes_and_skips_thinking() {
        let markdown = "```mermaid\nflowchart LR\n  A --> B\n```";
        assert_eq!(
            transform(markdown, default_ctx(100, false, MermaidMode::Off)),
            markdown
        );
        assert_eq!(
            transform(markdown, default_ctx(100, true, MermaidMode::Final)),
            markdown
        );
        assert!(
            !transform(markdown, default_ctx(100, false, MermaidMode::Final))
                .contains("```mermaid")
        );
        let thinking = MermaidContext {
            available_width: 100,
            is_streaming: false,
            message_type: "assistant-thinking",
            mode: MermaidMode::Streaming,
        };
        assert_eq!(transform(markdown, thinking), markdown);
    }
}
