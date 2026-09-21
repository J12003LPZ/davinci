//! Model selection: a compact quick picker and the full `/model` catalog.
//! The full catalog uses a numbered, unboxed panel with factual descriptions and an
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

pub fn visible_indices(model: &Model) -> Vec<usize> {
    model
        .catalog
        .iter()
        .enumerate()
        .filter_map(|(index, row)| {
            super::picker::matches(&model.catalog_query, &[&row.name, &row.id, &row.provider])
                .then_some(index)
        })
        .collect()
}

/// The catalog keeps source order so row indices still identify runtime choices.
pub fn catalog(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    if model.catalog.is_empty() {
        return section_detail(
            model.width,
            th,
            "No models in the catalog. Check model configuration or use /login.",
        );
    }
    let indices = visible_indices(model);
    if indices.is_empty() {
        return section_detail(
            model.width,
            th,
            "No matching models. Backspace to edit the search.",
        );
    }
    let name_width = indices
        .iter()
        .map(|index| {
            let entry = &model.catalog[*index];
            text_width(if entry.id.is_empty() {
                &entry.name
            } else {
                &entry.id
            }) + 4
        })
        .max()
        .unwrap_or(0)
        .min(38)
        .min(model.width.saturating_sub(12));
    indices
        .iter()
        .enumerate()
        .map(|(ordinal, index)| {
            let entry = &model.catalog[*index];
            catalog_row(
                model,
                entry,
                *index == model.catalog_index,
                is_current_model(model, entry),
                name_width,
                ordinal + 1,
            )
        })
        .collect()
}

/// Content-sized rounded panel, with the header fixed while the list scrolls.
pub fn screen(model: &Model, height: usize) -> Vec<Line<'static>> {
    picker_panel(model, height, true)
}

