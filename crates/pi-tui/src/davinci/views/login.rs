//! `3d` — `/login`. A device-code flow in progress over a ledger of every
//! provider and where its credential came from: an environment variable, the
//! auth file, a refresh token, or nothing.
//!
//! The waiting spinner is the one already on the 250ms clock (design.md §8),
//! and the panel says plainly that ctrl+c cancels the login and not the
//! session.
//!
//! Mirrors artboard `3d` of `docs/ui/Pi TUI Instruments.dc.html`.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::sheet::{facts, hint, hint_dim, Composer, SheetChrome};
use crate::davinci::model::{Credential, Model, ProviderRow};
use crate::davinci::theme::{State, Theme};
use crate::davinci::ui::{
    blank, clip_ellipsis, column_header, footnote, pad, run_width, selection_bar, span, spread,
    truncate_run, wrap, Surface,
};

/// Column widths of the provider ledger, as the artboard sets them.
const PROVIDER: u16 = 21;
const METHOD: u16 = 10;
const STATE_W: u16 = 12;
/// The selection bar and the state glyph.
const LEAD: u16 = 5;
/// The code box beside the steps.
const CODE_BOX: u16 = 26;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;

    let mut rows: Vec<Line<'static>> = Vec::new();
    if let Some(device) = &model.device_code {
        let inner = width.saturating_sub(4);
        let provider = pending_provider(model).unwrap_or_default();
        let steps: Vec<Vec<Span<'static>>> = vec![
            vec![
                span("1 · ", th.border),
                span("open ", th.muted),
                span(device.url.clone(), th.secondary),
            ],
            vec![
                span("2 · ", th.border),
                span("enter the code below", th.muted),
            ],
            vec![
                span("3 · ", th.border),
                span(
                    "davinci writes the refresh token and returns here",
                    th.muted,
                ),
            ],
            Vec::new(),
        ];
        // The code sits in a box of its own at the right, letter-spaced so
        // it can be read across the room; the steps share its rows.
        let spaced: String = device
            .code
            .chars()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let code_box = Surface::new(CODE_BOX, th)
            .border(th.border)
            .row(centre(
                CODE_BOX - 4,
                vec![Span::styled(
                    spaced,
                    Style::default().fg(th.text).add_modifier(th.emphasis),
                )],
            ))
            .row(centre(
                CODE_BOX - 4,
                vec![span(format!("expires in {}", device.expires), th.warning)],
            ))
            .lines();
        let mut body: Vec<Vec<Span<'static>>> = steps
            .into_iter()
            .zip(code_box)
            .map(|(left, right)| spread(inner, left, right.spans).spans)
            .collect();
        for row in wrap(
            "the browser was not opened for you — nothing leaves this terminal \
             until you approve it there",
            inner.saturating_sub(CODE_BOX + 2),
        ) {
            body.push(vec![span(row, th.border)]);
        }
        body.push(Vec::new());
        body.push(
            spread(
                inner,
                vec![
                    Span::styled(
                        format!("{} ", th.spinner(model.tick, model.animate)),
                        Style::default().fg(th.primary).add_modifier(th.emphasis),
                    ),
                    span("waiting for approval", th.text),
                    span(" · ", th.border),
                    span(format!("polled {}×", device.polls), th.muted),
                ],
                vec![span("ctrl+c cancels the login, not the session", th.border)],
            )
            .spans,
        );

        let mut title = vec![span("DEVICE AUTHORISATION", th.primary)];
        if !provider.is_empty() {
            title.push(span(" · ", th.border));
            title.push(span(provider.to_uppercase(), th.muted));
        }
        rows.extend(
            Surface::new(width, th)
                .border(th.primary)
                .title(title)
                .rows(body)
                .lines(),
        );
        rows.push(blank());
    }

    // At 80 columns the source column goes rather than the row wrapping
    // (design.md §7): the state is the answer, the source is the footnote.
    let source = model.width >= 88;
    let mut columns: Vec<(&str, u16, bool)> = vec![
        ("", LEAD - 1, false),
        ("PROVIDER", PROVIDER, false),
        ("METHOD", METHOD, false),
    ];
    if source {
        columns.push(("SOURCE", 0, false));
    }
    columns.push(("STATE", if source { STATE_W } else { 0 }, true));
    rows.extend(column_header(width, &columns, th));

    if model.providers.is_empty() {
        rows.push(Line::from(vec![span(
            "no providers configured — /login <provider> adds one",
            th.muted,
        )]));
    } else {
        let selected = model.login_index % model.providers.len();
        for (index, provider) in model.providers.iter().enumerate() {
            rows.push(provider_row(provider, index == selected, source, width, th));
        }
    }

    rows.push(blank());
    for row in wrap(
        "keys are never echoed, never written to the transcript, never sent \
         to another provider",
        width,
    ) {
        rows.push(Line::from(vec![span(row, th.muted)]));
    }
    rows.extend(footnote(
        width,
        vec![span(
            "davinci auth print-bearer-token --provider openai-codex",
            th.text,
        )],
        vec![span("hands one to an external client", th.border)],
        th,
    ));

    rows.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

