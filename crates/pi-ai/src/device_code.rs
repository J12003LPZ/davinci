//! RFC 8628 device-code polling matching TypeScript `packages/ai/src/auth/oauth/device-code.ts`.

use std::cell::Cell;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const CANCEL_MESSAGE: &str = "Login cancelled";
pub const TIMEOUT_MESSAGE: &str = "Device flow timed out";
pub const SLOW_DOWN_TIMEOUT_MESSAGE: &str = "Device flow timed out after one or more slow_down responses. This is often caused by clock drift in WSL or VM environments. Please sync or restart the VM clock and try again.";
pub const MINIMUM_INTERVAL_MS: i64 = 1000;
pub const DEFAULT_POLL_INTERVAL_SECONDS: f64 = 5.0;
pub const SLOW_DOWN_INTERVAL_INCREMENT_MS: i64 = 5000;

#[derive(Debug, Clone, PartialEq)]
pub enum DevicePollResult<T> {
    Pending,
    SlowDown { interval_seconds: Option<f64> },
    Failed { message: String },
    Complete(T),
}

#[derive(Debug, Clone, Default)]
pub struct DevicePollOptions {
    pub interval_seconds: Option<f64>,
    pub expires_in_seconds: Option<f64>,
    pub wait_before_first_poll: bool,
}

pub trait DevicePollClock {
    fn now_ms(&self) -> i64;
    fn sleep_ms(&self, ms: i64) -> Result<(), String>;
    fn aborted(&self) -> bool;
}

#[derive(Debug, Default)]
pub struct InstantClock {
    now_ms: Cell<i64>,
    aborted: Cell<bool>,
    pub sleeps: Cell<i64>,
}

impl InstantClock {
    pub fn new(now_ms: i64) -> Self {
        Self {
            now_ms: Cell::new(now_ms),
            aborted: Cell::new(false),
            sleeps: Cell::new(0),
        }
    }

    pub fn abort(&self) {
        self.aborted.set(true);
    }
}

impl DevicePollClock for InstantClock {
    fn now_ms(&self) -> i64 {
        self.now_ms.get()
    }

    fn sleep_ms(&self, ms: i64) -> Result<(), String> {
        if self.aborted.get() {
            return Err(CANCEL_MESSAGE.into());
        }
        self.sleeps.set(self.sleeps.get() + 1);
        self.now_ms.set(self.now_ms.get() + ms.max(0));
        if self.aborted.get() {
            return Err(CANCEL_MESSAGE.into());
        }
        Ok(())
    }

    fn aborted(&self) -> bool {
        self.aborted.get()
    }
}

pub struct RealClock;

