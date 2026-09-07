//! Pure monotonic-clock dictation reducer. Effects cannot submit or queue text.

use crate::protocol::VoiceError;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub session: u64,
    pub epoch: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Disabled,
    NeedsSetup,
    #[default]
    Idle,
    Preparing,
    Listening,
    Stopping,
    Transcribing,
    Cancelling,
    Inserted,
    Error,
}

impl Phase {
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Listening
                | Self::Stopping
                | Self::Transcribing
                | Self::Cancelling
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Prepare(Identity),
    Start(Identity),
    Stop(u64),
    Cancel(u64),
    Kill,
    Insert { epoch: u64, text: String },
    OpenSetup,
}

#[derive(Debug, Default, Clone)]
pub struct Controller {
    pub phase: Phase,
    pub identity: Option<Identity>,
    pub error: Option<VoiceError>,
    next_id: u64,
    last_toggle: Option<u64>,
    deadline: u64,
    capture_at: u64,
    insert_at: Option<u64>,
    insertion_drawn: bool,
    kill_requested: bool,
    start_requested: bool,
}

impl Controller {
    pub fn toggle(&mut self, now: u64, epoch: u64) -> Option<Effect> {
        if self
            .last_toggle
            .is_some_and(|last| now.saturating_sub(last) < 250)
        {
            return None;
        }
        match self.phase {
            Phase::NeedsSetup => Some(Effect::OpenSetup),
            Phase::Idle | Phase::Inserted | Phase::Error => {
                self.last_toggle = Some(now);
                self.next_id = self.next_id.checked_add(1)?;
                let id = Identity {
                    session: self.next_id,
                    epoch,
                };
                self.identity = Some(id);
                self.error = None;
                self.phase = Phase::Preparing;
                self.deadline = now.saturating_add(30_000);
                self.kill_requested = false;
                self.start_requested = false;
                Some(Effect::Prepare(id))
            }
            Phase::Listening => {
                self.last_toggle = Some(now);
                self.stop(now)
            }
            _ => None,
        }
    }

    pub fn ready(&mut self, session: u64, epoch: u64, now: u64) -> Option<Effect> {
        if self.start_requested
            || self.phase != Phase::Preparing
            || self.identity != Some(Identity { session, epoch })
            || now >= self.deadline
        {
            return None;
        }
        self.start_requested = true;
        Some(Effect::Start(self.identity?))
    }

    fn is_session(&self, session: u64) -> bool {
        self.identity.is_some_and(|id| id.session == session)
    }

    pub fn capture_started(&mut self, session: u64, now: u64) {
        if self.is_session(session) && self.phase == Phase::Preparing && now < self.deadline {
            self.phase = Phase::Listening;
            self.capture_at = now;
            self.deadline = now.saturating_add(120_000);
        }
    }

    fn stop(&mut self, now: u64) -> Option<Effect> {
        self.phase = Phase::Stopping;
        self.deadline = now.saturating_add(1_000);
        Some(Effect::Stop(self.identity?.session))
    }

    pub fn capture_stopped(&mut self, session: u64, now: u64) {
        if self.is_session(session) && matches!(self.phase, Phase::Listening | Phase::Stopping) {
            self.phase = Phase::Transcribing;
            let duration = now.saturating_sub(self.capture_at).min(120_000);
            self.deadline = now.saturating_add(duration.saturating_mul(3).clamp(60_000, 360_000));
        }
    }

    pub fn complete(&mut self, session: u64, epoch: u64, text: String, now: u64) -> Option<Effect> {
        if self.phase != Phase::Transcribing
            || self.identity != Some(Identity { session, epoch })
            || now >= self.deadline
        {
            return None;
        }
        self.identity = None;
        self.phase = Phase::Inserted;
        self.insert_at = Some(now);
        self.insertion_drawn = false;
        Some(Effect::Insert { epoch, text })
    }

    pub fn cancel(&mut self, now: u64) -> Option<Effect> {
        if !self.phase.active() || self.phase == Phase::Cancelling {
            return None;
        }
        self.phase = Phase::Cancelling;
        self.deadline = now.saturating_add(750);
        self.kill_requested = false;
        Some(Effect::Cancel(self.identity?.session))
    }

    /// Called only after capture closure acknowledgement or process reaping.
    pub fn closed(&mut self) {
        self.identity = None;
        self.phase = if self.error.is_some() {
            Phase::Error
        } else {
            Phase::Idle
        };
    }

