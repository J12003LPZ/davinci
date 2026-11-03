//! The `/theme` picker of the reference shell: a pointer list, a live diff
//! preview and a syntax line. Moving the highlight previews each theme at once;
//! `esc` puts the previous one back. Nothing here writes settings: `enter`
//! returns the chosen setting value for the host to store.
use crate::davinci::{
    app::Flow,
    model::{wrap_index, Choice, Entry, EntryLog, Model, ThemePicker},
    ui::{self, span},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// Every built-in theme DaVinci's shell resolves, as `(setting value, label)`.
/// The labels of the shared themes are the reference's short functional ones,
/// in the reference's order. DaVinci's own Vox stands last, where the
/// reference has its custom-theme entry; there is no such entry here.
pub const ENTRIES: [(&str, &str); 8] = [
    ("auto", "Auto (match terminal)"),
    ("dark", "Dark mode"),
    ("light", "Light mode"),
    ("daltonized", "Dark mode (colorblind-friendly)"),
    ("light-daltonized", "Light mode (colorblind-friendly)"),
    ("ansi", "Dark mode (ANSI colors only)"),
    ("light-ansi", "Light mode (ANSI colors only)"),
    ("vox", "Vox mode"),
];

/// Open the picker on `current` (the stored theme name). Only the reference
/// shell has it; otherwise nothing opens and `false` is returned.
pub fn open(model: &mut Model, current: &str) -> bool {
    if !model.reference {
        return false;
    }
    // A stored name the list lacks (`pi`, a custom theme) highlights Dark but
    // is not Dark: no row carries the check mark.
    let index = ENTRIES
        .iter()
        .position(|entry| entry.0 == current)
        .unwrap_or(1);
    model.theme_picker = Some(ThemePicker {
        index,
        applied: current.to_string(),
        previous: model.theme,
        syntax: true,
    });
    true
}

/// The conversation as drawn behind the open picker: a `/theme` still waiting
/// for its answer is not echoed (the picker is the answer), and the rows it
/// leaves behind go with it. The line itself stays in the conversation, so the
/// result hangs from it once a theme is chosen.
pub fn without_pending_echo(transcript: &mut EntryLog) {
    let waiting = transcript
        .iter()
        .rposition(|entry| matches!(entry, Entry::User(prompt) if prompt.trim() == "/theme"))
        .filter(|&index| {
            transcript[index + 1..]
                .iter()
                .all(|entry| matches!(entry, Entry::Gap | Entry::Agent(_)))
        });
    if let Some(index) = waiting {
        transcript.truncate(index);
        while matches!(transcript.last(), Some(Entry::Gap)) {
            transcript.pop();
        }
    }
}

/// All keys belong to the picker while it is open, so nothing typed reaches
/// the composer and its draft is left as it was.
pub fn handle_key(model: &mut Model, key: KeyEvent) -> Flow {
    let Some(picker) = model.theme_picker.as_mut() else {
        return Flow::Continue;
    };
    // ctrl+c cancels like esc; it is not an interrupt the host counts towards
    // quitting.
    let ctrl_c = matches!(key.code, KeyCode::Char(c) if c.eq_ignore_ascii_case(&'c'))
        && key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        _ if ctrl_c || key.code == KeyCode::Esc => {
            let previous = picker.previous;
            model.theme = previous;
            model.theme_picker = None;
        }
        KeyCode::Enter => {
            let name = ENTRIES[picker.index].0.to_string();
            // The preview shows only if the highlight moved; a theme that
            // failed to save can leave another one on screen.
            model.theme = picker.previous.with_name(&name);
            model.theme_picker = None;
            return Flow::Choose(Choice::Theme(name));
        }
        KeyCode::Up | KeyCode::Down => {
            let delta = if key.code == KeyCode::Up { -1 } else { 1 };
            picker.index = wrap_index(picker.index, delta, ENTRIES.len());
            let previous = picker.previous;
            model.theme = previous.with_name(ENTRIES[picker.index].0);
        }
        KeyCode::Char('t') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            picker.syntax = !picker.syntax;
        }
        _ => {}
    }
    Flow::Continue
}

fn dashed(model: &Model, indent: usize) -> Line<'static> {
    let width = usize::from(model.width).saturating_sub(2 * indent);
    Line::from(vec![
        span(" ".repeat(indent), model.theme.text),
        span("╌".repeat(width), model.theme.cc().subtle),
    ])
}

/// The sentence under the picker's title, DaVinci's own. It takes as many cells
/// as the reference's, so a structure comparison finds the bold in the same cells.
pub const HEADING_SENTENCE: &str = "Pick the colour palette that reads best in your terminal";

