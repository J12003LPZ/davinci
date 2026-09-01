//! Markdown for the transcript's prose blocks. What the model writes as
//! `# heading`, `*emphasis*`, `` `code` `` and fenced code arrives here as
//! rows the transcript can count, so a streaming reply keeps its rhythm
//! (design.md §3) instead of printing raw syntax.
//!
//! The behavioural reference is the TypeScript renderer,
//! `vendor/pi/packages/tui/src/components/markdown.ts`; the visual rules are
//! davinci's own — colours only through `Theme`, a `│` left rule for code, the
//! `·` tick for bullets, a hair rule under the two top heading levels — and
//! there is no `.ex` mirror yet, so this file is the reference for the Elixir
//! tree rather than the other way round.
//!
//! The parser is `pulldown-cmark` (0.12, no cargo features). Tables are a
//! runtime option there, not a feature, so they render; the same goes for
//! strikethrough and task-list markers. Everything else — footnotes, math,
//! smart punctuation — is left off so the text stays what the model wrote.
//!
//! This runs on every frame while a reply streams, so it has to be cheap and
//! it has to render whatever prefix of a document it is handed: an unclosed
//! fence is code to the end, an unclosed `*` is a literal `*`, and no input
//! panics.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::davinci::theme::{glyph, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, hair_rule, indent, run_width, span, span_strong, truncate_run, wrap,
};

/// Below this width there is no measure to wrap to: rows are clipped instead.
const NARROW: u16 = 8;

/// Code rows sit two columns in, behind `│ `.
const CODE_INSET: u16 = 4;

/// Containers deeper than this are flattened into their parent. Rendering
/// recurses once per container, and a reply is never legitimately this deep.
const MAX_DEPTH: usize = 32;

/// Render markdown `source` as styled rows no wider than `width`.
///
/// Blocks are separated by exactly one blank row and the vector never starts
/// or ends with one. Prose wraps at `width`; code, tables and anything at a
/// width below [`NARROW`] is clipped with an ellipsis instead. A width of zero
/// has no room for anything and returns no rows.
pub fn lines(theme: &Theme, source: &str, width: u16) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let blocks = parse(source);
    let ctx = Ctx {
        theme,
        base: theme.text,
    };
    let mut rows: Vec<Line<'static>> = render_blocks(&blocks, width, &ctx, true)
        .into_iter()
        // Every row fits, whatever a nested container did with its share of
        // the width: clipped, never wrapped, so a row count stays a row count.
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect();
    while rows.first().is_some_and(is_blank) {
        rows.remove(0);
    }
    while rows.last().is_some_and(is_blank) {
        rows.pop();
    }
    rows
}

// ---------------------------------------------------------------------------
// Document model
// ---------------------------------------------------------------------------

/// How one run of inline text is drawn.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Face {
    strong: bool,
    italic: bool,
    strike: bool,
    code: bool,
    link: bool,
    html: bool,
}

impl Face {
    /// The run as a span. Colour comes from the theme only: verdigris for
    /// code and links (that is *where something is*, §2), muted for raw
    /// HTML, otherwise the block's base colour. Strong is `theme.emphasis`,
    /// which is what makes headings bold under `NO_COLOR` (§9).
    fn span(self, text: String, ctx: &Ctx) -> Span<'static> {
        let theme = ctx.theme;
        let color = if self.code || self.link {
            theme.secondary
        } else if self.html {
            theme.muted
        } else {
            ctx.base
        };
        let mut out = if self.strong {
            // Bold in every theme: a heading or `**strong**` run has to read
            // as one on a colour terminal too, not only under `NO_COLOR`.
            let mut strong = span_strong(text, color, theme);
            strong.style = strong.style.add_modifier(Modifier::BOLD);
            strong
        } else {
            span(text, color)
        };
        if self.italic {
            out.style = out.style.add_modifier(Modifier::ITALIC);
        }
        if self.strike {
            out.style = out.style.add_modifier(Modifier::CROSSED_OUT);
        }
        out
    }
}

#[derive(Debug)]
enum Inline {
    Text(String, Face),
    /// A hard break: the paragraph continues on a new row.
    Break,
}

#[derive(Debug, Default)]
struct Item {
    blocks: Vec<Block>,
    /// `- [x]` / `- [ ]`, when the list is a task list.
    task: Option<bool>,
}