/// Exact height of the full picker before terminal-height clipping.
pub fn screen_height(model: &Model) -> usize {
    // Match Claude Code's fixed-height model sheet at 120x40; longer DaVinci
    // catalogs scroll instead of growing the panel upward.
    (catalog(model).len() + 11)
        .min(16)
        .min(usize::from(model.height.saturating_sub(4)).max(8))
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
        catalog_query: String::new(),
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
    let entries = catalog(model);
    let anchor = model
        .section_offset
        .unwrap_or_else(|| ui::focused_row(&entries).unwrap_or(0));
    if height < 8 || model.width < 12 {
        return ui::window(entries, height, anchor, th)
            .into_iter()
            .map(|row| Line::from(ui::truncate_run(row.spans, model.width)))
            .collect();
    }
    let bounded = |spans| Line::from(ui::truncate_run(spans, model.width));
    let detail = |text: String, color| bounded(vec![span("   ", th.text), span(text, color)]);
    let mut out = vec![
        super::chrome::effort_rule(model),
        bounded(vec![
            span("   ", th.text),
            ui::span_strong("Select model", th.text, th),
        ]),
    ];
    for text in ui::wrap(
        "Switch between configured models. Your pick becomes the default for new sessions. Use /model <provider/model> for a specific configured model.",
        model.width.saturating_sub(3),
    )
    .into_iter()
    .take(2)
    {
        out.push(detail(text, th.text));
    }
    out.push(ui::blank());
    if !model.catalog_query.is_empty() {
        out.push(detail(format!("Search: {}", model.catalog_query), th.muted));
    }
    let selected = visible_indices(model)
        .contains(&model.catalog_index)
        .then(|| model.catalog.get(model.catalog_index))
        .flatten();
    let mut footer = vec![ui::blank()];
    if let Some(entry) = selected {
        let level = entry
            .reasoning_levels
            .get(entry.reasoning_index)
            .map(String::as_str)
            .unwrap_or(&model.thinking_level);
        if !level.is_empty() {
            footer.push(detail(
                format!("● {level} effort (default) ←/→ to adjust"),
                th.primary,
            ));
        }
        if matches!(entry.credential, Credential::Absent | Credential::Expired) {
            footer.push(detail(
                format!(
                    "{} · /login {}",
                    super::login::state_label(entry.credential),
                    entry.provider
                ),
                th.warning,
            ));
        }
    }
    if let Some(warning) = window_warning(model) {
        footer.push(detail(warning, th.warning));
    }
    if let Some(notice) = &model.section_notice {
        footer.push(detail(notice.clone(), th.warning));
    }
    footer.push(ui::blank());
    footer.push(detail(
        if echo && model.width < 60 {
            "Enter save · s session · Esc cancel".into()
        } else if echo {
            "Enter to set as default · s to use this session only · Esc to cancel".into()
        } else {
            "↑↓ move · tab/↵ take · esc close".into()
        },
        th.muted,
    ));
    // On short terminals preserve selection and action guidance before decoration.
    let footer_room = height.saturating_sub(out.len() + 1);
    if footer.len() > footer_room {
        let hint = footer.pop().unwrap();
        footer.truncate(footer_room.saturating_sub(1));
        footer.push(hint);
    }
    let room = height.saturating_sub(out.len() + footer.len());
    out.extend(ui::window(entries, room, anchor, th));
    while out.len() + footer.len() < height {
        out.push(ui::blank());
    }
    out.extend(footer);
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
    let mut facts = vec![provider_label(&entry.provider).to_string()];
    if !entry.window.is_empty() {
        facts.push(format!("{} context", entry.window));
    }
    if !entry.reasoning_levels.is_empty() {
        facts.push("Adjustable effort".into());
    }
    facts
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn is_current_model(model: &Model, entry: &crate::davinci::model::CatalogRow) -> bool {
    if !model.active_provider.is_empty() {
        return model.active_provider == entry.provider && model.model_name == entry.id;
    }
    if model.model_name == format!("{}/{}", entry.provider, entry.id) {
        return true;
    }
    // An unqualified identifier is only unambiguous when exactly one provider owns it.
    let matches = |candidate: &crate::davinci::model::CatalogRow| {
        model.model_name == candidate.id || model.model_name == candidate.name
    };
    matches(entry)
        && model
            .catalog
            .iter()
            .filter(|candidate| matches(candidate))
            .count()
            == 1
}

fn catalog_row(
    model: &Model,
    entry: &crate::davinci::model::CatalogRow,
    focused: bool,
    current: bool,
    name_width: u16,
    ordinal: usize,
) -> Line<'static> {
    let th = &model.theme;
    let color = if focused { th.primary } else { th.text };
    let name = if entry.id.is_empty() {
        &entry.name
    } else {
        &entry.id
    };
    let label = format!("{ordinal}. {name}{}", if current { " ✔" } else { "" });
    let clipped = ui::clip_ellipsis(&label, name_width);
    let mut spans = vec![
        span(if focused { "   ❯ " } else { "     " }, color),
        span(clipped.clone(), color),
    ];
    if model.width >= 60 {
        let padding = name_width.saturating_sub(text_width(&clipped)) + 2;
        spans.push(span(" ".repeat(usize::from(padding)), color));
        spans.push(span(
            model_description(entry),
            if focused { color } else { th.muted },
        ));
    }
    Line::from(ui::truncate_run(spans, model.width))
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
        assert!(!drawn.contains("LATEST"));
        for label in [
            "▔",
            "Select model",
            "Enter to set as default",
            "s to use this session only",
            "OpenAI Codex",
            "gpt-6-astra",
            "gpt-5.6-luna",
        ] {
            assert!(drawn.contains(label), "{label}: {drawn}");
        }
        let rows = catalog(&m);
        assert!(rows[0].to_string().contains("✔"));
        assert!(!rows[1].to_string().contains("✔"));
        assert_eq!(ui::focused_row(&rows), Some(1));
        assert!(rows[1]
            .spans
            .iter()
            .all(|s| s.style.fg == Some(m.theme.model_picker_colors().0)));
    }

    #[test]
    fn unavailable_models_and_selection_errors_remain_visible() {
        let mut m = model(100);
        m.catalog_index = 0;
        m.catalog[0].credential = Credential::Expired;
        m.catalog[0].provider = "example-provider".into();
        m.section_notice = Some("Selection failed".into());
        let drawn = text(&screen(&m, 24));
        assert!(!drawn.contains("LATEST"));
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
            drawn.contains("Select model")
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
                if width >= 40 && height >= 10 {
                    assert!(rows.last().unwrap().to_string().contains("Enter"));
                }
            }
        }
    }
}