impl DevicePollClock for RealClock {
    fn now_ms(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    fn sleep_ms(&self, ms: i64) -> Result<(), String> {
        if self.aborted() {
            return Err(CANCEL_MESSAGE.into());
        }
        let ms = sleep_override_ms().unwrap_or(ms.max(0) as u64);
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        if self.aborted() {
            return Err(CANCEL_MESSAGE.into());
        }
        Ok(())
    }

    fn aborted(&self) -> bool {
        false
    }
}

pub fn sleep_override_ms() -> Option<u64> {
    std::env::var("PI_OAUTH_DEVICE_SLEEP_MS")
        .ok()
        .and_then(|value| value.parse().ok())
}

pub fn use_instant_device_clock() -> bool {
    sleep_override_ms() == Some(0)
        || std::env::var("PI_OAUTH_DEVICE_FIXTURE")
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
        || std::env::var("PI_OAUTH_DEVICE_POLL_FIXTURE")
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
        || std::env::var("PI_OAUTH_TOKEN_FIXTURE")
            .ok()
            .is_some_and(|v| !v.trim().is_empty())
}

pub fn default_device_clock() -> Box<dyn DevicePollClock> {
    if use_instant_device_clock() {
        Box::new(InstantClock::new(0))
    } else {
        Box::new(RealClock)
    }
}

fn interval_ms(interval_seconds: Option<f64>) -> i64 {
    let seconds = interval_seconds.unwrap_or(DEFAULT_POLL_INTERVAL_SECONDS);
    MINIMUM_INTERVAL_MS.max(seconds.floor() as i64 * 1000)
}

fn apply_slow_down(current_ms: i64, interval_seconds: Option<f64>) -> i64 {
    match interval_seconds {
        Some(seconds) if seconds.is_finite() && seconds > 0.0 => {
            MINIMUM_INTERVAL_MS.max(seconds.floor() as i64 * 1000)
        }
        _ => MINIMUM_INTERVAL_MS.max(current_ms + SLOW_DOWN_INTERVAL_INCREMENT_MS),
    }
}

pub fn poll_oauth_device_code_flow<T, F>(
    options: DevicePollOptions,
    clock: &dyn DevicePollClock,
    mut poll: F,
) -> Result<T, String>
where
    F: FnMut() -> Result<DevicePollResult<T>, String>,
{
    let deadline = match options.expires_in_seconds {
        Some(seconds) if seconds.is_finite() => clock.now_ms() + (seconds * 1000.0) as i64,
        _ => i64::MAX,
    };
    let mut interval = interval_ms(options.interval_seconds);
    let mut slow_down_responses = 0;

    if options.wait_before_first_poll {
        let remaining = deadline - clock.now_ms();
        if remaining > 0 {
            clock.sleep_ms(interval.min(remaining))?;
        }
    }

    while clock.now_ms() < deadline {
        if clock.aborted() {
            return Err(CANCEL_MESSAGE.into());
        }
        match poll()? {
            DevicePollResult::Complete(value) => return Ok(value),
            DevicePollResult::Failed { message } => return Err(message),
            DevicePollResult::Pending => {}
            DevicePollResult::SlowDown { interval_seconds } => {
                slow_down_responses += 1;
                interval = apply_slow_down(interval, interval_seconds);
            }
        }
        let remaining = deadline - clock.now_ms();
        if remaining <= 0 {
            break;
        }
        clock.sleep_ms(interval.min(remaining))?;
    }

    Err(if slow_down_responses > 0 {
        SLOW_DOWN_TIMEOUT_MESSAGE.into()
    } else {
        TIMEOUT_MESSAGE.into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_immediately_and_returns_completed_value() {
        let clock = InstantClock::new(0);
        let mut polls = 0;
        let value = poll_oauth_device_code_flow(
            DevicePollOptions {
                interval_seconds: Some(2.0),
                expires_in_seconds: Some(30.0),
                wait_before_first_poll: false,
            },
            &clock,
            || {
                polls += 1;
                if polls == 1 {
                    Ok(DevicePollResult::Pending)
                } else {
                    Ok(DevicePollResult::Complete("token"))
                }
            },
        )
        .unwrap();
        assert_eq!(value, "token");
        assert_eq!(polls, 2);
        assert_eq!(clock.now_ms(), 2000);
    }

    #[test]
    fn can_wait_before_the_first_poll() {
        let clock = InstantClock::new(0);
        let mut polls = 0;
        let value = poll_oauth_device_code_flow(
            DevicePollOptions {
                interval_seconds: Some(2.0),
                expires_in_seconds: Some(30.0),
                wait_before_first_poll: true,
            },
            &clock,
            || {
                polls += 1;
                Ok(DevicePollResult::Complete("token"))
            },
        )
        .unwrap();
        assert_eq!(value, "token");
        assert_eq!(polls, 1);
        assert_eq!(clock.now_ms(), 2000);
    }

    #[test]
    fn increases_interval_by_five_seconds_after_slow_down() {
        let clock = InstantClock::new(0);
        let mut polls = 0;
        let value = poll_oauth_device_code_flow(
            DevicePollOptions {
                interval_seconds: Some(2.0),
                expires_in_seconds: Some(900.0),
                wait_before_first_poll: false,
            },
            &clock,
            || {
                polls += 1;
                if polls == 1 {
                    Ok(DevicePollResult::SlowDown {
                        interval_seconds: None,
                    })
                } else {
                    Ok(DevicePollResult::Complete("token"))
                }
            },
        )
        .unwrap();
        assert_eq!(value, "token");
        assert_eq!(clock.now_ms(), 7000);
    }

    #[test]
    fn honors_server_provided_slow_down_interval() {
        let clock = InstantClock::new(0);
        let mut polls = 0;
        let value = poll_oauth_device_code_flow(
            DevicePollOptions {
                interval_seconds: Some(2.0),
                expires_in_seconds: Some(900.0),
                wait_before_first_poll: false,
            },
            &clock,
            || {
                polls += 1;
                if polls == 1 {
                    Ok(DevicePollResult::SlowDown {
                        interval_seconds: Some(30.0),
                    })
                } else {
                    Ok(DevicePollResult::Complete("token"))
                }
            },
        )
        .unwrap();
        assert_eq!(value, "token");
        assert_eq!(clock.now_ms(), 30_000);
    }

    #[test]
    fn cancels_an_in_flight_wait() {
        let clock = InstantClock::new(0);
        clock.abort();
        let err = poll_oauth_device_code_flow(
            DevicePollOptions {
                interval_seconds: Some(5.0),
                expires_in_seconds: Some(30.0),
                wait_before_first_poll: false,
            },
            &clock,
            || Ok(DevicePollResult::<&str>::Pending),
        )
        .unwrap_err();
        assert_eq!(err, CANCEL_MESSAGE);
    }

    #[test]
    fn timeout_message_mentions_slow_down_when_seen() {
        let clock = InstantClock::new(0);
        let err = poll_oauth_device_code_flow(
            DevicePollOptions {
                interval_seconds: Some(1.0),
                expires_in_seconds: Some(1.0),
                wait_before_first_poll: false,
            },
            &clock,
            || {
                Ok(DevicePollResult::<&str>::SlowDown {
                    interval_seconds: None,
                })
            },
        )
        .unwrap_err();
        assert_eq!(err, SLOW_DOWN_TIMEOUT_MESSAGE);
    }
}