/// A run centred in `width`, as a surface row.
fn centre(width: u16, spans: Vec<Span<'static>>) -> Vec<Span<'static>> {
    let lead = width.saturating_sub(run_width(&spans)) / 2;
    let mut row = vec![pad(lead, None)];
    row.extend(spans);
    row
}

/// The provider whose grant is in flight, if one is.
fn pending_provider(model: &Model) -> Option<String> {
    model
        .providers
        .iter()
        .find(|row| row.state == Credential::Pending)
        .map(|row| row.name.clone())
}

/// The sheet's frame (design.md §11): where credentials live in the header,
/// how many providers are ready in the status bar, the context as a pie
/// beside the exit, no composer — the flow is the input.
pub fn chrome(model: &Model) -> SheetChrome {
    let th = &model.theme;
    let ready = model
        .providers
        .iter()
        .filter(|row| matches!(row.state, Credential::Ready | Credential::Local))
        .count();
    // A row may name several providers (`xai · deepseek · zai`); the count
    // is of providers, not rows.
    let total: usize = model
        .providers
        .iter()
        .map(|row| row.name.split(" · ").count())
        .sum();
    let fraction = model.context_fraction();
    let percent = (fraction * 100.0) as u32;
    SheetChrome {
        header_right: facts(
            th,
            vec![
                if model.facts.auth_path.is_empty() {
                    Vec::new()
                } else {
                    vec![
                        span("credentials in ", th.muted),
                        span(model.facts.auth_path.clone(), th.text),
                    ]
                },
                if model.facts.auth_mode.is_empty() {
                    Vec::new()
                } else {
                    vec![span(model.facts.auth_mode.clone(), th.muted)]
                },
            ],
        ),
        status_third: (total > 0)
            .then(|| vec![span(format!("{ready} of {total} ready"), th.muted)]),
        status_right: Some(vec![
            span("mensura ", th.muted),
            span(th.pie(fraction), th.primary),
            span(format!(" {percent}%"), th.muted),
            span(" · ", th.border),
            span("esc close", th.border),
        ]),
        hints: vec![
            hint(th, "enter re-authenticate"),
            hint_dim(th, "k paste api key"),
            hint(th, "d /logout provider"),
            hint_dim(th, "r refresh now"),
        ],
        escape: Some("esc close"),
        composer: Composer::Hidden,
        echo: model
            .device_code
            .as_ref()
            .and_then(|_| pending_provider(model))
            .map(|provider| format!("/login {provider}")),
    }
}

fn provider_row(
    provider: &ProviderRow,
    selected: bool,
    source: bool,
    width: u16,
    th: &Theme,
) -> Line<'static> {
    let state = match provider.state {
        Credential::Ready | Credential::Local => State::Done,
        Credential::Pending => State::Active,
        Credential::Expired => State::Attention,
        Credential::Absent => State::Queued,
    };
    let dim = provider.state == Credential::Absent;
    let ink = if dim { th.dim() } else { *th };
    let color = ink.state_color(state);
    let band = selected.then_some(th.surface);
    let name_color = if selected {
        th.text
    } else if dim {
        ink.muted
    } else {
        th.text
    };

    let mut spans = vec![
        selection_bar(selected, th),
        strong_on(format!("{} ", state.glyph()), color, band, th),
        on(
            format!(
                "{:<w$}",
                clip_ellipsis(&provider.name, PROVIDER - 1),
                w = PROVIDER as usize
            ),
            name_color,
            band,
        ),
        on(
            format!("{:<w$}", provider.method, w = METHOD as usize + 1),
            ink.muted,
            band,
        ),
    ];
    let state_text = match provider.state {
        Credential::Pending | Credential::Absent => {
            format!("{} {}", state.glyph(), state_label(provider.state))
        }
        _ => state_label(provider.state).to_string(),
    };
    let state_run = vec![on(state_text, color, band)];
    if source {
        let room = width
            .saturating_sub(run_width(&spans))
            .saturating_sub(STATE_W + 1);
        spans.push(on(clip_ellipsis(&provider.source, room), ink.border, band));
    }
    let gap = width
        .saturating_sub(run_width(&spans))
        .saturating_sub(run_width(&state_run))
        .max(1);
    spans.push(pad(gap, band));
    spans.extend(state_run);
    Line::from(spans)
}