/// DaVinci's own sample: an unchanged line, a removed one, an added one with
/// the same word changed, and an unchanged one. Only the colours, modifiers and
/// the width of each token come from the reference; none of its text does, so a
/// structure comparison finds every cell in the style it expects.
const KEYWORD: &str = "function";
const NAME: &str = "label";
const SIGNATURE: &str = "() {";
const IDENT: &str = "emitter";
const CALL: &str = "add";
const OPEN: &str = "\"Total: ";
const REMOVED: &str = "empty";
const ADDED: &str = "filled";

pub(super) fn preview(model: &Model, syntax: bool, indent: usize) -> Vec<Line<'static>> {
    let cc = model.theme.diff_palette(syntax);
    let ink = |colour| if syntax { colour } else { cc.diff_text };
    let on = |text: &str, fg, bg| Span::styled(text.to_string(), Style::default().fg(fg).bg(bg));
    // With highlighting the numbers are the plain ink, dimmed; without it they
    // are the quiet ink of the palette.
    let number = |text: &str| {
        let style = if syntax {
            Style::default()
                .fg(cc.diff_text)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default()
                .fg(cc.inactive)
                .add_modifier(cc.inactive_modifier)
        };
        Span::styled(text.to_string(), style)
    };
    let none = ratatui::style::Color::Reset;
    // A row is a three-cell indent, then cells up to a three-cell margin at the
    // right edge; a changed row is tinted from its gutter to that margin.
    let room = usize::from(model.width.saturating_sub(6));
    let row = |spans: Vec<Span<'static>>| {
        let mut all = vec![span(" ".repeat(indent), cc.text)];
        all.extend(spans);
        Line::from(ui::truncate_run(all, model.width))
    };
    let fill = |used: usize| " ".repeat(room.saturating_sub(used));
    let width = |parts: &[&str]| {
        parts
            .iter()
            .map(|part| UnicodeWidthStr::width(*part))
            .sum::<usize>()
    };
    let gutter = 4;
    let before = format!("  {IDENT}.{CALL}({OPEN}");
    let removed_tail = "!\");";
    let removed = {
        let rest = fill(gutter + width(&[&before, REMOVED, removed_tail]));
        row(vec![
            on(
                " 2 -",
                if syntax { cc.diff_del } else { cc.diff_text },
                cc.diff_del_bg,
            ),
            on(&before, cc.diff_text, cc.diff_del_bg),
            on(REMOVED, cc.diff_text, cc.del_word_bg()),
            on(
                &format!("{removed_tail}{rest}"),
                cc.diff_text,
                cc.diff_del_bg,
            ),
        ])
    };
    let added = {
        let close = "!\"";
        let end = ");";
        let rest = fill(gutter + width(&["  ", IDENT, ".", CALL, "(", OPEN, ADDED, close, end]));
        row(vec![
            on(
                " 2 +",
                if syntax { cc.diff_add } else { cc.diff_text },
                cc.diff_add_bg,
            ),
            on("  ", cc.diff_text, cc.diff_add_bg),
            on(IDENT, ink(cc.text), cc.diff_add_bg),
            on(".", cc.diff_text, cc.diff_add_bg),
            on(CALL, ink(cc.function_ink()), cc.diff_add_bg),
            on("(", cc.diff_text, cc.diff_add_bg),
            on(OPEN, ink(cc.code_string), cc.diff_add_bg),
            on(ADDED, ink(cc.code_string), cc.add_word_bg()),
            on(close, ink(cc.code_string), cc.diff_add_bg),
            on(&format!("{end}{rest}"), cc.diff_text, cc.diff_add_bg),
        ])
    };
    let (first, last) = if syntax {
        (
            row(vec![
                number(" 1 "),
                on(" ", cc.diff_text, none),
                on(KEYWORD, cc.code_keyword, none),
                on(" ", cc.diff_text, none),
                on(NAME, cc.function_ink(), none),
                on(SIGNATURE, cc.diff_text, none),
            ]),
            row(vec![number(" 3 "), on(" }", cc.diff_text, none)]),
        )
    } else {
        (
            row(vec![
                number(" 1  "),
                on(&format!("{KEYWORD} {NAME}{SIGNATURE}"), cc.diff_text, none),
            ]),
            row(vec![number(" 3  "), on("}", cc.diff_text, none)]),
        )
    };
    vec![first, removed, added, last]
}

/// The picker's rows, whole: heading, themes, preview and hint, with blank
/// rows between. See [`fitted`] for a screen too short to hold them.
pub fn lines(model: &Model) -> Vec<Line<'static>> {
    assemble(model, true, true)
}

