//! Device-code polling matching `vendor/pi/packages/ai/src/auth/oauth/device-code.ts`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DevicePollStatus<T> {
    Pending,
    SlowDown { interval_seconds: Option<u64> },
    Complete(T),
    Expired,
}

#[derive(Debug, Clone)]
pub struct DeviceCodePoller {
    interval_ms: u64,
    expires_at_ms: u64,
    wait_before_first: bool,
    first: bool,
}

impl DeviceCodePoller {
    pub fn new(
        interval_seconds: u64,
        expires_in_seconds: u64,
        wait_before_first: bool,
        now_ms: u64,
    ) -> Self {
        Self {
            interval_ms: interval_seconds.saturating_mul(1000),
            expires_at_ms: now_ms.saturating_add(expires_in_seconds.saturating_mul(1000)),
            wait_before_first,
            first: true,
        }
    }

    pub fn next_delay_ms(&mut self) -> Option<u64> {
        if self.first {
            self.first = false;
            if self.wait_before_first {
                return Some(self.interval_ms);
            }
            return Some(0);
        }
        Some(self.interval_ms)
    }

    pub fn expired(&self, now_ms: u64) -> bool {
        now_ms >= self.expires_at_ms
    }

    pub fn on_slow_down(&mut self, server_interval_seconds: Option<u64>) {
        self.interval_ms = match server_interval_seconds {
            Some(seconds) => seconds.saturating_mul(1000),
            None => self.interval_ms.saturating_add(5_000),
        };
    }
}

pub fn poll_oauth_device_code_flow<T, F>(
    interval_seconds: u64,
    expires_in_seconds: u64,
    wait_before_first_poll: bool,
    mut poll: F,
) -> Result<T, String>
where
    F: FnMut() -> DevicePollStatus<T>,
{
    let mut poller = DeviceCodePoller::new(
        interval_seconds,
        expires_in_seconds,
        wait_before_first_poll,
        0,
    );
    let mut now = 0_u64;
    loop {
        if poller.expired(now) {
            return Err("OAuth device-code flow expired".into());
        }
        let delay = poller.next_delay_ms().unwrap_or(0);
        now = now.saturating_add(delay);
        match poll() {
            DevicePollStatus::Complete(value) => return Ok(value),
            DevicePollStatus::Pending => {}
            DevicePollStatus::SlowDown { interval_seconds } => {
                poller.on_slow_down(interval_seconds);
            }
            DevicePollStatus::Expired => return Err("OAuth device-code flow expired".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_immediately_then_after_interval() {
        let mut poller = DeviceCodePoller::new(2, 30, false, 0);
        assert_eq!(poller.next_delay_ms(), Some(0));
        assert_eq!(poller.next_delay_ms(), Some(2000));
        poller.on_slow_down(None);
        assert_eq!(poller.next_delay_ms(), Some(7000));
    }

    #[test]
    fn wait_before_first_poll_matches_ts() {
        let mut poller = DeviceCodePoller::new(2, 30, true, 0);
        assert_eq!(poller.next_delay_ms(), Some(2000));
        let mut calls = 0;
        let token = poll_oauth_device_code_flow(2, 30, false, || {
            calls += 1;
            if calls == 1 {
                DevicePollStatus::Pending
            } else {
                DevicePollStatus::Complete("token")
            }
        })
        .unwrap();
        assert_eq!(token, "token");
        assert_eq!(calls, 2);
    }
}
