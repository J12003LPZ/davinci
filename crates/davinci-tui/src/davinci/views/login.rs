//! `/login`: provider status with focused method/source details.
//! Authentication is still owned by the existing runtime and login dialog.
//! A device grant is displayed only when actual state supplies one.

use ratatui::text::{Line, Span};

use super::extensions::{boxed, width_of};
use super::picker;
use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{Credential, Model, ProviderRow};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{
    self, clip_ellipsis, pad, section_detail, section_row, span, truncate_run, wrap, SELECTION_BAR,
    UNSELECTED_BAR,
};

/// Same list column as `/skills`: the detail pane takes the rest.
const LIST_WIDTH: u16 = 44;
/// Narrower or shorter than this, the providers stack as rows with the
/// focused one expanded: two panes need room to both read.
const SPLIT_MIN_WIDTH: u16 = 90;
const SPLIT_MIN_HEIGHT: u16 = 18;
/// Rows the sheet frame spends around its content (rule, title, blank, hint).
const FRAME_ROWS: usize = 6;

/// The providers the filter lets through, as indexes into `model.providers`.
/// A word matches the name, method, source or state (`ready`, `expired`).
pub fn visible_indices(model: &Model) -> Vec<usize> {
    model
        .providers
        .iter()
        .enumerate()
        .filter_map(|(at, row)| {
            picker::matches(
                &model.login_query,
                &[&row.name, &row.method, &row.source, state_label(row.state)],
            )
            .then_some(at)
        })
        .collect()
}

/// The selection, kept on a provider the filter shows.
fn selected_index(model: &Model, visible: &[usize]) -> Option<usize> {
    if visible.contains(&model.login_index) {
        Some(model.login_index)
    } else {
        visible.first().copied()
    }
}

/// The search box, always on screen above the list.
fn header(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let mut inner = vec![span("⌕ ", th.primary)];
    if model.login_query.is_empty() {
        inner.push(span("Filter providers", th.muted));
    } else {
        inner.push(span(model.login_query.clone(), th.text));
    }
    inner.push(span("▏", th.primary));
    let total = model.providers.len();
    let right = vec![span(
        if model.login_query.trim().is_empty() {
            format!("{total} providers")
        } else {
            let hits = visible_indices(model).len();
            format!("{hits} of {total}")
        },
        th.muted,
    )];
    let mut rows = boxed(model, inner, right);
    rows.push(Line::default());
    rows
}

/// How many leading rows stay on screen while the list scrolls.
pub fn pinned_rows(model: &Model) -> usize {
    header(model).len()
}

fn splits(model: &Model, visible: &[usize]) -> bool {
    width_of(model) >= SPLIT_MIN_WIDTH && model.height >= SPLIT_MIN_HEIGHT && !visible.is_empty()
}

/// The glyph and ink that say how healthy a credential is.
fn mark(state: Credential, th: &Theme) -> (&'static str, ratatui::style::Color) {
    match state {
        Credential::Ready | Credential::Local => ("●", th.success),
        Credential::Pending => ("◐", th.warning),
        Credential::Expired => ("!", th.warning),
        Credential::Absent => ("○", th.muted),
    }
}

fn list_row(model: &Model, row: &ProviderRow, selected: bool, width: u16) -> Line<'static> {
    let th = &model.theme;
    let cc = th.cc();
    let (glyph, ink) = mark(row.state, th);
    let label = list_label(row.state);
    let mut spans = vec![
        span(
            if selected {
                SELECTION_BAR
            } else {
                UNSELECTED_BAR
            },
            if selected { cc.permission } else { cc.inactive },
        ),
        span(glyph, ink),
        pad(1, None),
    ];
    let room = width.saturating_sub(2 + 2 + label.chars().count() as u16 + 1);
    spans.push(ui::span_strong(
        clip_ellipsis(&row.name, room),
        if selected { cc.permission } else { th.text },
        th,
    ));
    let used = ui::run_width(&spans);
    let gap = width
        .saturating_sub(used + label.chars().count() as u16 + 1)
        .max(1);
    spans.push(pad(gap, None));
    spans.push(span(label.to_string(), ink));
    Line::from(truncate_run(spans, width))
}

