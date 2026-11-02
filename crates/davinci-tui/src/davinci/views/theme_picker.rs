//! The `/theme` picker of the reference shell: a pointer list, a live diff
//! preview and a syntax line. Moving the highlight previews each theme at once;
//! `esc` puts the previous one back. Nothing here writes settings: `enter`
//! returns the chosen setting value for the host to store.
use crate::davinci::{
    app::Flow,
    model::{wrap_index, Choice, Model, ThemePicker},
    ui::{self, span},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// Every built-in theme DaVinci's shell resolves, as `(setting value, label)`.
/// The labels of the shared themes are the reference's short functional ones.
/// There is no "Light mode (ANSI colors only)" and no custom-theme entry:
/// the shell has neither, and none is invented here.
pub const ENTRIES: [(&str, &str); 7] = [
    ("auto", "Auto (match terminal)"),
    ("dark", "Dark mode"),
    ("light", "Light mode"),
    ("vox", "Vox mode"),
    ("daltonized", "Dark mode (colorblind-friendly)"),
    ("light-daltonized", "Light mode (colorblind-friendly)"),
    ("ansi", "Dark mode (ANSI colors only)"),
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

fn dashed(model: &Model) -> Line<'static> {
    let width = usize::from(model.width.saturating_sub(6));
    Line::from(vec![
        span("   ", model.theme.text),
        span("╌".repeat(width), model.theme.cc().subtle),
    ])
}

/// DaVinci's own sample: an unchanged line, a removed one, an added one with
/// the same word changed, and an unchanged one. Only the colours and
/// modifiers come from the reference; none of its text does.
const KEYWORD: &str = "fn";
const NAME: &str = "label";
const SIGNATURE: &str = "(n: u32) -> String {";
const BEFORE: &str = "    format!(\"";
const REMOVED: &str = "count";
const ADDED: &str = "total";
const AFTER: &str = ": {n}\")";

fn preview(model: &Model, syntax: bool) -> Vec<Line<'static>> {
    let cc = model.theme.diff_palette(syntax);
    let ink = |colour| if syntax { colour } else { cc.diff_text };
    let on = |text: &str, fg, bg| Span::styled(text.to_string(), Style::default().fg(fg).bg(bg));
    let number = Style::default().fg(cc.inactive).add_modifier(
        cc.inactive_modifier
            | if syntax {
                Modifier::DIM
            } else {
                Modifier::empty()
            },
    );
    let none = ratatui::style::Color::Reset;
    // The gutter is four cells after a three-cell indent; a changed row is
    // tinted from the end of its text to the edge.
    let pad = usize::from(model.width.saturating_sub(7));
    let row = |spans: Vec<Span<'static>>| {
        let mut all = vec![span("   ", cc.text)];
        all.extend(spans);
        Line::from(ui::truncate_run(all, model.width))
    };
    let changed = |gutter: &str, sign: ratatui::style::Color, bg, word_bg, text, word: &str| {
        let used = [BEFORE, word, AFTER]
            .iter()
            .map(|part| UnicodeWidthStr::width(*part))
            .sum::<usize>();
        let fill = " ".repeat(pad.saturating_sub(used));
        row(vec![
            on(gutter, sign, bg),
            on(BEFORE, text, bg),
            on(word, text, word_bg),
            on(AFTER, text, bg),
            on(&fill, cc.diff_text, bg),
        ])
    };
    vec![
        row(vec![
            Span::styled(" 1 ", number),
            on(KEYWORD, ink(cc.code_keyword), none),
            on(" ", cc.diff_text, none),
            on(NAME, ink(cc.function_ink()), none),
            on(SIGNATURE, cc.diff_text, none),
        ]),
        changed(
            " 2 -",
            cc.diff_del,
            cc.diff_del_bg,
            cc.del_word_bg(),
            cc.diff_text,
            REMOVED,
        ),
        changed(
            " 2 +",
            cc.diff_add,
            cc.diff_add_bg,
            cc.add_word_bg(),
            ink(cc.code_string),
            ADDED,
        ),
        row(vec![
            Span::styled(" 3 ", number),
            on("}", cc.diff_text, none),
        ]),
    ]
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
    let mut list = entries(model);
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
                "Choose a theme for the interface",
                Style::default().add_modifier(Modifier::BOLD),
            ),
        ]),
    ]
}

fn entries(model: &Model) -> Vec<Line<'static>> {
    let Some(picker) = &model.theme_picker else {
        return Vec::new();
    };
    let theme = &model.theme;
    let cc = theme.cc();
    ENTRIES
        .iter()
        .enumerate()
        .map(|(index, (name, label))| {
            let selected = index == picker.index;
            let ink = if selected { cc.permission } else { theme.text };
            let pointer = if selected { "❯" } else { " " };
            let check = if *name == picker.applied { "✔" } else { " " };
            Line::from(vec![
                span("   ", ink),
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
    rows.extend(entries(model));
    if with_preview {
        rows.push(ui::blank());
        rows.push(dashed(model));
        rows.extend(preview(model, picker.syntax));
        rows.push(dashed(model));
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