#[derive(Debug)]
enum Block {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Quote(Vec<Block>),
    List {
        /// The first number of an ordered list; `None` for bullets.
        start: Option<u64>,
        items: Vec<Item>,
    },
    Rule,
    Html(String),
    /// Cells are plain text: a table is read as a grid, not as prose.
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

// ---------------------------------------------------------------------------
// Parsing: pulldown-cmark events into the block tree
// ---------------------------------------------------------------------------

/// A block that holds other blocks. `Root` is the document itself.
#[derive(Debug)]
enum Container {
    Root(Vec<Block>),
    Quote(Vec<Block>),
    List {
        start: Option<u64>,
        items: Vec<Item>,
    },
    Item(Item),
}

/// The block currently taking text. Leaves never nest, so one is enough.
#[derive(Debug)]
enum Leaf {
    Paragraph(Vec<Inline>),
    Heading {
        level: u8,
        inlines: Vec<Inline>,
    },
    Code {
        lang: Option<String>,
        text: String,
    },
    Html(String),
    Table {
        header: Vec<String>,
        rows: Vec<Vec<String>>,
        row: Vec<String>,
        cell: Vec<Inline>,
    },
}

/// An open link or image: its text is collected until it closes.
#[derive(Debug)]
struct Capture {
    url: String,
    text: String,
}

#[derive(Debug)]
struct Builder {
    containers: Vec<Container>,
    leaf: Option<Leaf>,
    faces: Vec<Face>,
    captures: Vec<Capture>,
    /// Containers opened past [`MAX_DEPTH`], counted so their ends are skipped
    /// too. Their content lands in the deepest container that was kept.
    skipped: usize,
}

fn parse(source: &str) -> Vec<Block> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut builder = Builder {
        containers: vec![Container::Root(Vec::new())],
        leaf: None,
        faces: Vec::new(),
        captures: Vec::new(),
        skipped: 0,
    };
    for event in Parser::new_ext(source, options) {
        builder.event(event);
    }
    builder.finish()
}

