//! Provider HTTP retries matching `vendor/pi/packages/ai/src/utils/provider-retry.ts`.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

#[derive(Debug, Clone, Default)]
pub struct ProviderRetryOptions {
    pub max_retries: u32,
    pub max_retry_delay_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub status: Option<u16>,
    pub headers: HashMap<String, String>,
    pub message: String,
}

impl ProviderError {
    pub fn new(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            message: message.into(),
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .insert(key.into().to_ascii_lowercase(), value.into());
        self
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// TS `isRetryableProviderError`.
pub fn is_retryable_provider_error(error: &ProviderError) -> bool {
    match error.header("x-should-retry") {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    match error.status {
        None => true,
        Some(408 | 409 | 429) => true,
        Some(status) => status >= 500,
    }
}

fn validate_server_retry_delay_ms(
    delay_ms: f64,
    max_retry_delay_ms: Option<u64>,
    provider_error_message: &str,
) -> Result<u64, ProviderError> {
    let max_delay_ms = max_retry_delay_ms.unwrap_or(DEFAULT_MAX_RETRY_DELAY_MS);
    if max_delay_ms > 0 && delay_ms > max_delay_ms as f64 {
        return Err(ProviderError::new(
            None,
            format!(
                "Server requested {}s retry delay (max: {}s). {provider_error_message}",
                (delay_ms / 1000.0).ceil() as u64,
                (max_delay_ms as f64 / 1000.0).ceil() as u64
            ),
        ));
    }
    Ok(delay_ms.max(0.0) as u64)
}

/// Parse `Retry-After` / `retry-after-ms` the way undici and the TS helper do.
pub fn retry_delay_from_headers(
    error: &ProviderError,
    retry_index: u32,
    max_retry_delay_ms: Option<u64>,
    now_ms: i64,
) -> Result<u64, ProviderError> {
    if let Some(retry_after_ms) = error.header("retry-after-ms") {
        if let Ok(value) = retry_after_ms.parse::<f64>() {
            if !value.is_nan() {
                return validate_server_retry_delay_ms(value, max_retry_delay_ms, &error.message);
            }
        }
    }
    if let Some(retry_after) = error.header("retry-after") {
        let delay_ms = if let Ok(seconds) = retry_after.parse::<f64>() {
            if seconds.is_nan() {
                http_date_delay_ms(retry_after, now_ms)
            } else {
                seconds * 1000.0
            }
        } else {
            http_date_delay_ms(retry_after, now_ms)
        };
        return validate_server_retry_delay_ms(delay_ms, max_retry_delay_ms, &error.message);
    }
    let exponential = (0.5 * 2_f64.powi(retry_index as i32)).min(8.0) * 1000.0;
    let jitter = if cfg!(test) {
        1.0
    } else {
        1.0 - fastrand(0.25)
    };
    Ok((exponential * jitter) as u64)
}

fn http_date_delay_ms(value: &str, now_ms: i64) -> f64 {
    parse_http_date_ms(value)
        .map(|then| (then - now_ms) as f64)
        .unwrap_or(0.0)
}

fn parse_http_date_ms(value: &str) -> Option<i64> {
    let rest = value.trim().split_once(", ")?.1;
    let mut parts = rest.split_whitespace();
    let day: i64 = parts.next()?.parse().ok()?;
    let month = month_num(parts.next()?)?;
    let year: i64 = parts.next()?.parse().ok()?;
    let mut time = parts.next()?.split(':');
    let hour: i64 = time.next()?.parse().ok()?;
    let min: i64 = time.next()?.parse().ok()?;
    let sec: i64 = time.next()?.parse().ok()?;
    Some(ymd_hms_to_unix_ms(year, month, day, hour, min, sec))
}

fn month_num(name: &str) -> Option<i64> {
    Some(match name {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    })
}

fn ymd_hms_to_unix_ms(year: i64, month: i64, day: i64, hour: i64, min: i64, sec: i64) -> i64 {
    let (year, month) = if month <= 2 {
        (year - 1, month + 9)
    } else {
        (year, month - 3)
    };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let doy = (153 * month + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    (days * 86400 + hour * 3600 + min * 60 + sec) * 1000
}

fn fastrand(span: f64) -> f64 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(1);
    (nanos as f64 / 1_000_000_000.0) * span
}

fn sleep_retry(ms: u64) {
    if ms == 0 || cfg!(test) {
        return;
    }
    std::thread::sleep(Duration::from_millis(ms));
}

/// TS `retryProviderRequest`.
pub fn retry_provider_request<T, F>(
    mut request: F,
    options: ProviderRetryOptions,
) -> Result<T, ProviderError>
where
    F: FnMut() -> Result<T, ProviderError>,
{
    let max_retries = options.max_retries;
    let mut retries_remaining = max_retries;
    loop {
        match request() {
            Ok(value) => return Ok(value),
            Err(error) => {
                if retries_remaining == 0 || !is_retryable_provider_error(&error) {
                    return Err(error);
                }
                let retry_index = max_retries - retries_remaining;
                retries_remaining -= 1;
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let delay = retry_delay_from_headers(
                    &error,
                    retry_index,
                    options.max_retry_delay_ms,
                    now_ms,
                )?;
                sleep_retry(delay);
            }
        }
    }
}

pub fn provider_error_from_ureq(err: ureq::Error) -> ProviderError {
    match err {
        ureq::Error::Status(status, response) => {
            let mut headers = HashMap::new();
            for name in ["retry-after", "retry-after-ms", "x-should-retry"] {
                if let Some(value) = response.header(name) {
                    headers.insert(name.to_string(), value.to_string());
                }
            }
            let message = response
                .into_string()
                .ok()
                .filter(|body| !body.is_empty())
                .unwrap_or_else(|| format!("Provider error: {status}"));
            ProviderError {
                status: Some(status),
                headers,
                message: format!("Provider request failed: {message}"),
            }
        }
        ureq::Error::Transport(transport) => ProviderError {
            status: None,
            headers: HashMap::new(),
            message: format!("Provider request failed: {transport}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retries_retryable_provider_errors() {
        let mut calls = 0;
        let result = retry_provider_request(
            || {
                calls += 1;
                if calls == 1 {
                    Err(ProviderError::new(Some(429), "Provider error: 429")
                        .with_header("retry-after-ms", "1000"))
                } else {
                    Ok("ok")
                }
            },
            ProviderRetryOptions {
                max_retries: 1,
                max_retry_delay_ms: None,
            },
        )
        .unwrap();
        assert_eq!(result, "ok");
        assert_eq!(calls, 2);
    }

    #[test]
    fn does_not_retry_x_should_retry_false() {
        let mut calls = 0;
        let err = retry_provider_request::<(), _>(
            || {
                calls += 1;
                Err(ProviderError::new(Some(429), "no").with_header("x-should-retry", "false"))
            },
            ProviderRetryOptions {
                max_retries: 2,
                max_retry_delay_ms: None,
            },
        )
        .unwrap_err();
        assert_eq!(err.message, "no");
        assert_eq!(calls, 1);
    }

    #[test]
    fn rejects_provider_requested_delay_above_limit() {
        let err = retry_provider_request::<(), _>(
            || {
                Err(ProviderError::new(Some(429), "Provider error: 429")
                    .with_header("retry-after", "277403"))
            },
            ProviderRetryOptions {
                max_retries: 1,
                max_retry_delay_ms: Some(1000),
            },
        )
        .unwrap_err();
        assert!(err
            .message
            .contains("Server requested 277403s retry delay (max: 1s)"));
    }

    #[test]
    fn allows_disabling_retry_delay_cap() {
        let mut calls = 0;
        let result = retry_provider_request(
            || {
                calls += 1;
                if calls == 1 {
                    Err(ProviderError::new(Some(429), "Provider error: 429")
                        .with_header("retry-after", "2"))
                } else {
                    Ok("ok")
                }
            },
            ProviderRetryOptions {
                max_retries: 1,
                max_retry_delay_ms: Some(0),
            },
        )
        .unwrap();
        assert_eq!(result, "ok");
        assert_eq!(calls, 2);
    }

    #[test]
    fn parses_retry_after_http_date() {
        let then = ymd_hms_to_unix_ms(2030, 1, 1, 0, 0, 45);
        let now = ymd_hms_to_unix_ms(2030, 1, 1, 0, 0, 0);
        let error = ProviderError::new(Some(429), "wait")
            .with_header("retry-after", "Tue, 01 Jan 2030 00:00:45 GMT");
        let delay = retry_delay_from_headers(&error, 0, Some(60_000), now).unwrap();
        assert_eq!(delay, (then - now) as u64);
    }
}
