//! `/login`: provider status with focused method/source details.
//! Authentication is still owned by the existing runtime and login dialog.
//! A device grant is displayed only when actual state supplies one.

use ratatui::text::Line;

use super::sheet::{hint, Composer, SheetChrome};
use crate::davinci::model::{Credential, Model};
use crate::davinci::ui::{section_detail, section_row, span};

pub fn lines(model: &Model) -> Vec<Line<'static>> {
    let th = &model.theme;
    let width = model.width;
    let mut rows = Vec::new();
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
    let selected = model.login_index % model.providers.len();
    for (index, provider) in model.providers.iter().enumerate() {
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
        hints: vec![hint(th, "↑↓ move"), hint(th, "enter configure")],
        escape: Some("esc close"),
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
        assert_eq!(ui::focused_row(&lines(&m)), Some(1));
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
        assert!(!drawn.contains('╭'));
    }

    #[test]
    fn credential_states_are_textual_even_without_color() {
        assert_eq!(state_label(Credential::Expired), "! expired");
        assert_eq!(state_label(Credential::Local), "local");
        assert_eq!(state_label(Credential::Pending), "pending");
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
        }
    }
}
