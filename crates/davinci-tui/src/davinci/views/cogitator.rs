//! Model selection: a compact quick picker and the full `/model` catalog.
//! The full catalog uses a rounded panel with inline descriptions and an
//! independent current-model marker. Runtime selection remains index-based.

use ratatui::text::Line;

use super::chrome::thousands;
use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{Credential, Model};
use crate::davinci::ui::{self, section_detail, section_row, span, Surface};

const CARD_WIDTH: u16 = 64;

pub fn lines(model: &Model, config_path: &str) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inset = if model.bare() {
        0
    } else {
        model.width.saturating_sub(CARD_WIDTH) / 2
    };
    let inner = model.width.saturating_sub(inset * 2).saturating_sub(4);
    let selected = model.selection(model.models.len());
    let mut body = Vec::new();
    for (index, item) in model.models.iter().enumerate() {
        body.push(section_row(inner, th, Some(index) == selected, &item.name, &item.window).spans);
        if Some(index) == selected {
            body.extend(
                section_detail(
                    inner,
                    th,
                    &format!("{} · context {}", item.name, item.window),
                )
                .into_iter()
                .map(|r| r.spans),
            );
        }
    }
    if body.is_empty() {
        body.extend(
            section_detail(
                inner,
                th,
                "No models available. Use /login to configure a provider.",
            )
            .into_iter()
            .map(|r| r.spans),
        );
    }
    if !config_path.is_empty() {
        body.extend(
            section_detail(inner, th, &format!("Config: {config_path}"))
                .into_iter()
                .map(|r| r.spans),
        );
    }
    body.push(
        ui::hint_row(
            inner,
            &[
                hint(th, "↑↓ move"),
                hint(
                    th,
                    if model.overlay_offset.is_some() {
                        "enter back"
                    } else {
                        "enter select"
                    },
                ),
            ],
            Some("esc cancel"),
            th,
        )
        .spans,
    );
    Surface::section(model.width, th)
        .inset(inset)
        .title(vec![span("Select model", th.primary)])
        .rows(body)
        .lines()
}

fn text_width(text: &str) -> u16 {
    unicode_width::UnicodeWidthStr::width(text).min(u16::MAX as usize) as u16
}

/// The catalog keeps source order so row indices still identify runtime choices.
pub fn catalog(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let (accent, _) = th.model_picker_colors();
    if model.catalog.is_empty() {
        return section_detail(
            model.width,
            th,
            "No models in the catalog. Check model configuration or use /login.",
        );
    }
    let selected = model.catalog_index % model.catalog.len();
    let mut rows = Vec::new();
    let name_width = model
        .catalog
        .iter()
        .map(|item| {
            text_width(if item.id.is_empty() {
                &item.name
            } else {
                &item.id
            })
        })
        .max()
        .unwrap_or(0)
        .min(36)
        .min(model.width.saturating_sub(6));
    let mut provider = None;
    for (index, entry) in model.catalog.iter().enumerate() {
        if provider != Some(entry.provider.as_str()) {
            provider = Some(entry.provider.as_str());
            let mut heading = vec![ui::span_strong(provider_label(&entry.provider), accent, th)];
            if entry.provider == "openai-codex" && model.width >= 90 {
                heading.push(span(
                    "   Fast, capable models for coding, reasoning, and agentic tasks",
                    th.muted,
                ));
            }
            rows.push(Line::from(ui::truncate_run(heading, model.width)));
        }
        rows.push(catalog_row(
            model,
            entry,
            index == selected,
            is_current_model(model, entry),
            name_width,
        ));
    }
    rows
}

/// Content-sized rounded panel, with the header fixed while the list scrolls.
pub fn screen(model: &Model, height: usize) -> Vec<Line<'static>> {
    picker_panel(model, height, true)
}

/// Exact height of the full picker before terminal-height clipping.
pub fn screen_height(model: &Model) -> usize {
    let controls = usize::from(model.width.saturating_sub(6) < 76);
    let reasoning = usize::from(
        model
            .catalog
            .get(model.catalog_index % model.catalog.len().max(1))
            .and_then(|row| row.reasoning_levels.get(row.reasoning_index))
            .is_some(),
    );
    let notice = usize::from(
        model
            .catalog
            .get(model.catalog_index % model.catalog.len().max(1))
            .is_some_and(|entry| {
                matches!(entry.credential, Credential::Absent | Credential::Expired)
            })
            || window_warning(model).is_some()
            || model.section_notice.is_some(),
    );
    // Command echo + gap, panel borders, title/help/rule, optional controls and
    // reasoning rows, the catalog itself, and an optional warning row.
    7 + controls + reasoning + catalog(model).len() + notice
}