impl Builder {
    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => {
                let face = self.face();
                self.text(&text, face);
            }
            Event::Code(code) => {
                let face = Face {
                    code: true,
                    ..self.face()
                };
                self.text(&code, face);
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let face = Face {
                    html: true,
                    ..self.face()
                };
                self.text(&html, face);
            }
            Event::SoftBreak => {
                let face = self.face();
                self.text(" ", face);
            }
            Event::HardBreak => match self.captures.last_mut() {
                Some(capture) => capture.text.push(' '),
                None => self.push_inline(Inline::Break),
            },
            Event::Rule => {
                self.close_leaf();
                self.push_block(Block::Rule);
            }
            Event::TaskListMarker(checked) => {
                if let Some(Container::Item(item)) = self.containers.last_mut() {
                    item.task = Some(checked);
                }
            }
            // Footnotes and math are not enabled; nothing else carries text.
            Event::FootnoteReference(_) | Event::InlineMath(_) | Event::DisplayMath(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => self.open_leaf(Leaf::Paragraph(Vec::new())),
            Tag::Heading { level, .. } => self.open_leaf(Leaf::Heading {
                level: level as u8,
                inlines: Vec::new(),
            }),
            Tag::CodeBlock(kind) => {
                let lang = match kind {
                    CodeBlockKind::Fenced(info) => fence_language(&info),
                    CodeBlockKind::Indented => None,
                };
                self.open_leaf(Leaf::Code {
                    lang,
                    text: String::new(),
                });
            }
            Tag::HtmlBlock => self.open_leaf(Leaf::Html(String::new())),
            Tag::BlockQuote(_) => self.open_container(Container::Quote(Vec::new())),
            Tag::List(start) => self.open_container(Container::List {
                start,
                items: Vec::new(),
            }),
            Tag::Item => self.open_container(Container::Item(Item::default())),
            Tag::Table(_) => self.open_leaf(Leaf::Table {
                header: Vec::new(),
                rows: Vec::new(),
                row: Vec::new(),
                cell: Vec::new(),
            }),
            Tag::TableHead => {}
            Tag::TableRow => {
                if let Some(Leaf::Table { row, .. }) = &mut self.leaf {
                    row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(Leaf::Table { cell, .. }) = &mut self.leaf {
                    cell.clear();
                }
            }
            Tag::Emphasis => self.push_face(|face| face.italic = true),
            Tag::Strong => self.push_face(|face| face.strong = true),
            Tag::Strikethrough => self.push_face(|face| face.strike = true),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.captures.push(Capture {
                    url: dest_url.to_string(),
                    text: String::new(),
                });
            }
            // Not enabled; their inner blocks still flow into the current
            // container, which is the best reading of them anyway.
            Tag::FootnoteDefinition(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::MetadataBlock(_) => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::Table => self.close_leaf(),
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item => self.close_container(),
            TagEnd::TableHead => {
                if let Some(Leaf::Table { header, row, .. }) = &mut self.leaf {
                    *header = std::mem::take(row);
                }
            }
            TagEnd::TableRow => {
                if let Some(Leaf::Table { rows, row, .. }) = &mut self.leaf {
                    rows.push(std::mem::take(row));
                }
            }
            TagEnd::TableCell => {
                if let Some(Leaf::Table { row, cell, .. }) = &mut self.leaf {
                    row.push(plain_text(&std::mem::take(cell)));
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.faces.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                let Some(capture) = self.captures.pop() else {
                    return;
                };
                let shown = link_text(&capture.text, &capture.url);
                match self.captures.last_mut() {
                    // An image inside a link: it reads as part of the link.
                    Some(outer) => outer.text.push_str(&shown),
                    None => {
                        let face = Face {
                            link: true,
                            ..self.face()
                        };
                        self.push_inline(Inline::Text(shown, face));
                    }
                }
            }
            TagEnd::FootnoteDefinition
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::MetadataBlock(_) => {}
        }
    }

    fn finish(mut self) -> Vec<Block> {
        self.close_leaf();
        self.skipped = 0;
        while self.containers.len() > 1 {
            self.close_container();
        }
        match self.containers.pop() {
            Some(Container::Root(blocks)) => blocks,
            _ => Vec::new(),
        }
    }

    fn face(&self) -> Face {
        self.faces.last().copied().unwrap_or_default()
    }

    fn push_face(&mut self, change: impl FnOnce(&mut Face)) {
        let mut face = self.face();
        change(&mut face);
        self.faces.push(face);
    }

    /// Text goes to the open link if there is one, raw into a code or HTML
    /// block, and otherwise into the paragraph — opening one if a tight list
    /// item or a table cell is taking text with no paragraph of its own.
    fn text(&mut self, text: &str, face: Face) {
        if let Some(capture) = self.captures.last_mut() {
            capture.text.push_str(text);
            return;
        }
        match &mut self.leaf {
            Some(Leaf::Code { text: raw, .. }) | Some(Leaf::Html(raw)) => raw.push_str(text),
            _ => self.push_inline(Inline::Text(text.to_string(), face)),
        }
    }

    fn push_inline(&mut self, inline: Inline) {
        match &mut self.leaf {
            Some(Leaf::Paragraph(inlines)) | Some(Leaf::Heading { inlines, .. }) => {
                inlines.push(inline);
            }
            Some(Leaf::Table { cell, .. }) => cell.push(inline),
            // A break cannot land in a code or HTML block; text never gets
            // here for them (see `text`).
            Some(Leaf::Code { .. }) | Some(Leaf::Html(_)) => {}
            None => self.leaf = Some(Leaf::Paragraph(vec![inline])),
        }
    }

    fn open_leaf(&mut self, leaf: Leaf) {
        self.close_leaf();
        self.leaf = Some(leaf);
    }

    fn close_leaf(&mut self) {
        let Some(leaf) = self.leaf.take() else {
            return;
        };
        let block = match leaf {
            Leaf::Paragraph(inlines) => Block::Paragraph(inlines),
            Leaf::Heading { level, inlines } => Block::Heading { level, inlines },
            Leaf::Code { lang, text } => Block::Code { lang, text },
            Leaf::Html(text) => Block::Html(text),
            Leaf::Table { header, rows, .. } => Block::Table { header, rows },
        };
        self.push_block(block);
    }

    fn open_container(&mut self, container: Container) {
        self.close_leaf();
        if self.skipped > 0 || self.containers.len() >= MAX_DEPTH {
            self.skipped += 1;
            return;
        }
        self.containers.push(container);
    }

    fn close_container(&mut self) {
        self.close_leaf();
        if self.skipped > 0 {
            self.skipped -= 1;
            return;
        }
        if self.containers.len() <= 1 {
            // The root is never closed by an event.
            return;
        }
        let Some(container) = self.containers.pop() else {
            return;
        };
        match container {
            Container::Root(blocks) => self.blocks_mut().extend(blocks),
            Container::Quote(blocks) => self.push_block(Block::Quote(blocks)),
            Container::List { start, items } => self.push_block(Block::List { start, items }),
            Container::Item(item) => match self.containers.last_mut() {
                Some(Container::List { items, .. }) => items.push(item),
                // An item with no list around it: keep its content.
                _ => self.blocks_mut().extend(item.blocks),
            },
        }
    }

    fn push_block(&mut self, block: Block) {
        self.blocks_mut().push(block);
    }

    /// Where the next block goes. A list only holds items, so a block that
    /// lands on one directly (it should not) opens an item for it.
    fn blocks_mut(&mut self) -> &mut Vec<Block> {
        if self.containers.is_empty() {
            self.containers.push(Container::Root(Vec::new()));
        }
        let top = self.containers.len() - 1;
        match &mut self.containers[top] {
            Container::Root(blocks) | Container::Quote(blocks) => blocks,
            Container::Item(item) => &mut item.blocks,
            Container::List { items, .. } => {
                if items.is_empty() {
                    items.push(Item::default());
                }
                let last = items.len() - 1;
                &mut items[last].blocks
            }
        }
    }
}

/// The language a fence names: the first word of its info string, so
/// `rust,ignore` and `js title=x` still read as `rust` and `js`.
fn fence_language(info: &str) -> Option<String> {
    info.split(|c: char| c.is_whitespace() || c == ',')
        .find(|word| !word.is_empty())
        .map(str::to_string)
}

/// `text (url)`, unless the text *is* the url — an autolink, or a mailto
/// written out — in which case once is enough.
fn link_text(text: &str, url: &str) -> String {
    let text = text.trim();
    let bare = url.strip_prefix("mailto:").unwrap_or(url);
    if text.is_empty() {
        url.to_string()
    } else if url.is_empty() || text == url || text == bare {
        text.to_string()
    } else {
        format!("{text} ({url})")
    }
}

/// Inline content as one line of plain text, for table cells.
fn plain_text(inlines: &[Inline]) -> String {
    let mut raw = String::new();
    for inline in inlines {
        match inline {
            Inline::Text(text, _) => raw.push_str(text),
            Inline::Break => raw.push(' '),
        }
    }
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Rendering: the block tree into rows
// ---------------------------------------------------------------------------

/// What every row inherits from its containers.
#[derive(Clone, Copy)]
struct Ctx<'a> {
    theme: &'a Theme,
    /// The colour of ordinary text: `text`, or `muted` inside a block quote.
    base: Color,
}

impl Ctx<'_> {
    fn muted(self) -> Self {
        Ctx {
            theme: self.theme,
            base: self.theme.muted,
        }
    }
}