/// The rows that fit `height`. The preview goes first, whole, then the blank
/// rows, then the heading and the intro; past that the list scrolls around the
/// highlighted theme. The highlighted theme is always among the rows, and the
/// hint is while there is a second row for it.
pub fn fitted(model: &Model, height: usize) -> Vec<Line<'static>> {
    if height == 0 {
        return Vec::new();
    }
    for (with_preview, with_blanks) in [(true, true), (false, true), (false, false)] {
        let rows = assemble(model, with_preview, with_blanks);
        if rows.len() <= height {
            return rows;
        }
    }
    let Some(picker) = &model.theme_picker else {
        return Vec::new();
    };
    let mut head = heading(model);
    let mut list = entries(model, 3, ENTRIES.len());
    let hint = hint(model);
    // The intro line, then the rule, then the title.
    for drop in [2, 0, 0] {
        if head.len() + list.len() < height {
            break;
        }
        head.remove(drop);
    }
    let keep_hint = height >= 2;
    let room = height
        .saturating_sub(head.len() + usize::from(keep_hint))
        .max(1);
    if list.len() > room {
        let start = picker.index.saturating_sub(room / 2).min(list.len() - room);
        list = list.drain(start..start + room).collect();
    }
    let mut rows = head;
    rows.extend(list);
    if keep_hint {
        rows.push(hint);
    }
    clip(model, rows)
}

/// Every row cut to the screen's width.
fn clip(model: &Model, rows: Vec<Line<'static>>) -> Vec<Line<'static>> {
    rows.into_iter()
        .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
        .collect()
}

fn heading(model: &Model) -> Vec<Line<'static>> {
    let theme = &model.theme;
    let cc = theme.cc();
    vec![
        super::chrome::effort_rule(model),
        Line::from(vec![
            span("   ", theme.text),
            Span::styled(
                "Theme",
                Style::default()
                    .fg(cc.permission)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            span("   ", theme.text),
            Span::styled(
                HEADING_SENTENCE,
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
    ]
}

pub(super) fn entries(model: &Model, indent: usize, count: usize) -> Vec<Line<'static>> {
    let Some(picker) = &model.theme_picker else {
        return Vec::new();
    };
    let theme = &model.theme;
    let cc = theme.cc();
    ENTRIES
        .iter()
        .take(count)
        .enumerate()
        .map(|(index, (name, label))| {
            let selected = index == picker.index;
            let ink = if selected { cc.permission } else { theme.text };
            let pointer = if selected { "❯" } else { " " };
            let check = if *name == picker.applied { "✔" } else { " " };
            Line::from(vec![
                span(" ".repeat(indent), ink),
                span(pointer, ink),
                span(" ", ink),
                span(check, cc.success),
                span(" ", ink),
                span(*label, ink),
            ])
        })
        .collect()
}

fn hint(model: &Model) -> Line<'static> {
    let cc = model.theme.cc();
    let quiet = Style::default()
        .fg(cc.inactive)
        .add_modifier(cc.inactive_modifier);
    Line::from(vec![
        span("   ", model.theme.text),
        Span::styled(
            "Enter to select · Esc to cancel",
            quiet.add_modifier(Modifier::ITALIC),
        ),
    ])
}

fn assemble(model: &Model, with_preview: bool, with_blanks: bool) -> Vec<Line<'static>> {
    let Some(picker) = &model.theme_picker else {
        return Vec::new();
    };
    let cc = model.theme.cc();
    let quiet = Style::default()
        .fg(cc.inactive)
        .add_modifier(cc.inactive_modifier);
    let blank = |rows: &mut Vec<Line<'static>>| {
        if with_blanks {
            rows.push(ui::blank());
        }
    };
    let mut head = heading(model).into_iter();
    let mut rows = Vec::new();
    rows.extend(head.next());
    rows.extend(head.next());
    blank(&mut rows);
    rows.extend(head.next());
    blank(&mut rows);
    rows.extend(entries(model, 3, ENTRIES.len()));
    if with_preview {
        rows.push(ui::blank());
        rows.push(dashed(model, 3));
        rows.extend(preview(model, picker.syntax, 3));
        rows.push(dashed(model, 3));
        let status = if picker.syntax {
            " Syntax theme: DaVinci (ctrl+t to disable)"
        } else {
            " Syntax highlighting disabled (ctrl+t to enable)"
        };
        rows.push(Line::from(vec![
            span("   ", model.theme.text),
            Span::styled(status, quiet),
        ]));
    }
    blank(&mut rows);
    rows.push(hint(model));
    clip(model, rows)
}

/// The preview as the first-run steps draw it: a dashed rule, the sample, a
/// dashed rule and the syntax line, all from `indent` cells in.
pub fn sample(model: &Model, syntax: bool, indent: usize) -> Vec<Line<'static>> {
    let cc = model.theme.cc();
    let quiet = Style::default()
        .fg(cc.inactive)
        .add_modifier(cc.inactive_modifier);
    let status = if syntax {
        " Syntax theme: DaVinci (ctrl+t to disable)"
    } else {
        " Syntax highlighting disabled (ctrl+t to enable)"
    };
    let mut rows = vec![dashed(model, indent)];
    rows.extend(preview(model, syntax, indent));
    rows.push(dashed(model, indent));
    rows.push(Line::from(ui::truncate_run(
        vec![
            span(" ".repeat(indent), model.theme.text),
            Span::styled(status, quiet),
        ],
        model.width,
    )));
    rows
}