    pub fn failed(&mut self, error: VoiceError) {
        self.error = Some(error);
        self.closed();
    }

    pub fn tick(&mut self, now: u64) -> Option<Effect> {
        if self.phase == Phase::Inserted
            && self
                .insert_at
                .is_some_and(|t| now.saturating_sub(t) >= 2_000)
        {
            self.phase = Phase::Idle;
        }
        if now < self.deadline {
            return None;
        }
        match self.phase {
            Phase::Listening => self.stop(now),
            Phase::Preparing | Phase::Stopping | Phase::Transcribing => {
                self.error = Some(VoiceError::Timeout);
                self.cancel(now)
            }
            Phase::Cancelling if !self.kill_requested => {
                self.kill_requested = true;
                Some(Effect::Kill)
            }
            _ => None,
        }
    }

    pub fn blocks_send(&self, now: u64) -> bool {
        self.phase.active()
            || self
                .insert_at
                .is_some_and(|t| !self.insertion_drawn || now.saturating_sub(t) < 250)
    }

    pub fn drawn(&mut self) {
        self.insertion_drawn = true;
    }
    pub fn elapsed_seconds(&self, now: u64) -> u64 {
        now.saturating_sub(self.capture_at) / 1_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listening() -> Controller {
        let mut c = Controller::default();
        assert_eq!(
            c.toggle(0, 4),
            Some(Effect::Prepare(Identity {
                session: 1,
                epoch: 4
            }))
        );
        assert_eq!(c.ready(1, 4, 1), Some(Effect::Start(c.identity.unwrap())));
        c.capture_started(1, 2);
        assert_eq!(c.phase, Phase::Listening);
        c
    }

    #[test]
    fn stop_completion_is_single_use_and_guard_requires_draw() {
        let mut c = listening();
        assert!(c.toggle(200, 4).is_none());
        assert_eq!(c.toggle(300, 4), Some(Effect::Stop(1)));
        assert!(c.complete(1, 4, "hello".into(), 301).is_none());
        c.capture_stopped(1, 302);
        assert_eq!(
            c.complete(1, 4, "hello".into(), 303),
            Some(Effect::Insert {
                epoch: 4,
                text: "hello".into()
            })
        );
        assert!(c.complete(1, 4, "duplicate".into(), 304).is_none());
        assert!(c.blocks_send(900));
        c.drawn();
        assert!(!c.blocks_send(900));
    }

    #[test]
    fn cancel_invalidates_result_until_capture_closes() {
        let mut c = listening();
        assert_eq!(c.cancel(20), Some(Effect::Cancel(1)));
        assert!(c.complete(1, 4, "late".into(), 21).is_none());
        assert!(c.toggle(1000, 4).is_none());
        assert_eq!(c.tick(770), Some(Effect::Kill));
        assert_eq!(c.phase, Phase::Cancelling);
        c.closed();
        assert_eq!(c.phase, Phase::Idle);
        assert!(c.toggle(1001, 5).is_some());
        assert_eq!(
            c.identity.unwrap(),
            Identity {
                session: 2,
                epoch: 5
            }
        );
    }

    #[test]
    fn maximum_duration_and_stale_epoch() {
        let mut c = listening();
        assert_eq!(c.tick(120_002), Some(Effect::Stop(1)));
        c.capture_stopped(1, 120_003);
        assert!(c.complete(1, 5, "stale".into(), 120_004).is_none());
        assert!(c.blocks_send(120_005));
    }

    #[test]
    fn preparing_expires_without_delayed_capture() {
        let mut c = Controller::default();
        c.toggle(0, 1);
        assert!(matches!(c.tick(30_000), Some(Effect::Cancel(1))));
        assert!(c.ready(1, 1, 30_001).is_none());
    }

    #[test]
    fn duplicate_ready_and_expired_completion_are_ignored() {
        let mut c = listening();
        assert!(c.ready(1, 4, 3).is_none());
        c.capture_stopped(1, 100);
        assert!(c.complete(1, 4, "late".into(), 60_100).is_none());
        let mut c = Controller::default();
        c.toggle(0, 1);
        assert!(c.ready(1, 1, 1).is_some());
        assert!(c.ready(1, 1, 2).is_none());
        c.capture_started(1, 30_000);
        assert_eq!(c.phase, Phase::Preparing);
    }
}