/// The width prose wraps to. Below [`NARROW`] nothing wraps: each block is
/// laid out as one row and clipped by the caller.
fn measure(width: u16) -> u16 {
    if width < NARROW {
        u16::MAX
    } else {
        width
    }
}

fn is_blank(line: &Line<'_>) -> bool {
    line.spans.iter().all(|span| span.content.trim().is_empty())
}

fn prefixed(mut prefix: Vec<Span<'static>>, line: Line<'static>) -> Line<'static> {
    prefix.extend(line.spans);
    Line::from(prefix)
}

/// A sequence of blocks as rows. `spaced` puts one blank row between blocks
/// (§3); a list keeps its items tight. A block that renders to nothing —
/// an empty quote, a bare fence while it streams — leaves no gap behind.
fn render_blocks(blocks: &[Block], width: u16, ctx: &Ctx, spaced: bool) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for block in blocks {
        let rows = render_block(block, width, ctx);
        if rows.is_empty() {
            continue;
        }
        if spaced && !out.is_empty() {
            out.push(blank());
        }
        out.extend(rows);
    }
    out
}

fn render_block(block: &Block, width: u16, ctx: &Ctx) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    match block {
        Block::Paragraph(inlines) => wrap_inlines(inlines, measure(width), false, ctx),
        Block::Heading { level, inlines } => {
            let mut rows = wrap_inlines(inlines, measure(width), true, ctx);
            if !rows.is_empty() && *level <= 2 {
                rows.push(hair_rule(width, theme, ""));
            }
            rows
        }
        Block::Code { lang, text } => code_rows(lang.as_deref(), text, width, ctx),
        Block::Quote(children) => {
            render_blocks(children, width.saturating_sub(2), &ctx.muted(), true)
                .into_iter()
                .map(|line| prefixed(vec![span("│ ", theme.muted)], line))
                .collect()
        }
        Block::List { start, items } => list_rows(*start, items, width, ctx),
        Block::Rule => vec![hair_rule(width, theme, "")],
        Block::Html(text) => text
            .lines()
            .flat_map(|line| wrap(line, measure(width)))
            .filter(|row| !row.trim().is_empty())
            .map(|row| Line::from(vec![span(row, theme.muted)]))
            .collect(),
        Block::Table { header, rows } => table_rows(header, rows, width, ctx),
    }
}

/// Paragraph and heading text: hard breaks split the run, everything else
/// wraps. `strong` is the heading's weight, laid over every run.
fn wrap_inlines(inlines: &[Inline], width: u16, strong: bool, ctx: &Ctx) -> Vec<Line<'static>> {
    let mut rows = Vec::new();
    let mut run: Vec<(&str, Face)> = Vec::new();
    for inline in inlines {
        match inline {
            Inline::Text(text, face) => run.push((
                text.as_str(),
                Face {
                    strong: face.strong || strong,
                    ..*face
                },
            )),
            Inline::Break => {
                rows.extend(wrap_run(&run, width, ctx));
                run.clear();
            }
        }
    }
    rows.extend(wrap_run(&run, width, ctx));
    rows
}

/// Wrap one run of faced text to `width`, keeping each character's face.
///
/// `ui::wrap` decides the breaks on the plain text, so markdown prose breaks
/// exactly where plain prose does; the faces are then laid back over its rows
/// character by character. `wrap` collapses whitespace to single spaces and
/// drops the space it breaks at, so the face stream is normalised the same
/// way and one pending space is skipped at each row boundary. A run that is
/// only whitespace has no row.
fn wrap_run(run: &[(&str, Face)], width: u16, ctx: &Ctx) -> Vec<Line<'static>> {
    let mut plain = String::new();
    let mut faces: Vec<(char, Face)> = Vec::new();
    for (text, face) in run {
        plain.push_str(text);
        for ch in text.chars() {
            if ch.is_whitespace() {
                if faces.last().is_some_and(|(last, _)| *last != ' ') {
                    faces.push((' ', *face));
                }
            } else {
                faces.push((ch, *face));
            }
        }
    }
    if faces.is_empty() {
        return Vec::new();
    }

    let mut next = 0usize;
    let mut rows = Vec::new();
    for row in wrap(&plain, width) {
        if faces.get(next).is_some_and(|(ch, _)| *ch == ' ') {
            next += 1;
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut text = String::new();
        let mut current = Face::default();
        for ch in row.chars() {
            let face = match faces.get(next) {
                Some((expected, face)) if *expected == ch => {
                    next += 1;
                    *face
                }
                // Out of step with `wrap`, which cannot happen while it keeps
                // every non-blank character; if it ever does, the rest of the
                // paragraph is plain rather than wrong.
                _ => {
                    next = faces.len();
                    Face::default()
                }
            };
            if face != current && !text.is_empty() {
                spans.push(current.span(std::mem::take(&mut text), ctx));
            }
            current = face;
            text.push(ch);
        }
        if !text.is_empty() {
            spans.push(current.span(text, ctx));
        }
        rows.push(Line::from(spans));
    }
    rows
}

