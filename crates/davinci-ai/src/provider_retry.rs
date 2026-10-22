//! Provider HTTP retries matching `vendor/pi/packages/ai/src/utils/provider-retry.ts`.

use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_MAX_RETRY_DELAY_MS: u64 = 60_000;

#[derive(Debug, Clone, Default)]
pub struct ProviderRetryOptions {
    pub max_retries: u32,
    pub max_retry_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestPhase {
    /// DNS, TCP connect, TLS: the request never reached the provider.
    Connect,
    /// Anything after the request may have been received.
    #[default]
    Response,
}

#[derive(Debug, Clone)]
pub struct ProviderError {
    pub status: Option<u16>,
    pub headers: HashMap<String, String>,
    pub message: String,
    pub phase: RequestPhase,
}

impl ProviderError {
    pub fn new(status: Option<u16>, message: impl Into<String>) -> Self {
        Self {
            status,
            headers: HashMap::new(),
            message: message.into(),
            phase: RequestPhase::Response,
        }
    }

    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers
            .insert(key.into().to_ascii_lowercase(), value.into());
        self
    }

    pub fn with_phase(mut self, phase: RequestPhase) -> Self {
        self.phase = phase;
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
    if crate::codex_usage::is_usage_limit_error(&error.message) {
        return false;
    }
    match error.header("x-should-retry") {
        Some("true") => return true,
        Some("false") => return false,
        _ => {}
    }
    match error.status {
        None => error.phase == RequestPhase::Connect,
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
        if let Some(value) = retry_after_number(retry_after_ms) {
            return validate_server_retry_delay_ms(value, max_retry_delay_ms, &error.message);
        }
    }
    if let Some(retry_after) = error.header("retry-after") {
        let delay_ms = match retry_after_number(retry_after) {
            Some(seconds) => Some(seconds * 1000.0),
            None => http_date_delay_ms(retry_after, now_ms),
        };
        // An unparseable or negative header falls through to exponential
        // backoff. A date in the past is valid and means "retry now".
        if let Some(delay_ms) = delay_ms {
            return validate_server_retry_delay_ms(delay_ms, max_retry_delay_ms, &error.message);
        }
    }
    let exponential = (0.5 * 2_f64.powi(retry_index as i32)).min(8.0) * 1000.0;
    let jitter = if cfg!(test) {
        1.0
    } else {
        1.0 - fastrand(0.25)
    };
    Ok((exponential * jitter) as u64)
}

/// A numeric Retry-After value: finite and not negative. A negative delay is
/// not a valid header value, and honoring it as zero would retry in a burst.
fn retry_after_number(value: &str) -> Option<f64> {
    value
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value >= 0.0)
}

fn http_date_delay_ms(value: &str, now_ms: i64) -> Option<f64> {
    // Avoid i64 overflow if a caller supplies an extreme clock value.
    parse_http_date_ms(value).map(|then| (i128::from(then) - i128::from(now_ms)) as f64)
}

fn parse_two_digits(value: &str) -> Option<i64> {
    if value.len() != 2 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}

fn parse_http_date_ms(value: &str) -> Option<i64> {
    // Retry-After HTTP dates use IMF-fixdate. Reject malformed timestamps
    // rather than interpreting a different timezone or rolling invalid dates
    // into another month; an invalid header uses exponential backoff.
    let (weekday, rest) = value.trim().split_once(", ")?;
    let weekday = match weekday {
        "Sun" => 0,
        "Mon" => 1,
        "Tue" => 2,
        "Wed" => 3,
        "Thu" => 4,
        "Fri" => 5,
        "Sat" => 6,
        _ => return None,
    };
    let mut parts = rest.split_whitespace();
    let day = parse_two_digits(parts.next()?)?;
    let month = month_num(parts.next()?)?;
    let year_str = parts.next()?;
    if year_str.len() != 4 || !year_str.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let year: i64 = year_str.parse().ok()?;
    let mut time = parts.next()?.split(':');
    let hour = parse_two_digits(time.next()?)?;
    let min = parse_two_digits(time.next()?)?;
    let sec = parse_two_digits(time.next()?)?;
    if time.next().is_some() || parts.next()? != "GMT" || parts.next().is_some() {
        return None;
    }
    let leap_year = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let max_day = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=max_day).contains(&day) || hour > 23 || min > 59 || sec > 59 {
        return None;
    }
    let then = ymd_hms_to_unix_ms(year, month, day, hour, min, sec);
    // 1970-01-01 was a Thursday; Sunday is index zero.
    if (then.div_euclid(86_400_000) + 4).rem_euclid(7) != weekday {
        return None;
    }
    Some(then)
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
    request: F,
    options: ProviderRetryOptions,
) -> Result<T, ProviderError>
where
    F: FnMut() -> Result<T, ProviderError>,
{
    retry_provider_request_controlled(request, options, None, sleep_retry)
}

