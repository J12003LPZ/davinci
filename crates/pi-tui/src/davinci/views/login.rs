//! `3d` — `/login`. A device-code flow in progress over a ledger of every
//! provider and where its credential came from: an environment variable, the
//! auth file, a refresh token, or nothing.
//!
//! The waiting spinner is the one already on the 250ms clock (design.md §8),
//! and the panel says plainly that ctrl+c cancels the login and not the
//! session.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/views/login.ex`. The mockup's
//! `> /login anthropic` echo names a provider the model does not carry, so
//! the sheet opens straight on the flow.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use crate::davinci::model::{Credential, Model, ProviderRow};
use crate::davinci::theme::{State, Theme};
use crate::davinci::ui::{blank, clip_ellipsis, pad, span, truncate_run, wrap, Surface, MEASURE};

/// Column widths of the provider ledger, as the Elixir reference sets them.
const PROVIDER: u16 = 16;
const METHOD: usize = 10;
const STATE_W: usize = 12;
const SOURCE: usize = 30;

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width.min(MEASURE + 14);

    let mut rows: Vec<Line<'static>> = Vec::new();
    if let Some(device) = &model.device_code {
        let mut body: Vec<Vec<Span<'static>>> = vec![
            vec![
                span("1 ", th.border),
                span("open ", th.muted),
                span(device.url.clone(), th.secondary),
            ],
            vec![
                span("2 ", th.border),
                span("enter ", th.muted),
                Span::styled(
                    device.code.clone(),
                    Style::default().fg(th.text).add_modifier(th.emphasis),
                ),
            ],
            vec![
                span("3 ", th.border),
                span("pi writes the refresh token and returns here", th.muted),
            ],
            Vec::new(),
        ];
        for row in wrap(
            "the browser was not opened for you — nothing leaves this terminal \
             until you approve it there",
            width.saturating_sub(6),
        ) {
            body.push(vec![span(row, th.border)]);
        }
        body.push(Vec::new());
        body.push(vec![
            Span::styled(
                format!("{} ", th.spinner(model.tick, model.animate)),
                Style::default().fg(th.primary).add_modifier(th.emphasis),
            ),
            span("waiting for approval", th.text),
            span("  ·  ", th.border),
            span(format!("polled {}×", device.polls), th.muted),
        ]);
        body.push(vec![span(
            "ctrl+c cancels the login, not the session",
            th.border,
        )]);

        rows.extend(
            Surface::new(width, th)
                .border(th.primary)
                .title(vec![span("DEVICE AUTHORISATION", th.primary)])
                .right(vec![span(
                    format!("expires in {}", device.expires),
                    th.warning,
                )])
                .rows(body)
                .lines(),
        );
        rows.push(blank());
    }

    // At 80 columns the source column goes rather than the row wrapping
    // (design.md §7): the state is the answer, the source is the footnote.
    let source = model.width >= 88;
    let mut header = vec![
        pad(2, None),
        span(
            format!("{:<w$}", "PROVIDER", w = PROVIDER as usize - 1),
            th.border,
        ),
        span(format!("{:<w$}", "METHOD", w = METHOD + 1), th.border),
    ];
    if source {
        header.push(span(format!("{:<SOURCE$}", "SOURCE"), th.border));
    }
    header.push(span("STATE", th.border));
    rows.push(Line::from(header));

    if model.providers.is_empty() {
        rows.push(Line::from(vec![span(
            "no providers configured — /login <provider> adds one",
            th.muted,
        )]));
    } else {
        let selected = model.login_index % model.providers.len();
        for (index, provider) in model.providers.iter().enumerate() {
            rows.push(provider_row(provider, index == selected, source, th));
        }
    }

    rows.push(blank());
    for row in wrap(
        "keys are never echoed, never written to the transcript, never sent \
         to another provider",
        MEASURE,
    ) {
        rows.push(Line::from(vec![span(row, th.muted)]));
    }
    rows.push(Line::from(vec![
        span("enter re-authenticate", th.border),
        span(" · ", th.border),
        span("d /logout provider", th.border),
        span(" · ", th.border),
        span("esc close", th.border),
    ]));

    rows.into_iter()
        .map(|line| Line::from(truncate_run(line.spans, width)))
        .collect()
}

fn provider_row(provider: &ProviderRow, selected: bool, source: bool, th: &Theme) -> Line<'static> {
    let state = match provider.state {
        Credential::Ready | Credential::Local => State::Done,
        Credential::Pending => State::Active,
        Credential::Expired => State::Attention,
        Credential::Absent => State::Queued,
    };
    let color = th.state_color(state);
    let dim = provider.state == Credential::Absent;
    let band = selected.then_some(th.surface);
    let name_color = if selected {
        th.text
    } else if dim {
        th.border
    } else {
        th.muted
    };

    let mut spans = vec![
        strong_on(format!("{} ", state.glyph()), color, band, th),
        on(
            format!(
                "{:<w$}",
                clip_ellipsis(&provider.name, PROVIDER - 2),
                w = PROVIDER as usize - 1
            ),
            name_color,
            band,
        ),
        on(
            format!("{:<w$}", provider.method, w = METHOD + 1),
            if dim { th.border } else { th.muted },
            band,
        ),
    ];
    if source {
        spans.push(on(
            format!(
                "{:<SOURCE$}",
                clip_ellipsis(&provider.source, SOURCE as u16 - 1)
            ),
            th.border,
            band,
        ));
    }
    spans.push(on(
        format!("{:<STATE_W$}", state_label(provider.state)),
        color,
        band,
    ));
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
        assert!(drawn.iter().any(|row| row.contains("DEVICE AUTHORISATION")));
        assert!(drawn.iter().any(|row| row.contains("WQPT-FJ4M")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("https://claude.ai/oauth/device")));
        assert!(drawn.iter().any(|row| row.contains("expires in 8m 41s")));
        assert!(drawn.iter().any(|row| row.contains("polled 6×")));
        assert!(drawn
            .iter()
            .any(|row| row.contains("ctrl+c cancels the login, not the session")));
    }

    #[test]
    fn without_a_grant_in_flight_the_ledger_stands_alone() {
        let mut m = model(100);
        m.device_code = None;
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        assert!(!drawn.iter().any(|row| row.contains("DEVICE AUTHORISATION")));
        assert!(drawn.iter().any(|row| row.contains("PROVIDER")));
    }

    #[test]
    fn every_provider_states_its_method_and_credential_state() {
        let m = model(100);
        let drawn: Vec<String> = lines(&m).iter().map(text).collect();
        for (name, label) in [
            ("anthropic", "pending"),
            ("openai", "ready"),
            ("github-copilot", "expired"),
            ("xai", "absent"),
            ("llama.cpp", "running"),
        ] {
            assert!(
                drawn
                    .iter()
                    .any(|row| row.contains(name) && row.contains(label)),
                "{name} should read {label}: {drawn:?}"
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
        assert!(text(expired).starts_with('!'), "{}", text(expired));
        let absent = rows
            .iter()
            .find(|row| text(row).contains("xai"))
            .expect("the absent row");
        assert!(text(absent).starts_with('○'), "{}", text(absent));
        assert!(absent
            .spans
            .iter()
            .any(|span| span.style.fg == Some(m.theme.border)));
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
    fn nothing_overflows_at_any_width() {
        for width in [72u16, 80, 100, 120, 160] {
            let cap = width.min(MEASURE + 14);
            for row in lines(&model(width)) {
                assert!(
                    UnicodeWidthStr::width(text(&row).as_str()) <= cap as usize,
                    "row wider than {cap} at {width}: {:?}",
                    text(&row)
                );
            }
        }
    }
}