/// Model argument completion shares the catalog presentation; its values and
/// order still come from the autocomplete engine, including extension choices.
pub fn suggestions(model: &Model) -> Option<Vec<Line<'static>>> {
    let composer = model.composer.to_string();
    if !composer
        .strip_prefix("/model")
        .is_some_and(|tail| tail.starts_with(char::is_whitespace))
    {
        return None;
    }
    let found = model.suggestions.as_ref()?;
    if found.items.is_empty() {
        return None;
    }
    let catalog = found
        .items
        .iter()
        .map(|item| {
            if let Some(entry) = model
                .catalog
                .iter()
                .find(|entry| entry.name == item.label || entry.name == item.value)
            {
                return entry.clone();
            }
            let known = model
                .models
                .iter()
                .find(|entry| entry.name == item.label || entry.name == item.value);
            let (provider, id) = item
                .value
                .split_once('/')
                .unwrap_or(("", item.value.as_str()));
            crate::davinci::model::CatalogRow {
                name: item.label.clone(),
                provider: known
                    .map(|entry| entry.provider.clone())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| provider.trim().into()),
                id: known
                    .map(|entry| entry.id.clone())
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| id.trim().into()),
                window: known.map(|entry| entry.window.clone()).unwrap_or_default(),
                credential: Credential::Ready,
                thinking: "none".into(),
                ..Default::default()
            }
        })
        .collect();
    let picker = Model {
        catalog,
        catalog_index: model.suggestion_index,
        section_offset: None,
        section_notice: None,
        ..model.clone()
    };
    let height = model
        .suggestion_rows
        .saturating_add(9)
        .min(model.height.saturating_sub(5) as usize);
    Some(picker_panel(&picker, height, false))
}

fn picker_panel(model: &Model, height: usize, echo: bool) -> Vec<Line<'static>> {
    let th = &model.theme;
    let inner = Model {
        width: model.width.saturating_sub(6),
        ..model.clone()
    };
    let entries = catalog(&inner);
    let anchor = model
        .section_offset
        .unwrap_or_else(|| ui::focused_row(&entries).unwrap_or(0));
    if height < 10 || model.width < 8 {
        return ui::window(entries, height, anchor, th)
            .into_iter()
            .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
            .collect();
    }
    let width = inner.width;
    let controls = if !echo {
        "↑↓ move   tab/↵ take   esc close"
    } else if model.section_offset.is_some() {
        "↑↓ navigate   ↵ back   esc close"
    } else if width >= 76 {
        "↑↓ navigate   ↵ select   esc close"
    } else {
        "↑↓ move  ↵ select  esc close"
    };
    let mut header = vec![ui::paper_label("Select a model", th, false)];
    let separate_controls = width < 76;
    if !separate_controls {
        let gap = width.saturating_sub(ui::run_width(&header) + text_width(controls));
        header.push(span(" ".repeat(gap as usize), th.muted));
        header.push(span(controls, th.muted));
    }
    let mut content = vec![
        Line::from(header),
        Line::from(span(
            if echo {
                "↑↓ model · ←→ reasoning · Enter saves"
            } else {
                "Choose a model"
            },
            th.muted,
        )),
    ];
    if separate_controls {
        content.push(Line::from(span(controls, th.muted)));
    }
    if echo {
        if let Some(row) = model
            .catalog
            .get(model.catalog_index % model.catalog.len().max(1))
        {
            if let Some(level) = row.reasoning_levels.get(row.reasoning_index) {
                content.push(Line::from(span(
                    format!("Reasoning: ◀ {level} ▶"),
                    th.primary,
                )));
            }
        }
    }
    content.push(ui::print_rule(width, th));
    let selected = model
        .catalog
        .get(model.catalog_index % model.catalog.len().max(1));
    let mut notices = Vec::new();
    if let Some(entry) = selected {
        if matches!(entry.credential, Credential::Absent | Credential::Expired) {
            notices.push(format!(
                "{} · /login {}",
                super::login::state_label(entry.credential),
                entry.provider
            ));
        }
    }
    if let Some(warning) = window_warning(model) {
        notices.push(warning);
    }
    if let Some(notice) = &model.section_notice {
        notices.push(notice.clone());
    }
    let notice_rows = usize::from(!notices.is_empty());
    let room = height
        .saturating_sub(2 + usize::from(echo) * 2 + content.len() + notice_rows)
        .max(1);
    content.extend(ui::window(entries, room, anchor, th));
    if notice_rows > 0 {
        content.push(Line::from(span(notices.join(" · "), th.warning)));
    }
    let border = |left: &str, right: &str| {
        Line::from(span(
            format!(
                "{left}{}{right}",
                "─".repeat(model.width.saturating_sub(2) as usize)
            ),
            th.border,
        ))
    };
    let mut out = Vec::new();
    if echo {
        out.extend([Line::from(span("> /model", th.text)), ui::blank()]);
    }
    out.push(border("╭", "╮"));
    for row in content {
        let mut run = ui::truncate_run(row.spans, width);
        let padding = width.saturating_sub(ui::run_width(&run));
        let mut framed = vec![span("│  ", th.border)];
        framed.append(&mut run);
        framed.push(span(" ".repeat(padding as usize), th.text));
        framed.push(span("  │", th.border));
        out.push(Line::from(framed));
    }
    out.push(border("╰", "╯"));
    out.truncate(height);
    out
}