/// The right pane: a device grant in flight, then the selected provider.
fn detail_rows(model: &Model, selected: &ProviderRow, width: u16) -> Vec<Vec<Span<'static>>> {
    let th = &model.theme;
    let cc = th.cc();
    let mut out: Vec<Vec<Span<'static>>> = Vec::new();
    let label = |text: &str| vec![span(text.to_string(), th.muted)];
    let value = |text: &str| -> Vec<Vec<Span<'static>>> {
        wrap(text, width.saturating_sub(2))
            .into_iter()
            .map(|line| vec![pad(2, None), span(line, th.text)])
            .collect()
    };
    if let Some(device) = &model.device_code {
        out.push(label("DEVICE AUTHORIZATION"));
        out.extend(value("Open this address:"));
        out.extend(value(&device.url));
        out.extend(value(&format!("Code: {}", device.code)));
        if !device.expires.is_empty() {
            out.extend(value(&format!("Expires in {}", device.expires)));
        }
        out.push(vec![
            pad(2, None),
            span("◐ ", th.warning),
            span(
                format!("Waiting for approval · {} polls", device.polls),
                th.text,
            ),
        ]);
        out.push(Vec::new());
    }
    out.push(vec![ui::span_strong(selected.name.clone(), th.text, th)]);
    if !selected.method.is_empty() {
        out.push(label(&selected.method));
    }
    out.push(Vec::new());
    out.push(label("STATUS"));
    let (glyph, ink) = mark(selected.state, th);
    out.push(vec![
        pad(2, None),
        span(format!("{glyph} "), ink),
        span(state_label(selected.state).to_string(), ink),
    ]);
    if !selected.source.is_empty() {
        out.push(Vec::new());
        out.push(label("SOURCE"));
        out.extend(value(&selected.source));
    }
    out.push(Vec::new());
    out.push(label("ACTIONS"));
    out.push(vec![
        pad(2, None),
        span("enter", cc.permission),
        span(" configure", th.muted),
    ]);
    out
}

/// The list on the left, a rule, the selected provider on the right, each
/// exactly `height` rows so the sheet's own scrolling never moves the pane.
fn split_body(
    model: &Model,
    visible: &[usize],
    selected: usize,
    header_rows: usize,
) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = width_of(model);
    let height = usize::from(model.height)
        .saturating_sub(FRAME_ROWS + header_rows)
        .max(6);
    let at = visible.iter().position(|&i| i == selected).unwrap_or(0);
    let rows: Vec<Line<'static>> = visible
        .iter()
        .map(|&i| list_row(model, &model.providers[i], i == selected, LIST_WIDTH))
        .collect();
    let list = ui::window(rows, height, at, th);
    let pane_width = width.saturating_sub(LIST_WIDTH + 4);
    let mut pane = detail_rows(model, &model.providers[selected], pane_width);
    pane.truncate(height);
    (0..height)
        .map(|row| {
            let mut spans = list
                .get(row)
                .map(|line| line.spans.clone())
                .unwrap_or_default();
            let used = ui::run_width(&spans);
            spans.push(pad(LIST_WIDTH.saturating_sub(used), None));
            spans.push(span(" │ ", th.cc().subtle));
            if let Some(detail) = pane.get(row) {
                spans.extend(detail.clone());
            }
            Line::from(truncate_run(spans, width))
        })
        .collect()
}

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let mut rows = header(model);
    let visible = visible_indices(model);
    let selected = selected_index(model, &visible);
    if let (true, Some(selected)) = (splits(model, &visible), selected) {
        let head = rows.len();
        rows.extend(split_body(model, &visible, selected, head));
        return rows;
    }
    if let Some(device) = &model.device_code {
        rows.extend(section_detail(width, th, "Device authorization"));
        rows.extend(section_detail(width, th, "Open this address:"));
        rows.extend(section_detail(width, th, &device.url));
        rows.extend(section_detail(width, th, &format!("Code: {}", device.code)));
        if !device.expires.is_empty() {
            rows.extend(section_detail(
                width,
                th,
                &format!("Expires in {}", device.expires),
            ));
        }
        rows.extend(section_detail(
            width,
            th,
            &format!("Waiting for approval · {} polls", device.polls),
        ));
    }
    if model.providers.is_empty() {
        rows.extend(section_detail(
            width,
            th,
            "No providers configured. Use /login <provider> to configure one.",
        ));
        return rows;
    }
    let Some(selected) = selected else {
        rows.extend(section_detail(
            width,
            th,
            &format!("No provider matches \"{}\".", model.login_query.trim()),
        ));
        rows.extend(section_detail(
            width,
            th,
            "Press esc to clear the filter, or use /login <provider>.",
        ));
        return rows;
    };
    for &index in &visible {
        let provider = &model.providers[index];
        let focused = index == selected;
        rows.push(section_row(
            width,
            th,
            focused,
            &provider.name,
            state_label(provider.state),
        ));
        if focused {
            rows.extend(section_detail(width, th, &provider.name));
            rows.extend(section_detail(
                width,
                th,
                &format!("Status: {}", state_label(provider.state)),
            ));
            if !provider.method.is_empty() {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Method: {}", provider.method),
                ));
            }
            if !provider.source.is_empty() {
                rows.extend(section_detail(
                    width,
                    th,
                    &format!("Source: {}", provider.source),
                ));
            }
        }
    }
    rows
}