/// Code sits two columns in behind a `│`, never wrapped: a row past the
/// width is clipped, and tabs are four spaces. A fence that names its
/// language gets that name as a muted first row.
fn code_rows(lang: Option<&str>, text: &str, width: u16, ctx: &Ctx) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let room = width.saturating_sub(CODE_INSET);
    let row = |content: String, color: Color| {
        indent(
            2,
            vec![
                span("│ ", theme.border),
                span(clip_ellipsis(&content, room), color),
            ],
        )
    };
    let mut out = Vec::new();
    if let Some(lang) = lang {
        out.push(row(lang.to_string(), theme.muted));
    }
    if text.is_empty() {
        return out;
    }
    let body = text.strip_suffix('\n').unwrap_or(text);
    out.extend(
        body.split('\n')
            .map(|line| row(line.trim_end_matches('\r').replace('\t', "    "), ctx.base)),
    );
    out
}

/// `·` for bullets in border, `1.` for numbers in muted, then the item's
/// text hanging under itself. Numbers right-align to the widest label in the
/// list so every item's text shares a column; nested content, lists
/// included, starts in that column — two columns in for a bullet.
fn list_rows(start: Option<u64>, items: &[Item], width: u16, ctx: &Ctx) -> Vec<Line<'static>> {
    let theme = ctx.theme;
    let label_width = start.map_or(0, |first| {
        (0..items.len() as u64)
            .map(|offset| format!("{}.", first.saturating_add(offset)).len())
            .max()
            .unwrap_or(0)
    });
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut marker = match start {
            Some(first) => vec![span(
                format!(
                    "{:>width$} ",
                    format!("{}.", first.saturating_add(index as u64)),
                    width = label_width
                ),
                theme.muted,
            )],
            None => vec![span(format!("{} ", glyph::TICK), theme.border)],
        };
        match item.task {
            Some(true) => marker.push(span(format!("{} ", glyph::DONE), theme.success)),
            Some(false) => marker.push(span(format!("{} ", glyph::QUEUED), theme.border)),
            None => {}
        }
        let hang = run_width(&marker);
        let body = render_blocks(&item.blocks, width.saturating_sub(hang), ctx, false);
        if body.is_empty() {
            // A bare marker — `* ` as it streams in — with no space hanging
            // off it.
            if let Some(last) = marker.last_mut() {
                last.content = last.content.trim_end().to_string().into();
            }
            out.push(Line::from(marker));
            continue;
        }
        for (row, line) in body.into_iter().enumerate() {
            out.push(if row == 0 {
                prefixed(marker.clone(), line)
            } else {
                indent(hang, line.spans)
            });
        }
    }
    out
}