fn state_label(state: Credential) -> &'static str {
    match state {
        Credential::Ready => "ready",
        Credential::Pending => "pending",
        Credential::Expired => "expired",
        Credential::Local => "running",
        Credential::Absent => "absent",
    }
}

fn on(content: String, color: Color, band: Option<Color>) -> Span<'static> {
    let mut style = Style::default().fg(color);
    if let Some(band) = band {
        style = style.bg(band);
    }
    Span::styled(content, style)
}

fn strong_on(content: String, color: Color, band: Option<Color>, th: &Theme) -> Span<'static> {
    let mut style = Style::default().fg(color).add_modifier(th.emphasis);
    if let Some(band) = band {
        style = style.bg(band);
    }
    Span::styled(content, style)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::fixtures;
    use crate::davinci::model::{DeviceCode, Screen};
    use crate::davinci::theme::{ColorDepth, Theme};
    use unicode_width::UnicodeWidthStr;

    fn model(width: u16) -> Model {
        let mut model = Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            44,
            true,
        );
        model.providers = vec![
            ProviderRow {
                name: "anthropic".into(),
                method: "oauth".into(),
                source: "device flow, in progress".into(),
                state: Credential::Pending,
            },
            ProviderRow {
                name: "openai".into(),
                method: "api key".into(),
                source: "env OPENAI_API_KEY".into(),
                state: Credential::Ready,
            },
            ProviderRow {
                name: "github-copilot".into(),
                method: "oauth".into(),
                source: "refresh rejected 401 · 2d ago".into(),
                state: Credential::Expired,
            },
            ProviderRow {
                name: "xai".into(),
                method: "api key".into(),
                source: "never configured".into(),
                state: Credential::Absent,
            },
            ProviderRow {
                name: "llama.cpp".into(),
                method: "local".into(),
                source: "router at 127.0.0.1:8080".into(),
                state: Credential::Local,
            },
        ];
        model.device_code = Some(DeviceCode {
            code: "WQPT-FJ4M".into(),
            url: "https://claude.ai/oauth/device".into(),
            expires: "8m 41s".into(),
            polls: 6,
        });
        model.toggle_screen(Screen::Login);
        model
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn the_device_flow_states_its_code_url_expiry_and_what_ctrl_c_does() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("DEVICE AUTHORISATION · ANTHROPIC")));
        assert!(drawn.iter().any(|row| row.contains("W Q P T - F J 4 M")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("https://claude.ai/oauth/device")));
        assert!(drawn.iter().any(|row| row.contains("expires in 8m 41s")));
        assert!(drawn.iter().any(|row| row.contains("polled 6×")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("ctrl+c cancels the login, not the session")));
        // The code box shares the step rows rather than taking rows of its own.
        let first_step = drawn.iter().find(|row| row.contains("1 · open")).unwrap();
        assert!(first_step.contains('╭'), "{first_step}");
    }

    #[test]
    fn without_a_grant_in_flight_the_ledger_stands_alone() {
        let mut m = model(100);
        m.device_code = None;
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("DEVICE AUTHORISATION")));
        assert!(drawn.iter().any(|row| row.contains("PROVIDER")));
        assert!(chrome(&m).echo.is_none());
    }

    #[test]
    fn every_provider_states_its_method_and_credential_state() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        for (name, label) in [
            ("anthropic", "◉ pending"),
            ("openai", "ready"),
            ("github-copilot", "expired"),
            ("xai", "○ absent"),
            ("llama.cpp", "running"),
        ] {
            assert!(
                drawn
                    .iter()
                    .any(|row| row.contains(name) && row.trim_end().ends_with(label)),
                "{name} should end in {label}: {drawn:?}"
            );
        }
    }

    #[test]
    fn the_expired_row_warns_by_glyph_and_the_absent_row_dims() {
        let m = model(100);
        let rows = lines(&m);
        let expired = rows
            .iter()
            .find(|row| text(row).contains("github-copilot"))
            .expect("the expired row");
        assert!(text(expired).starts_with("   !"), "{}", text(expired));
        let absent = rows
            .iter()
            .find(|row| text(row).contains("xai"))
            .expect("the absent row");
        assert!(text(absent).starts_with("   ○"), "{}", text(absent));
        assert!(absent
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.dim().muted)));
    }

    #[test]
    fn at_eighty_columns_the_source_column_goes_rather_than_the_row_wrapping() {
        let wide = model(100);
        let drawn: Vec<String> = lines(&wide).iter().map(text).collect();
        assert!(drawn.iter().any(|row| row.contains("SOURCE")));
        assert!(drawn.iter().any(|row| row.contains("env OPENAI_API_KEY")));

        let narrow = model(80);
        let drawn: Vec<String> = lines(&narrow).iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("SOURCE")));
        assert!(!drawn.iter().any(|row| row.contains("env OPENAI_API_KEY")));
    }

    #[test]
    fn no_providers_says_how_to_add_one() {
        let mut m = model(100);
        m.providers.clear();
        m.device_code = None;
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("no providers configured")));
    }

    #[test]
    fn the_sheet_wears_its_artboard_chrome() {
        let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
        fixtures::dress_screen(&mut m, "3d");
        let c = chrome(&m);
        let header: String = c.header_right.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            header,
            "credentials in %USERPROFILE%\\.pi\\agent\\auth.json │ 0600"
        );
        let third: String = c
            .status_third
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert_eq!(third, "4 of 10 ready");
        let right: String = c
            .status_right
            .as_deref()
            .unwrap()
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        assert!(right.starts_with("mensura "), "{right}");
        assert!(right.ends_with("23% · esc close"), "{right}");
        assert_eq!(c.escape, Some("esc close"));
        assert_eq!(c.composer, Composer::Hidden);
        assert_eq!(c.echo.as_deref(), Some("/login anthropic"));
        let hint = text(&super::super::sheet::hint_row(&m, &c).unwrap());
        assert!(
            hint.starts_with("enter re-authenticate │ k paste api key"),
            "{hint}"
        );
        assert!(hint.trim_end().ends_with("esc close"), "{hint}");
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(drawn
            .iter()
            .any(|row| row.contains("davinci auth print-bearer-token --provider openai-codex")));
        assert!(!drawn.iter().any(|row| row.contains("esc close")));
    }

    #[test]
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            for row in lines(&model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= width as usize,
                    "row wider than {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