/// The short word the list column has room for; the pane says it in full.
fn list_label(state: Credential) -> &'static str {
    match state {
        Credential::Absent => "not set",
        Credential::Expired => "expired",
        other => state_label(other),
    }
}

pub(super) fn state_label(state: Credential) -> &'static str {
    match state {
        Credential::Ready => "ready",
        Credential::Pending => "pending",
        Credential::Expired => "! expired",
        Credential::Local => "local",
        Credential::Absent => "not configured",
    }
}

pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let ready = model
        .providers
        .iter()
        .filter(|r| matches!(r.state, Credential::Ready | Credential::Local))
        .count();
    SheetChrome {
        header_right: vec![span(
            format!("{ready} of {} ready", model.providers.len()),
            th.muted,
        )],
        status_third: Some(vec![span("providers", th.muted)]),
        hints: vec![
            hint(th, "↑↓ move"),
            hint(th, "enter configure"),
            hint(th, "type to filter"),
        ],
        escape: Some(if model.login_query.is_empty() {
            "esc close"
        } else {
            "esc clear filter"
        }),
        composer: Composer::Hidden,
        ..SheetChrome::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::{
        model::{DeviceCode, ProviderRow},
        theme::{ColorDepth, Theme},
        ui,
    };

    fn model(width: u16) -> Model {
        let mut m = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            32,
            false,
        );
        m.providers = vec![
            ProviderRow {
                name: "provider-one".into(),
                method: "oauth".into(),
                source: "auth file".into(),
                state: Credential::Ready,
            },
            ProviderRow {
                name: "provider-two".into(),
                method: "api key".into(),
                source: "environment".into(),
                state: Credential::Absent,
            },
        ];
        m
    }
    fn text(m: &Model) -> String {
        lines(m)
            .iter()
            .map(Line::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn provider_status_is_always_named_and_focus_expands_method_and_source() {
        let mut m = model(80);
        let drawn = text(&m);
        assert!(drawn.contains("provider-one") && drawn.contains("provider-two"));
        assert!(drawn.contains("not configured") && drawn.contains("ready"));
        assert!(drawn.contains("Method: oauth") && drawn.contains("Source: auth file"));
        assert!(!drawn.contains("Source: environment"));
        m.login_index = 1;
        assert!(text(&m).contains("Source: environment"));
        assert_eq!(ui::focused_row(&lines(&m)), Some(pinned_rows(&m) + 1));
    }

    #[test]
    fn device_grants_are_vertical_and_do_not_invent_browser_or_cancel_guarantees() {
        let mut m = model(40);
        assert!(!text(&m).contains("Device authorization"));
        m.device_code = Some(DeviceCode {
            code: "ABCD-1234".into(),
            url: "https://example.test/device".into(),
            expires: "5m".into(),
            polls: 2,
        });
        let drawn = text(&m);
        for value in [
            "Device authorization",
            "https://example.test/device",
            "Code: ABCD-1234",
            "Expires in 5m",
            "Waiting for approval",
        ] {
            assert!(drawn.contains(value), "{value}: {drawn}");
        }
        assert!(!drawn.contains("browser was not opened") && !drawn.contains("ctrl+c cancels"));
        // Only the search box is framed; the grant itself stays plain rows.
        assert_eq!(drawn.matches('╭').count(), 1, "{drawn}");
    }

    #[test]
    fn credential_states_are_textual_even_without_color() {
        assert_eq!(state_label(Credential::Expired), "! expired");
        assert_eq!(state_label(Credential::Local), "local");
        assert_eq!(state_label(Credential::Pending), "pending");
    }

    fn tall(width: u16, height: u16) -> Model {
        let mut m = model(width);
        m.height = height;
        m.providers.push(ProviderRow {
            name: "provider-three".into(),
            method: "oauth".into(),
            source: "auth file".into(),
            state: Credential::Expired,
        });
        m
    }

    #[test]
    fn the_search_box_is_always_there_and_counts_what_it_leaves() {
        let mut m = tall(100, 30);
        let drawn = text(&m);
        assert!(drawn.contains("⌕ Filter providers"), "{drawn}");
        assert!(drawn.contains("3 providers"), "{drawn}");
        m.login_query = "oauth".into();
        let drawn = text(&m);
        assert!(drawn.contains("⌕ oauth"), "{drawn}");
        assert!(drawn.contains("2 of 3"), "{drawn}");
        assert!(!drawn.contains("provider-two"), "{drawn}");
    }

    #[test]
    fn a_word_matches_the_name_method_source_or_state() {
        let mut m = tall(100, 30);
        for (query, expect) in [
            ("two", vec![1]),
            ("api key", vec![1]),
            ("environment", vec![1]),
            ("expired", vec![2]),
            ("ready", vec![0]),
            ("PROVIDER", vec![0, 1, 2]),
            ("zzz", vec![]),
        ] {
            m.login_query = query.into();
            assert_eq!(visible_indices(&m), expect, "{query}");
        }
    }

    #[test]
    fn a_roomy_window_splits_into_a_list_and_the_selected_providers_details() {
        let mut m = tall(100, 30);
        m.login_index = 2;
        let drawn = text(&m);
        let row = |needle: &str| {
            drawn
                .lines()
                .find(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle}: {drawn}"))
                .to_string()
        };
        // List column left of the rule, details right of it, on shared rows.
        let list = row("❯");
        assert!(
            list.contains("provider-three") && list.contains("expired"),
            "{list}"
        );
        assert!(drawn.contains(" │ "), "{drawn}");
        for value in [
            "STATUS",
            "SOURCE",
            "ACTIONS",
            "enter configure",
            "auth file",
        ] {
            assert!(drawn.contains(value), "{value}: {drawn}");
        }
        assert_eq!(ui::focused_row(&lines(&m)), Some(pinned_rows(&m) + 2));
        // The body fills the window exactly so the sheet never scrolls it.
        assert_eq!(lines(&m).len(), usize::from(m.height) - FRAME_ROWS);
    }

    #[test]
    fn a_device_grant_sits_above_the_selected_providers_details() {
        let mut m = tall(100, 30);
        m.device_code = Some(DeviceCode {
            code: "ABCD-1234".into(),
            url: "https://example.test/device".into(),
            expires: "5m".into(),
            polls: 2,
        });
        let drawn = text(&m);
        let at = |needle: &str| {
            drawn
                .find(needle)
                .unwrap_or_else(|| panic!("{needle}: {drawn}"))
        };
        assert!(at("DEVICE AUTHORIZATION") < at("Code: ABCD-1234"));
        assert!(at("Waiting for approval · 2 polls") < at("STATUS"));
    }

    #[test]
    fn a_filter_with_no_hits_says_so_and_how_to_leave() {
        for width in [100, 60] {
            let mut m = tall(width, 30);
            m.login_query = "mistral".into();
            let drawn = text(&m);
            assert!(drawn.contains("No provider matches \"mistral\""), "{drawn}");
            assert!(drawn.contains("esc"), "{drawn}");
            assert!(drawn.contains("0 of 3"), "{drawn}");
            assert!(!drawn.contains("provider-one"), "{drawn}");
        }
    }

    #[test]
    fn the_hint_bar_offers_filtering_and_esc_clears_before_it_closes() {
        let mut m = model(100);
        let hints = |m: &Model| {
            let c = chrome(m);
            (
                c.hints
                    .iter()
                    .map(|h| h.iter().map(|s| s.content.to_string()).collect::<String>())
                    .collect::<Vec<_>>()
                    .join("|"),
                c.escape,
            )
        };
        let (listed, escape) = hints(&m);
        assert!(listed.contains("type to filter"), "{listed}");
        assert_eq!(escape, Some("esc close"));
        m.login_query = "o".into();
        assert_eq!(hints(&m).1, Some("esc clear filter"));
    }

    #[test]
    fn the_open_sheet_footer_keeps_login_s_own_keys() {
        let mut m = model(100);
        m.screen = crate::davinci::model::Screen::Login;
        let chrome = super::super::sheet::chrome(&m).expect("login is a sheet");
        let hints: String = chrome
            .hints
            .iter()
            .flat_map(|h| h.iter().map(|s| s.content.to_string()))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(hints.contains("type to filter"), "{hints}");
        assert_eq!(chrome.escape, Some("esc close"));
        m.login_query = "x".into();
        let chrome = super::super::sheet::chrome(&m).expect("login is a sheet");
        assert_eq!(chrome.escape, Some("esc clear filter"));
    }

    #[test]
    fn narrow_windows_keep_the_stacked_rows_under_the_search_box() {
        let m = tall(60, 30);
        let drawn = text(&m);
        assert!(drawn.contains("⌕ Filter providers"), "{drawn}");
        assert!(
            drawn
                .lines()
                .filter(|l| l.contains("provider-"))
                .all(|l| !l.contains('│')),
            "{drawn}"
        );
        assert!(drawn.contains("Method: oauth"), "{drawn}");
    }

    #[test]
    fn empty_and_unicode_providers_are_width_safe() {
        let mut m = model(80);
        m.providers.clear();
        assert!(text(&m).contains("No providers configured"));
        for width in [0, 1, 20, 32, 40, 80, 120] {
            let mut m = model(width);
            m.providers[0].name = "提供者 café 🦀 long-name".into();
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
            m.height = 30;
            for row in lines(&m) {
                assert!(ui::run_width(&row.spans) <= width);
            }
        }
    }
}
