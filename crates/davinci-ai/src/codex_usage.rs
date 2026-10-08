//! ChatGPT plan usage windows for `openai-codex` models: how much of the
//! 5-hour and weekly allowance is used, and when each resets.
//!
//! Two sources feed one snapshot. The Codex app-server's
//! `account/rateLimits/read` (and its `account/rateLimits/updated`
//! notifications) is the one Codex's own usage screen reads; the
//! `x-codex-*` response headers are kept for backends that send them.
//! Windows are told apart by their length (300 minutes, 10 080 minutes),
//! never by position: a plan may report only one, in either slot.

use std::sync::Mutex;

use serde_json::Value;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub used_percent: f64,
    pub window_minutes: Option<u64>,
    pub resets_in_seconds: Option<u64>,
    /// Unix seconds, when the source gives an absolute time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<i64>,
}

impl UsageWindow {
    /// When it resets, as Unix seconds, measured from `now` for a relative
    /// reset.
    pub fn reset_time(&self, now: i64) -> Option<i64> {
        self.resets_at.or_else(|| {
            self.resets_in_seconds
                .map(|seconds| now.saturating_add(seconds as i64))
        })
    }

    pub fn remaining_percent(&self) -> f64 {
        (100.0 - self.used_percent).clamp(0.0, 100.0)
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexUsageSnapshot {
    pub primary: Option<UsageWindow>,
    pub secondary: Option<UsageWindow>,
    /// `plus`, `pro`, `free`, … as the backend names the plan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_type: Option<String>,
}

/// A 5-hour window.
pub const FIVE_HOURS_MINUTES: u64 = 300;
/// A weekly window.
pub const WEEK_MINUTES: u64 = 7 * 24 * 60;
/// A 30-day window (what a free plan reported in October 2026).
pub const MONTH_MINUTES: u64 = 30 * 24 * 60;

/// A window's short name by its length: `5h`, `week`, `month`, else `3d`,
/// `2h`, `45m`, or `limit` when the length is unknown.
pub fn window_label(minutes: Option<u64>) -> String {
    match minutes {
        Some(FIVE_HOURS_MINUTES) => "5h".into(),
        Some(WEEK_MINUTES) => "week".into(),
        Some(MONTH_MINUTES) => "month".into(),
        Some(minutes) if minutes > 0 && minutes % 1440 == 0 => format!("{}d", minutes / 1440),
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => format!("{}h", minutes / 60),
        Some(minutes) if minutes > 0 => format!("{minutes}m"),
        _ => "limit".into(),
    }
}

impl CodexUsageSnapshot {
    /// Every reported window, in slot order.
    pub fn windows(&self) -> impl Iterator<Item = &UsageWindow> {
        self.primary.iter().chain(self.secondary.iter())
    }

    /// The window of `minutes` length, whichever slot carries it.
    pub fn window_of(&self, minutes: u64) -> Option<&UsageWindow> {
        self.windows()
            .find(|window| window.window_minutes == Some(minutes))
    }

    pub fn five_hour(&self) -> Option<&UsageWindow> {
        self.window_of(FIVE_HOURS_MINUTES)
    }

    pub fn weekly(&self) -> Option<&UsageWindow> {
        self.window_of(WEEK_MINUTES)
    }

    /// The window to warn about: the 5-hour one when there is one, else the
    /// first reported.
    fn leading(&self) -> Option<&UsageWindow> {
        self.five_hour().or_else(|| self.windows().next())
    }
}

const WARN_AT: [f64; 2] = [80.0, 95.0];

struct State {
    latest: Option<CodexUsageSnapshot>,
    warned_primary: f64,
    pending: Option<String>,
    unavailable: Option<String>,
}

static STATE: Mutex<State> = Mutex::new(State {
    latest: None,
    warned_primary: 0.0,
    pending: None,
    unavailable: None,
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
        resets_at: None,
    })
}

pub fn parse_usage_headers(headers: &[(String, String)]) -> Option<CodexUsageSnapshot> {
    let snapshot = CodexUsageSnapshot {
        primary: window(headers, "primary"),
        secondary: window(headers, "secondary"),
        plan_type: header(headers, "x-codex-plan-type"),
    };
    (snapshot.primary.is_some() || snapshot.secondary.is_some()).then_some(snapshot)
}

fn app_server_window(value: &Value) -> Option<UsageWindow> {
    Some(UsageWindow {
        used_percent: value.get("usedPercent")?.as_f64()?,
        window_minutes: value.get("windowDurationMins").and_then(Value::as_u64),
        resets_in_seconds: None,
        resets_at: value.get("resetsAt").and_then(Value::as_i64),
    })
}

/// The `rateLimits` object of `account/rateLimits/read` or of an
/// `account/rateLimits/updated` notification. A field it leaves out stays
/// `None`, so [`merge`] can tell "not sent" from "gone".
pub fn parse_app_server_rate_limits(rate_limits: &Value) -> Option<CodexUsageSnapshot> {
    let snapshot = CodexUsageSnapshot {
        primary: rate_limits.get("primary").and_then(app_server_window),
        secondary: rate_limits.get("secondary").and_then(app_server_window),
        plan_type: rate_limits
            .get("planType")
            .and_then(Value::as_str)
            .map(str::to_string),
    };
    (snapshot.primary.is_some() || snapshot.secondary.is_some() || snapshot.plan_type.is_some())
        .then_some(snapshot)
}

/// A sparse update laid over the last snapshot: what it carries replaces,
/// what it leaves out stays. An updated window replaces the window of the
/// same length wherever it sat (slots carry no meaning), and keeps that
/// window's reset time when the update leaves it out.
pub fn merge(base: Option<CodexUsageSnapshot>, update: CodexUsageSnapshot) -> CodexUsageSnapshot {
    let mut merged = base.unwrap_or_default();
    for (is_primary, window) in [(true, update.primary), (false, update.secondary)] {
        let Some(mut window) = window else {
            continue;
        };
        let length = |slot: &Option<UsageWindow>| slot.as_ref().and_then(|w| w.window_minutes);
        let into_primary = match window.window_minutes {
            Some(minutes) if length(&merged.primary) == Some(minutes) => true,
            Some(minutes) if length(&merged.secondary) == Some(minutes) => false,
            _ => is_primary,
        };
        let slot = if into_primary {
            &mut merged.primary
        } else {
            &mut merged.secondary
        };
        if let Some(previous) = slot.as_ref() {
            if previous.window_minutes == window.window_minutes {
                // A reset time already past means the window rolled over:
                // better no time than "now" forever.
                let now = now_seconds();
                window.resets_at = window
                    .resets_at
                    .or(previous.resets_at.filter(|at| *at > now));
                window.resets_in_seconds = window.resets_in_seconds.or(previous.resets_in_seconds);
            }
        }
        *slot = Some(window);
    }
    merged.plan_type = update.plan_type.or(merged.plan_type);
    merged
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

pub fn record(snapshot: CodexUsageSnapshot) {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    record_locked(&mut state, snapshot);
}

fn record_locked(state: &mut State, mut snapshot: CodexUsageSnapshot) {
    // A relative reset (from headers) is pinned to the clock now, so it
    // counts down instead of staying "in 20 minutes" forever.
    let now = now_seconds();
    for window in [&mut snapshot.primary, &mut snapshot.secondary]
        .into_iter()
        .flatten()
    {
        window.resets_at = window.reset_time(now);
    }
    if let Some(leading) = snapshot.leading() {
        let used = leading.used_percent;
        let crossed = WARN_AT
            .iter()
            .rev()
            .find(|threshold| used >= **threshold && state.warned_primary < **threshold);
        if let Some(threshold) = crossed {
            state.warned_primary = *threshold;
            let reset = leading
                .reset_time(now)
                .map(|at| {
                    format!(
                        " · resets in {} min",
                        (at.saturating_sub(now).max(0) as u64).div_ceil(60)
                    )
                })
                .unwrap_or_default();
            state.pending = Some(format!(
                "ChatGPT plan usage at {used:.0}% of the current window{reset}"
            ));
        }
        if used < 50.0 {
            state.warned_primary = 0.0;
        }
    }
    state.latest = Some(snapshot);
    state.unavailable = None;
}

/// Lay a sparse update over the latest snapshot and record the result, under
/// one lock so a read landing in between is not lost.
pub fn record_update(update: CodexUsageSnapshot) {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    let merged = merge(state.latest.clone(), update);
    record_locked(&mut state, merged);
}

/// Why no usage can be shown (the Codex CLI is missing, its login expired),
/// until the next successful read clears it.
pub fn set_unavailable(reason: impl Into<String>) {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .unavailable = Some(reason.into());
}

/// Drop the last snapshot and any pending warning, and say why: the numbers
/// on hand belong to an account DaVinci is not using.
pub fn withdraw(reason: impl Into<String>) {
    let mut state = STATE.lock().unwrap_or_else(|error| error.into_inner());
    withdraw_locked(&mut state, reason.into());
}

fn withdraw_locked(state: &mut State, reason: String) {
    state.latest = None;
    state.pending = None;
    state.warned_primary = 0.0;
    state.unavailable = Some(reason);
}

pub fn unavailable() -> Option<String> {
    STATE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .unavailable
        .clone()
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
    use serde_json::json;

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
        let primary = snapshot.primary.clone().unwrap();
        assert_eq!(primary.used_percent, 82.5);
        assert_eq!(primary.window_minutes, Some(300));
        assert_eq!(primary.resets_in_seconds, Some(1200));
        assert_eq!(snapshot.secondary.unwrap().used_percent, 40.0);
        assert_eq!(primary.reset_time(1_000), Some(2_200));
    }

    #[test]
    fn no_codex_headers_means_no_snapshot() {
        assert_eq!(
            parse_usage_headers(&[h("content-type", "text/event-stream")]),
            None
        );
    }

    #[test]
    fn app_server_windows_are_told_apart_by_length_not_slot() {
        // A Plus account: 5-hour and weekly, in either order.
        let plus = parse_app_server_rate_limits(&json!({
            "limitId": "codex",
            "primary": {"usedPercent": 59, "windowDurationMins": 10080, "resetsAt": 1791741600},
            "secondary": {"usedPercent": 27, "windowDurationMins": 300, "resetsAt": 1791349200},
            "planType": "plus"
        }))
        .unwrap();
        let five = plus.five_hour().unwrap();
        assert_eq!(five.used_percent, 27.0);
        assert_eq!(five.remaining_percent(), 73.0);
        assert_eq!(five.reset_time(0), Some(1791349200));
        assert_eq!(plus.weekly().unwrap().remaining_percent(), 41.0);
        assert_eq!(plus.plan_type.as_deref(), Some("plus"));
        // A free account: weekly only.
        let free = parse_app_server_rate_limits(&json!({
            "primary": {"usedPercent": 12, "windowDurationMins": 10080, "resetsAt": null},
            "secondary": null,
            "planType": "free"
        }))
        .unwrap();
        assert!(free.five_hour().is_none());
        assert_eq!(free.weekly().unwrap().used_percent, 12.0);
        assert_eq!(free.leading().unwrap().used_percent, 12.0);
        assert!(parse_app_server_rate_limits(&json!({})).is_none());
    }

    #[test]
    fn a_free_plan_with_one_thirty_day_window_is_read() {
        // The shape a free account returned live (October 2026), with the
        // account id left out.
        let free = parse_app_server_rate_limits(&json!({
            "limitId": "codex", "limitName": null, "normalModelSlug": null,
            "primary": {"usedPercent": 0, "windowDurationMins": 43200, "resetsAt": 1793936153},
            "secondary": null,
            "credits": {"hasCredits": false, "unlimited": false, "balance": null},
            "individualLimit": null, "spendControlReached": false,
            "planType": "free", "rateLimitReachedType": null
        }))
        .unwrap();
        assert!(free.five_hour().is_none() && free.weekly().is_none());
        let windows: Vec<_> = free.windows().collect();
        assert_eq!(windows.len(), 1);
        assert_eq!(window_label(windows[0].window_minutes), "month");
        assert_eq!(windows[0].remaining_percent(), 100.0);
        assert_eq!(free.leading().unwrap().used_percent, 0.0);
    }

    #[test]
    fn windows_are_named_by_length() {
        assert_eq!(window_label(Some(300)), "5h");
        assert_eq!(window_label(Some(10_080)), "week");
        assert_eq!(window_label(Some(43_200)), "month");
        assert_eq!(window_label(Some(4_320)), "3d");
        assert_eq!(window_label(Some(120)), "2h");
        assert_eq!(window_label(Some(45)), "45m");
        assert_eq!(window_label(Some(0)), "limit");
        assert_eq!(window_label(None), "limit");
    }

    #[test]
    fn a_sparse_update_keeps_what_it_does_not_carry() {
        let base = parse_app_server_rate_limits(&json!({
            "primary": {"usedPercent": 10, "windowDurationMins": 300},
            "secondary": {"usedPercent": 40, "windowDurationMins": 10080},
            "planType": "plus"
        }));
        let update = parse_app_server_rate_limits(&json!({
            "primary": {"usedPercent": 35, "windowDurationMins": 300}
        }))
        .unwrap();
        let merged = merge(base, update);
        assert_eq!(merged.five_hour().unwrap().used_percent, 35.0);
        assert_eq!(merged.weekly().unwrap().used_percent, 40.0);
        assert_eq!(merged.plan_type.as_deref(), Some("plus"));
    }

    #[test]
    fn an_update_in_the_other_slot_replaces_the_window_of_its_length() {
        // The read put the weekly window first; the update sends the 5-hour
        // one in the primary slot, without a reset time.
        let base = parse_app_server_rate_limits(&json!({
            "primary": {"usedPercent": 59, "windowDurationMins": 10080, "resetsAt": 4_000_002_000_i64},
            "secondary": {"usedPercent": 27, "windowDurationMins": 300, "resetsAt": 4_000_001_000_i64},
        }));
        let update = parse_app_server_rate_limits(&json!({
            "primary": {"usedPercent": 31, "windowDurationMins": 300}
        }))
        .unwrap();
        let merged = merge(base, update);
        assert_eq!(merged.weekly().unwrap().used_percent, 59.0);
        let five = merged.five_hour().unwrap();
        assert_eq!(five.used_percent, 31.0);
        assert_eq!(five.resets_at, Some(4_000_001_000));
        // A kept reset time already past is dropped: the window rolled over.
        let past = parse_app_server_rate_limits(&json!({
            "primary": {"usedPercent": 90, "windowDurationMins": 300, "resetsAt": 1000}
        }));
        let rolled = merge(
            past,
            parse_app_server_rate_limits(&json!({
                "primary": {"usedPercent": 2, "windowDurationMins": 300}
            }))
            .unwrap(),
        );
        assert_eq!(rolled.five_hour().unwrap().resets_at, None);
        // An update with no length falls back to its slot.
        let merged = merge(
            Some(merged),
            parse_app_server_rate_limits(&json!({"secondary": {"usedPercent": 40}})).unwrap(),
        );
        assert_eq!(merged.secondary.unwrap().window_minutes, None);
    }

    #[test]
    fn used_past_a_hundred_leaves_nothing_not_less_than_nothing() {
        let window = UsageWindow {
            used_percent: 104.0,
            window_minutes: Some(300),
            resets_in_seconds: None,
            resets_at: None,
        };
        assert_eq!(window.remaining_percent(), 0.0);
    }

    #[test]
    fn withdrawing_drops_the_snapshot_and_the_warning() {
        let mut state = State {
            latest: None,
            warned_primary: 0.0,
            pending: None,
            unavailable: None,
        };
        record_locked(
            &mut state,
            parse_app_server_rate_limits(&json!({
                "primary": {"usedPercent": 90, "windowDurationMins": 300}
            }))
            .unwrap(),
        );
        assert!(state.latest.is_some() && state.pending.is_some());
        withdraw_locked(&mut state, "other account".into());
        assert!(state.latest.is_none() && state.pending.is_none());
        assert_eq!(state.warned_primary, 0.0);
        assert_eq!(state.unavailable.as_deref(), Some("other account"));
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