/// Retry with interruptible, bounded wait slices. The injected wait exercises
/// the same cancellation decisions in offline fixtures as in production.
pub fn retry_provider_request_controlled<T, F, W>(
    mut request: F,
    options: ProviderRetryOptions,
    abort: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
    mut wait: W,
) -> Result<T, ProviderError>
where
    F: FnMut() -> Result<T, ProviderError>,
    W: FnMut(u64),
{
    let max_retries = options.max_retries;
    let mut retries_remaining = max_retries;
    loop {
        check_retry_abort(abort)?;
        match request() {
            Ok(value) => return Ok(value),
            Err(error) => {
                check_retry_abort(abort)?;
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
                let mut remaining = delay;
                while remaining > 0 {
                    check_retry_abort(abort)?;
                    let slice = remaining.min(25);
                    wait(slice);
                    remaining -= slice;
                }
            }
        }
    }
}

fn check_retry_abort(
    abort: Option<&std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> Result<(), ProviderError> {
    if abort.is_some_and(|signal| signal.load(std::sync::atomic::Ordering::SeqCst)) {
        Err(ProviderError::new(None, "Request aborted"))
    } else {
        Ok(())
    }
}

/// Classifies a failed send. A connection that never opened is retryable
/// (`Connect`); anything after the request left is `Response`. ureq 3 reports a
/// 4xx or 5xx as `Error::StatusCode` without a body, so callers that need the
/// body or the retry headers read the response themselves (see
/// `provider_error_from_status`).
pub fn provider_error_from_ureq(err: ureq::Error) -> ProviderError {
    match err {
        ureq::Error::StatusCode(status) => ProviderError::new(
            None,
            format!(
                "Provider HTTP {status} lost its response headers; configure \
                 http_status_as_error(false) and use provider_error_from_status \
                 before deciding whether to retry"
            ),
        ),
        transport => {
            let phase = match &transport {
                ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed
                | ureq::Error::ConnectProxyFailed(_) => RequestPhase::Connect,
                ureq::Error::Io(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                    RequestPhase::Connect
                }
                _ => RequestPhase::Response,
            };
            ProviderError {
                status: None,
                headers: HashMap::new(),
                message: format!("Provider request failed: {transport}"),
                phase,
            }
        }
    }
}

/// The provider error for an HTTP failure whose response was read: status, the
/// retry headers, and the body as the message (or `Provider error: <status>`).
pub(crate) fn provider_error_from_status(
    status: u16,
    header: impl Fn(&str) -> Option<String>,
    body: Option<String>,
) -> ProviderError {
    let mut headers = HashMap::new();
    for name in ["retry-after", "retry-after-ms", "x-should-retry"] {
        if let Some(value) = header(name) {
            headers.insert(name.to_string(), value);
        }
    }
    let message = body
        .filter(|body| !body.is_empty())
        .unwrap_or_else(|| format!("Provider error: {status}"));
    ProviderError {
        status: Some(status),
        headers,
        message: format!("Provider request failed: {message}"),
        phase: RequestPhase::Response,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headerless_status_errors_fail_closed_instead_of_guessing_a_retry() {
        let error = provider_error_from_ureq(ureq::Error::StatusCode(429));
        assert!(!is_retryable_provider_error(&error));
        assert!(error.message.contains("http_status_as_error(false)"));
        assert!(error.message.contains("provider_error_from_status"));
    }

    #[test]
    fn cancel_during_provider_backoff_sends_no_request() {
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let abort = Arc::new(AtomicBool::new(false));
        let mut calls = 0;
        let mut waited = 0;
        let result = retry_provider_request_controlled(
            || {
                calls += 1;
                Err::<(), _>(
                    ProviderError::new(Some(429), "wait").with_header("retry-after-ms", "1000"),
                )
            },
            ProviderRetryOptions {
                max_retries: 3,
                max_retry_delay_ms: None,
            },
            Some(&abort),
            |ms| {
                waited += ms;
                abort.store(true, Ordering::SeqCst);
            },
        );
        assert!(result.unwrap_err().message.contains("aborted"));
        assert_eq!(calls, 1);
        assert_eq!(waited, 25);
    }

    #[test]
    fn usage_limit_reached_is_not_retried() {
        let mut calls = 0;
        let error = retry_provider_request::<(), _>(
            || {
                calls += 1;
                Err(ProviderError::new(
                    Some(429),
                    "{\"error\":{\"type\":\"usage_limit_reached\"}}",
                ))
            },
            ProviderRetryOptions {
                max_retries: 3,
                max_retry_delay_ms: None,
            },
        )
        .unwrap_err();
        assert!(error.message.contains("usage_limit_reached"));
        assert_eq!(calls, 1);
    }

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
    fn wor79_invalid_retry_after_falls_back_to_exponential_backoff() {
        for (retry_index, expected) in [(0, 500), (1, 1000), (3, 4000)] {
            for value in ["invalid-date", "", "NaN", "Tue, 99 Foo 2030 00:00:00 GMT"] {
                let error = ProviderError::new(Some(429), "wait").with_header("retry-after", value);
                let delay = retry_delay_from_headers(&error, retry_index, Some(60_000), 0).unwrap();
                assert_eq!(delay, expected, "retry-after {value:?}");
            }
        }
    }

    /// A negative delay is not a valid header value; honoring it as zero
    /// retried in a burst against a provider that was already throttling.
    #[test]
    fn wor79_negative_or_infinite_retry_after_falls_back_to_backoff() {
        for header in ["retry-after", "retry-after-ms"] {
            for value in ["-5", "-0.5", "-1", "inf", "-inf"] {
                let error = ProviderError::new(Some(429), "wait").with_header(header, value);
                let delay = retry_delay_from_headers(&error, 1, Some(60_000), 0).unwrap();
                assert_eq!(delay, 1000, "{header}: {value:?}");
            }
        }
        // Zero is valid and still means retry now.
        let error = ProviderError::new(Some(429), "wait").with_header("retry-after", "0");
        assert_eq!(
            retry_delay_from_headers(&error, 1, Some(60_000), 0).unwrap(),
            0
        );
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

    #[test]
    fn malformed_retry_after_http_dates_use_exponential_backoff() {
        for value in [
            "Mon, 01 Jan 2030 00:00:45 GMT", // Incorrect weekday.
            "Tue, 32 Jan 2030 00:00:45 GMT",
            "Tue, 00 Jan 2030 00:00:45 GMT",
            "Tue, 29 Feb 2030 00:00:45 GMT",
            "Tue, 01 Jan 2030 24:00:45 GMT",
            "Tue, 01 Jan 2030 00:60:45 GMT",
            "Tue, 01 Jan 2030 00:00:60 GMT",
            "Tue, 01 Jan 2030 00:00:45 PST",
            "Tue, 01 Jan 2030 00:00:45",
            "Tue, 01 Jan 2030 00:00:45 GMT extra",
            "Tue, 01 Jan 20300 00:00:45 GMT",
        ] {
            let error = ProviderError::new(Some(429), "wait").with_header("retry-after", value);
            assert_eq!(
                retry_delay_from_headers(&error, 0, Some(60_000), 0).unwrap(),
                500,
                "{value:?}"
            );
        }
    }

    #[test]
    fn leap_day_and_extreme_clocks_are_handled_without_overflow() {
        let leap_day = parse_http_date_ms("Sun, 29 Feb 2032 00:00:00 GMT").unwrap();
        assert_eq!(leap_day, ymd_hms_to_unix_ms(2032, 2, 29, 0, 0, 0));
        assert!(parse_http_date_ms("Fri, 29 Feb 2030 00:00:00 GMT").is_none());

        let error = ProviderError::new(Some(429), "wait")
            .with_header("retry-after", "Tue, 01 Jan 2030 00:00:45 GMT");
        assert_eq!(
            retry_delay_from_headers(&error, 0, Some(60_000), i64::MAX).unwrap(),
            0
        );
        assert!(retry_delay_from_headers(&error, 0, Some(60_000), i64::MIN).is_err());
    }

    #[test]
    fn only_connect_phase_errors_without_status_are_retried() {
        let connect =
            ProviderError::new(None, "connection refused").with_phase(RequestPhase::Connect);
        let midway = ProviderError::new(None, "timed out reading response");
        assert!(is_retryable_provider_error(&connect));
        assert!(!is_retryable_provider_error(&midway));
        assert!(is_retryable_provider_error(&ProviderError::new(
            Some(503),
            "unavailable"
        )));
    }

    #[test]
    fn does_not_retry_a_transport_error_after_the_server_receives_the_request() {
        use std::io::Read;
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
        });

        let mut calls = 0;
        let result = retry_provider_request(
            || {
                calls += 1;
                ureq::get(&format!("http://{addr}/v1/x"))
                    .call()
                    .map(|_| ())
                    .map_err(provider_error_from_ureq)
            },
            ProviderRetryOptions {
                max_retries: 2,
                max_retry_delay_ms: Some(0),
            },
        );
        server.join().unwrap();

        let error = result.unwrap_err();
        assert_eq!(error.phase, RequestPhase::Response);
        assert_eq!(calls, 1);
    }

    #[test]
    fn a_connection_refusal_is_classified_as_retryable_connect_phase() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);

        let error = ureq::get(&format!("http://{addr}/v1/x"))
            .call()
            .unwrap_err();
        let error = provider_error_from_ureq(error);
        assert_eq!(error.phase, RequestPhase::Connect);
        assert!(is_retryable_provider_error(&error));
    }
}