fn provider_label(provider: &str) -> &str {
    match provider {
        "openai-codex" => "OpenAI Codex",
        "openai" => "OpenAI",
        "anthropic" => "Anthropic",
        "google" | "google-vertex" => "Google",
        "github-copilot" => "GitHub Copilot",
        "openrouter" => "OpenRouter",
        "vercel-ai-gateway" => "Vercel AI Gateway",
        "deepseek" => "DeepSeek",
        "groq" => "Groq",
        "cerebras" => "Cerebras",
        "nvidia" => "NVIDIA",
        _ => provider,
    }
}

fn model_description(entry: &crate::davinci::model::CatalogRow) -> String {
    match entry.id.as_str() {
        "gpt-6-astra" => "Latest and most capable model".into(),
        "gpt-5.6-luna" => "Balanced performance for most tasks".into(),
        "gpt-5.6-sol" => "High reasoning for complex problems".into(),
        "gpt-5.6-terra" => "Optimized for long context and deep work".into(),
        "gpt-5.5" => "Reliable and efficient".into(),
        "gpt-5.4-mini" => "Faster, lightweight model".into(),
        _ if entry.thinking != "none" => format!("Reasoning model · {} context", entry.window),
        _ => format!("{} context", entry.window),
    }
}

fn is_current_model(model: &Model, entry: &crate::davinci::model::CatalogRow) -> bool {
    model.model_name == entry.id
        || model.model_name == entry.name
        || entry.name.ends_with(&format!("/{}", model.model_name))
}

fn catalog_row(
    model: &Model,
    entry: &crate::davinci::model::CatalogRow,
    focused: bool,
    current: bool,
    name_width: u16,
) -> Line<'static> {
    let th = &model.theme;
    let (accent, background) = th.model_picker_colors();
    let width = model.width;
    let name = if entry.id.is_empty() {
        &entry.name
    } else {
        &entry.id
    };
    let mut run = vec![
        span(if focused { ui::SELECTION_BAR } else { "   " }, accent),
        span(
            if current { "●  " } else { "○  " },
            if current { accent } else { th.muted },
        ),
        ui::span_strong(ui::clip_ellipsis(name, name_width), th.text, th),
    ];
    let badge = entry.id == "gpt-6-astra" && width >= 65;
    if width >= 65 {
        let pad = name_width.saturating_sub(text_width(name)) + 3;
        run.push(span(" ".repeat(pad as usize), th.text));
        let room = width.saturating_sub(ui::run_width(&run) + if badge { 10 } else { 0 });
        run.push(span(
            ui::clip_ellipsis(&model_description(entry), room),
            th.muted,
        ));
    }
    let padding = width.saturating_sub(ui::run_width(&run) + if badge { 8 } else { 0 });
    run.push(span(" ".repeat(padding as usize), th.text));
    if focused {
        for item in &mut run {
            item.style.bg = Some(background);
        }
    }
    if badge {
        run.push(ratatui::text::Span::styled(
            " LATEST ",
            ratatui::style::Style::default()
                .fg(th.background)
                .bg(accent)
                .add_modifier(ratatui::style::Modifier::BOLD),
        ));
    }
    Line::from(ui::truncate_run(run, width))
}

/// Warn about the model under consideration, not an unrelated small model.
fn window_warning(model: &Model) -> Option<String> {
    let entry = model
        .catalog
        .get(model.catalog_index % model.catalog.len().max(1))?;
    let window = parse_tokens(&entry.window)?;
    (window < model.context.0).then(|| {
        format!(
            "! {} of context will not fit a {} window",
            thousands(model.context.0),
            thousands(window)
        )
    })
}