/// Plain cells, columns padded to their widest cell and joined by two
/// spaces, the header strong. No rules: the alignment is the grid. A row
/// wider than the width is clipped, never wrapped.
fn table_rows(
    header: &[String],
    rows: &[Vec<String>],
    width: u16,
    ctx: &Ctx,
) -> Vec<Line<'static>> {
    let columns = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    if columns == 0 {
        return Vec::new();
    }
    let mut widths = vec![0usize; columns];
    for row in std::iter::once(header).chain(rows.iter().map(Vec::as_slice)) {
        for (column, cell) in row.iter().enumerate() {
            widths[column] = widths[column].max(UnicodeWidthStr::width(cell.as_str()));
        }
    }
    let join = |row: &[String]| {
        let mut out = String::new();
        for (column, column_width) in widths.iter().enumerate() {
            let cell = row.get(column).map_or("", String::as_str);
            if column > 0 {
                out.push_str("  ");
            }
            out.push_str(cell);
            if column + 1 < columns {
                let fill = column_width.saturating_sub(UnicodeWidthStr::width(cell));
                out.push_str(&" ".repeat(fill));
            }
        }
        clip_ellipsis(&out, width)
    };
    let mut out = Vec::new();
    if !header.is_empty() {
        out.push(Line::from(vec![span_strong(
            join(header),
            ctx.base,
            ctx.theme,
        )]));
    }
    out.extend(
        rows.iter()
            .map(|row| Line::from(vec![span(join(row), ctx.base)])),
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::ColorDepth;
    use crate::davinci::ui::MEASURE;

    fn theme() -> Theme {
        Theme::da_vinci(ColorDepth::TrueColor, false)
    }

    fn render(source: &str, width: u16) -> Vec<Line<'static>> {
        lines(&theme(), source, width)
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    fn texts(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(text).collect()
    }

    fn width_of(line: &Line<'_>) -> u16 {
        run_width(&line.spans)
    }

    fn span_with<'a>(line: &'a Line<'a>, content: &str) -> &'a Span<'a> {
        line.spans
            .iter()
            .find(|span| span.content.as_ref() == content)
            .unwrap_or_else(|| panic!("no span {content:?} in {:?}", text(line)))
    }

    #[test]
    fn paragraph_wraps_at_the_width() {
        let rows = render(
            "The quick brown fox jumps over the lazy dog and keeps on running.",
            20,
        );
        assert!(rows.len() > 1, "{:?}", texts(&rows));
        for row in &rows {
            assert!(width_of(row) <= 20, "{:?} is wider than 20", text(row));
            assert!(!is_blank(row));
            for span in &row.spans {
                assert_eq!(span.style.fg, Some(theme().text));
            }
        }
        assert_eq!(text(&rows[0]), "The quick brown fox");
        assert_eq!(
            texts(&rows).join(" "),
            "The quick brown fox jumps over the lazy dog and keeps on running."
        );
    }

    #[test]
    fn heading_drops_the_hash_and_level_one_gets_a_rule() {
        let rows = render("# Title\n\nBody text.", MEASURE);
        let shown = texts(&rows);
        assert_eq!(shown.len(), 4, "{shown:?}");
        assert_eq!(shown[0], "Title");
        assert!(shown[1].contains('─'), "{:?} is not a rule", shown[1]);
        assert!(width_of(&rows[1]) <= MEASURE);
        assert!(shown[2].is_empty());
        assert_eq!(shown[3], "Body text.");
        assert!(shown.iter().all(|row| !row.contains('#')));
        let title = span_with(&rows[0], "Title");
        assert_eq!(title.style.fg, Some(theme().text));
        assert!(title.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn level_two_gets_a_rule_and_deeper_levels_do_not() {
        let two = texts(&render("## Second", MEASURE));
        assert_eq!(two.len(), 2, "{two:?}");
        assert_eq!(two[0], "Second");
        assert!(two[1].contains('─'));

        let three = texts(&render("### Third\n\nafter", MEASURE));
        assert_eq!(three, vec!["Third", "", "after"]);
    }

    #[test]
    fn bullet_list_marks_items_and_hangs_continuations() {
        let rows = render(
            "- alpha beta gamma delta epsilon\n- short\n  - nested item here\n",
            20,
        );
        let shown = texts(&rows);
        assert_eq!(shown[0], "· alpha beta gamma");
        assert_eq!(shown[1], "  delta epsilon");
        assert_eq!(shown[2], "· short");
        assert_eq!(shown[3], "  · nested item here");
        assert_eq!(shown.len(), 4, "{shown:?}");
        for row in &rows {
            assert!(width_of(row) <= 20, "{:?}", text(row));
        }
        let marker = span_with(&rows[0], "· ");
        assert_eq!(marker.style.fg, Some(theme().border));
        assert!(!shown.iter().any(|row| row.contains("- ")));
    }

    #[test]
    fn ordered_list_counts_from_its_start() {
        let rows = render("3. three\n4. four\n5. five", MEASURE);
        assert_eq!(texts(&rows), vec!["3. three", "4. four", "5. five"]);
        let label = span_with(&rows[0], "3. ");
        assert_eq!(label.style.fg, Some(theme().muted));

        let wide = texts(&render("9. nine\n10. ten\n11. eleven", MEASURE));
        assert_eq!(wide, vec![" 9. nine", "10. ten", "11. eleven"]);
    }

    #[test]
    fn task_items_carry_state_glyphs() {
        let rows = render("- [x] done\n- [ ] later", MEASURE);
        assert_eq!(texts(&rows), vec!["· ✓ done", "· ○ later"]);
        assert_eq!(span_with(&rows[0], "✓ ").style.fg, Some(theme().success));
        assert_eq!(span_with(&rows[1], "○ ").style.fg, Some(theme().border));
    }

    #[test]
    fn fenced_code_names_its_language_and_clips_long_rows() {
        let long = "x".repeat(40);
        let source = format!("```rust\nfn main() {{}}\n\tindented\n{long}\n```\n");
        let rows = render(&source, 24);
        let shown = texts(&rows);
        assert_eq!(shown[0], "  │ rust");
        assert_eq!(shown[1], "  │ fn main() {}");
        assert_eq!(shown[2], "  │     indented");
        assert!(shown[3].ends_with('…'), "{:?}", shown[3]);
        assert_eq!(shown.len(), 4, "{shown:?}");
        for row in &rows {
            assert!(width_of(row) <= 24, "{:?}", text(row));
        }
        assert!(!shown.iter().any(|row| row.contains("```")));
        assert_eq!(span_with(&rows[0], "rust").style.fg, Some(theme().muted));
        assert_eq!(span_with(&rows[1], "│ ").style.fg, Some(theme().border));
        assert_eq!(
            span_with(&rows[1], "fn main() {}").style.fg,
            Some(theme().text)
        );
    }

    #[test]
    fn indented_code_is_code_too() {
        let rows = render("para\n\n    let x = 1;\n    let y = 2;\n", MEASURE);
        assert_eq!(
            texts(&rows),
            vec!["para", "", "  │ let x = 1;", "  │ let y = 2;"]
        );
    }

    #[test]
    fn inline_code_is_verdigris() {
        let rows = render("run `cargo test` now", MEASURE);
        assert_eq!(texts(&rows), vec!["run cargo test now"]);
        let code = span_with(&rows[0], "cargo test");
        assert_eq!(code.style.fg, Some(theme().secondary));
        assert_eq!(span_with(&rows[0], "run ").style.fg, Some(theme().text));
    }

    #[test]
    fn unclosed_fence_still_renders_as_code() {
        let rows = render("Look:\n\n```\nlet x = 1;\nlet y", MEASURE);
        assert_eq!(
            texts(&rows),
            vec!["Look:", "", "  │ let x = 1;", "  │ let y"]
        );
        assert_eq!(
            texts(&render("```py\nprint(1)", MEASURE)),
            vec!["  │ py", "  │ print(1)"]
        );
    }

    #[test]
    fn blocks_are_separated_by_exactly_one_blank_row() {
        assert_eq!(
            texts(&render("one\n\nthree", MEASURE)),
            vec!["one", "", "three"]
        );
        assert_eq!(
            texts(&render("\n\n\none\n\n\n\ntwo\n\n\n", MEASURE)),
            vec!["one", "", "two"]
        );
        assert_eq!(texts(&render("only", MEASURE)), vec!["only"]);
        // A list is one block: no blank rows between its items, even loose.
        assert_eq!(
            texts(&render("- a\n\n- b\n\nafter", MEASURE)),
            vec!["· a", "· b", "", "after"]
        );
    }

    #[test]
    fn emphasis_is_italic_and_strong_is_the_theme_emphasis() {
        let rows = render("*soft* and **hard** and ***both***", MEASURE);
        assert_eq!(texts(&rows), vec!["soft and hard and both"]);
        let soft = span_with(&rows[0], "soft");
        assert!(soft.style.add_modifier.contains(Modifier::ITALIC));
        assert_eq!(soft.style.fg, Some(theme().text));
        let hard = span_with(&rows[0], "hard");
        assert!(hard.style.add_modifier.contains(Modifier::BOLD));
        let both = span_with(&rows[0], "both");
        assert!(both.style.add_modifier.contains(Modifier::ITALIC));
        assert!(both.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn styles_survive_a_wrap() {
        let rows = render("plain **bold words that wrap** plain", 16);
        let shown = texts(&rows);
        assert_eq!(shown, vec!["plain bold words", "that wrap plain"]);
        assert_eq!(
            span_with(&rows[0], "plain ").style.add_modifier,
            Modifier::empty()
        );
        assert!(rows[1].spans[0].content.starts_with("that wrap"));
        let no_color = Theme::da_vinci(ColorDepth::TrueColor, true);
        let rows = lines(&no_color, "plain **bold words that wrap** plain", 16);
        assert!(span_with(&rows[0], "bold words")
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(span_with(&rows[1], "that wrap")
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(!span_with(&rows[1], " plain")
            .style
            .add_modifier
            .contains(Modifier::BOLD));
    }

    #[test]
    fn links_show_their_url_once() {
        let rows = render(
            "see [the docs](https://example.com/docs) or <https://example.com>",
            MEASURE,
        );
        assert_eq!(
            texts(&rows),
            vec!["see the docs (https://example.com/docs) or https://example.com"]
        );
        let link = span_with(&rows[0], "the docs (https://example.com/docs)");
        assert_eq!(link.style.fg, Some(theme().secondary));
        let bare = span_with(&rows[0], "https://example.com");
        assert_eq!(bare.style.fg, Some(theme().secondary));
        assert_eq!(
            texts(&render("<me@example.com>", MEASURE)),
            vec!["me@example.com"]
        );
    }

    #[test]
    fn block_quote_is_muted_behind_a_rule() {
        let rows = render("> quoted words\n> more\n>\n> second paragraph", MEASURE);
        assert_eq!(
            texts(&rows),
            vec!["│ quoted words more", "│ ", "│ second paragraph"]
        );
        for span in &rows[0].spans {
            assert_eq!(span.style.fg, Some(theme().muted), "{:?}", span.content);
        }
    }

    #[test]
    fn thematic_break_is_a_hair_rule() {
        let rows = render("above\n\n---\n\nbelow", MEASURE);
        let shown = texts(&rows);
        assert_eq!(shown.len(), 5, "{shown:?}");
        assert_eq!(text(&rows[2]), text(&hair_rule(MEASURE, &theme(), "")));
        assert!(width_of(&rows[2]) <= MEASURE);
    }

    #[test]
    fn tables_pad_columns_and_embolden_the_header() {
        let rows = render(
            "| name | tokens |\n|---|---|\n| manus | 12 |\n| memoria | 1240 |",
            MEASURE,
        );
        assert_eq!(
            texts(&rows),
            vec!["name     tokens", "manus    12", "memoria  1240"]
        );
        assert_eq!(rows[0].spans[0].style.add_modifier, theme().emphasis);
        assert!(!texts(&rows).iter().any(|row| row.contains('|')));
        let no_color = Theme::da_vinci(ColorDepth::TrueColor, true);
        let rows = lines(&no_color, "| a | b |\n|---|---|\n| 1 | 2 |", MEASURE);
        assert!(rows[0].spans[0].style.add_modifier.contains(Modifier::BOLD));
        assert!(!rows[1].spans[0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn html_is_muted_plain_text() {
        let rows = render("<div>\n<b>x</b>\n</div>\n\nafter <br> text", MEASURE);
        assert_eq!(
            texts(&rows),
            vec!["<div>", "<b>x</b>", "</div>", "", "after <br> text"]
        );
        assert_eq!(rows[0].spans[0].style.fg, Some(theme().muted));
        assert_eq!(span_with(&rows[4], "<br>").style.fg, Some(theme().muted));
        assert_eq!(span_with(&rows[4], "after ").style.fg, Some(theme().text));
    }

    #[test]
    fn soft_breaks_join_and_hard_breaks_split() {
        assert_eq!(texts(&render("one\ntwo", MEASURE)), vec!["one two"]);
        assert_eq!(texts(&render("one  \ntwo", MEASURE)), vec!["one", "two"]);
        assert_eq!(texts(&render("one\\\ntwo", MEASURE)), vec!["one", "two"]);
    }

    #[test]
    fn narrow_widths_clip_instead_of_wrapping() {
        let rows = render("hello wide world", 5);
        assert_eq!(texts(&rows), vec!["hell…"]);
        let rows = render("- item text here", 7);
        assert_eq!(texts(&rows), vec!["· item…"]);
        assert_eq!(
            texts(&render("hello wide world", 8)),
            vec!["hello", "wide", "world"]
        );
    }

    #[test]
    fn nested_blocks_keep_their_prefixes() {
        let rows = render("- item\n\n  ```\n  code\n  ```\n\n  > quote\n", MEASURE);
        assert_eq!(texts(&rows), vec!["· item", "    │ code", "  │ quote"]);
        let rows = render("> - one\n> - two", MEASURE);
        assert_eq!(texts(&rows), vec!["│ · one", "│ · two"]);
    }

    #[test]
    fn odd_inputs_never_panic() {
        let long = "y".repeat(5_000);
        let inputs: Vec<&str> = vec![
            "",
            "   \n\t\n  ",
            "```",
            "* ",
            "# ",
            "[x](",
            "> ",
            &long,
            "**unclosed",
            "- [ ",
            "|a|\n|-",
            "````\n```\n",
            "1. \n2. ",
            "> > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > > deep",
            "![](",
            "<",
            "\u{0}",
        ];
        let no_color = Theme::da_vinci(ColorDepth::TrueColor, true);
        for input in inputs {
            for width in [0, 1, 2, 7, 8, 20, MEASURE, 200] {
                for theme in [theme(), no_color] {
                    let rows = lines(&theme, input, width);
                    if width == 0 {
                        assert!(rows.is_empty());
                        continue;
                    }
                    for row in &rows {
                        assert!(
                            width_of(row) <= width,
                            "{:?} overflows {width} for {:?}",
                            text(row),
                            input.chars().take(30).collect::<String>()
                        );
                    }
                    assert!(rows.first().map_or(true, |row| !is_blank(row)));
                    assert!(rows.last().map_or(true, |row| !is_blank(row)));
                }
            }
        }
        assert!(render("", MEASURE).is_empty());
        assert!(render("   \n\t\n  ", MEASURE).is_empty());
        assert!(render("```", MEASURE).is_empty());
        assert!(render("> ", MEASURE).is_empty());
        assert!(render("# ", MEASURE).is_empty());
        assert_eq!(texts(&render("* ", MEASURE)), vec!["·"]);
        assert_eq!(texts(&render("[x](", MEASURE)), vec!["[x]("]);
        assert_eq!(texts(&render("**unclosed", MEASURE)), vec!["**unclosed"]);
        assert_eq!(
            render(&long, MEASURE).len(),
            5_000_usize.div_ceil(MEASURE as usize)
        );
    }

    #[test]
    fn no_color_theme_marks_headings_bold() {
        let no_color = Theme::da_vinci(ColorDepth::TrueColor, true);
        let rows = lines(&no_color, "# Title\n\nplain **strong**", MEASURE);
        let title = span_with(&rows[0], "Title");
        assert!(title.style.add_modifier.contains(Modifier::BOLD));
        assert!(!span_with(&rows[3], "plain ")
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        assert!(span_with(&rows[3], "strong")
            .style
            .add_modifier
            .contains(Modifier::BOLD));
        // Colour only ever comes from the theme, so the greyscale ramp holds.
        for row in &rows {
            for span in &row.spans {
                if let Some(Color::Rgb(r, g, b)) = span.style.fg {
                    assert!(r == g && g == b, "{:?} is not grey", span.content);
                }
            }
        }
    }
}
