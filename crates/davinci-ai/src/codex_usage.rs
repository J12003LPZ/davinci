//! ChatGPT plan usage windows reported by the Codex backend.
//! Header names are conservative defaults until the explicit backend probe
//! records the names returned by the authenticated route.

use std::sync::Mutex;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub used_percent: f64,
    pub window_minutes: Option<u64>,
    pub resets_in_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageSnapshot {
    pub primary: Option<UsageWindow>,
    pub secondary: Option<UsageWindow>,
}

const WARN_AT: [f64; 2] = [80.0, 95.0];

struct State {
    latest: Option<CodexUsageSnapshot>,
    warned_primary: f64,
    pending: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    latest: None,
    warned_primary: 0.0,
    pending: None,
});

fn header(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.trim().to_string())
}

fn window(headers: &[(String, String)], prefix: &str) -> Option<UsageWindow> {
    let used_percent = header(headers, &format!("x-codex-{prefix}-used-percent"))?
        .parse()
        .ok()?;
    Some(UsageWindow {
        used_percent,
        window_minutes: header(headers, &format!("x-codex-{prefix}-window-minutes"))
            .and_then(|value| value.parse().ok()),
        resets_in_seconds: header(headers, &format!("x-codex-{prefix}-reset-after-seconds"))
            .and_then(|value| value.parse().ok()),
    })
}

pub fn parse_usage_headers(headers: &[(String, String)]) -> Option<CodexUsageSnapshot> {
    let snapshot = CodexUsageSnapshot {
        primary: window(headers, "primary"),
        secondary: window(headers, "secondary"),
    };
    (snapshot.primary.is_some() || snapshot.secondary.is_some()).then_some(snapshot)
}

pub fn record(snapshot: CodexUsageSnapshot) {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(primary) = &snapshot.primary {
        let crossed = WARN_AT.iter().rev().find(|threshold| {
            primary.used_percent >= **threshold && state.warned_primary < **threshold
        });
        if let Some(threshold) = crossed {
            state.warned_primary = *threshold;
            let reset = primary
                .resets_in_seconds
                .map(|seconds| format!(" · resets in {} min", seconds.div_ceil(60)))
                .unwrap_or_default();
            state.pending = Some(format!(
                "ChatGPT plan usage at {:.0}% of the current window{reset}",
                primary.used_percent
            ));
        }
        if primary.used_percent < 50.0 {
            state.warned_primary = 0.0;
        }
    }
    state.latest = Some(snapshot);
}

pub fn latest() -> Option<CodexUsageSnapshot> {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .latest
        .clone()
}

pub fn take_warning() -> Option<String> {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .pending
        .take()
}

pub fn is_usage_limit_error(message: &str) -> bool {
    message.contains("usage_limit_reached")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(name: &str, value: &str) -> (String, String) {
        (name.into(), value.into())
    }

    #[test]
    fn both_windows_are_read_from_headers() {
        let snapshot = parse_usage_headers(&[
            h("x-codex-primary-used-percent", "82.5"),
            h("x-codex-primary-window-minutes", "300"),
            h("x-codex-primary-reset-after-seconds", "1200"),
            h("x-codex-secondary-used-percent", "40"),
        ])
        .unwrap();
        let primary = snapshot.primary.unwrap();
        assert_eq!(primary.used_percent, 82.5);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_in_seconds, Some(1200));
        assert_eq!(snapshot.secondary.unwrap().used_percent, 40.0);
    }

    #[test]
    fn no_codex_headers_means_no_snapshot() {
        assert_eq!(
            parse_usage_headers(&[h("content-type", "text/event-stream")]),
            None
        );
    }

    #[test]
    fn usage_limit_error_is_recognized() {
        assert!(is_usage_limit_error(
            "{\"error\":{\"type\":\"usage_limit_reached\"}}"
        ));
        assert!(!is_usage_limit_error(
            "{\"error\":{\"type\":\"rate_limit_exceeded\"}}"
        ));
    }
}