fn parse_tokens(window: &str) -> Option<u64> {
    let trimmed = window.trim();
    let (number, scale) = if let Some(number) = trimmed.strip_suffix('k') {
        (number, 1_000.0)
    } else if let Some(number) = trimmed.strip_suffix('m') {
        (number, 1_000_000.0)
    } else {
        (trimmed, 1.0)
    };
    let value = number.parse::<f64>().ok()? * scale;
    (value.is_finite() && value >= 0.0).then_some(value as u64)
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let ready = model
        .catalog
        .iter()
        .filter(|r| matches!(r.credential, Credential::Ready | Credential::Local))
        .count();
    SheetChrome {
        header_right: vec![span(
            format!("{} models · {ready} ready", model.catalog.len()),
            th.muted,
        )],
        status_third: Some(vec![span(
            format!(
                "{} in rotation",
                model.catalog.iter().filter(|r| r.ring).count()
            ),
            th.muted,
        )]),
        hints: vec![
            hint(th, "↑↓ move"),
            hint(
                th,
                if model.overlay_offset.is_some() {
                    "enter back"
                } else {
                    "enter select"
                },
            ),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        fixtures,
        model::Overlay,
        theme::{ColorDepth, Theme},
    };

    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        fixtures::dress_screen(&mut m, "3a");
        m
    }
    fn text(rows: &[Line<'_>]) -> String {
        rows.iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn picker_matches_reference_and_keeps_current_separate_from_focus() {
        let mut m = model(140);
        m.catalog = vec![
            crate::davinci::model::CatalogRow {
                provider: "openai-codex".into(),
                id: "gpt-6-astra".into(),
                credential: Credential::Ready,
                ..Default::default()
            },
            crate::davinci::model::CatalogRow {
                provider: "openai-codex".into(),
                id: "gpt-5.6-luna".into(),
                credential: Credential::Ready,
                ..Default::default()
            },
        ];
        m.model_name = "gpt-6-astra".into();
        m.catalog_index = 1;
        let drawn = text(&screen(&m, 24));
        for label in [
            "╭",
            "╯",
            "SELECT A MODEL",
            "↑↓ model · ←→ reasoning · Enter saves",
            "↑↓ navigate",
            "OpenAI Codex",
            "LATEST",
            "Latest and most capable model",
            "Balanced performance for most tasks",
        ] {
            assert!(drawn.contains(label), "{label}: {drawn}");
        }
        let rows = catalog(&m);
        assert!(rows[1].to_string().contains("●"));
        assert!(rows[2].to_string().contains("○"));
        assert_eq!(ui::focused_row(&rows), Some(2));
        assert!(rows[2]
            .spans
            .iter()
            .all(|s| s.style.bg == Some(m.theme.model_picker_colors().1)));
    }

    #[test]
    fn unavailable_models_and_selection_errors_remain_visible() {
        let mut m = model(100);
        m.catalog_index = 0;
        m.catalog[0].credential = Credential::Expired;
        m.catalog[0].provider = "example-provider".into();
        m.section_notice = Some("Selection failed".into());
        let drawn = text(&screen(&m, 24));
        assert!(drawn.contains("expired"));
        assert!(drawn.contains("/login example-provider"));
        assert!(drawn.contains("Selection failed"));
    }

    #[test]
    fn context_warning_describes_only_the_focused_model() {
        let mut m = model(80);
        m.context.0 = 47_000;
        m.catalog[0].window = "200k".into();
        m.catalog[1].window = "32k".into();
        m.catalog_index = 0;
        assert!(window_warning(&m).is_none());
        m.catalog_index = 1;
        assert!(window_warning(&m)
            .unwrap()
            .contains("47k of context will not fit a 32k window"));
        for input in ["", "模型", "💡", "NaN", "-1k", "unknown"] {
            assert!(parse_tokens(input).is_none());
        }
        assert_eq!(parse_tokens("1.5m"), Some(1_500_000));
    }

    #[test]
    fn quick_picker_names_its_source_and_exit_and_handles_empty_state() {
        let mut m = model(80);
        m.toggle_overlay(Overlay::Cogitator);
        let drawn = text(&lines(&m, "config.json"));
        assert!(
            drawn.contains("SELECT MODEL")
                && drawn.contains("config.json")
                && drawn.contains("esc cancel")
        );
        assert!(!drawn.contains('╭'));
        m.models.clear();
        assert!(text(&lines(&m, "")).contains("No models available"));
        m.catalog.clear();
        assert!(text(&catalog(&m)).contains("No models in the catalog"));
    }

    #[test]
    fn every_width_is_cell_bounded_including_unicode() {
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.catalog[0].name = "模型 / café 🦀 long-model-name".into();
            for row in catalog(&m)
                .into_iter()
                .chain(lines(&m, "config.json"))
                .chain(screen(&m, 24))
            {
                assert!(ui::run_width(&row.spans) <= width, "{width}: {row:?}");
            }
        }
    }

    #[test]
    fn scrolled_catalog_is_bounded_at_every_terminal_size() {
        for width in [0, 1, 7, 8, 20, 40, 80, 140] {
            for height in [0, 1, 7, 9, 10, 16, 24] {
                let mut m = model(width);
                m.catalog.resize(100, m.catalog[0].clone());
                m.catalog_index = 99;
                let rows = screen(&m, height);
                assert!(rows.len() <= height);
                assert!(rows.iter().all(|row| ui::run_width(&row.spans) <= width));
                if width >= 8 && height >= 10 {
                    assert!(rows.last().unwrap().to_string().ends_with('╯'));
                }
            }
        }
    }
}
