//! Per-request reasoning effort. No TypeScript counterpart.
//!
//! `Fixed` sends the configured level on every request. `Adaptive` sends one
//! level lower while the turn has made no file change yet (reading and
//! searching), the configured level once it edits, and one level higher after
//! two failed tool results in a row. `Off` is never raised.

use davinci_protocol::ThinkingLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EffortPolicy {
    #[default]
    Fixed,
    Adaptive,
}

impl EffortPolicy {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fixed" => Some(Self::Fixed),
            "adaptive" => Some(Self::Adaptive),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EffortSignals {
    pub mutations: u64,
    pub consecutive_failures: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffortSource {
    Fixed,
    Deterministic,
    JevAdvice,
}

impl EffortSignals {
    pub(crate) fn observe(&mut self, tool: Option<&str>, error: bool) {
        if error {
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        } else {
            self.consecutive_failures = 0;
            if tool.is_some_and(crate::tools::is_coordinated_mutation) {
                self.mutations = self.mutations.saturating_add(1);
            }
        }
    }
}

pub fn request_level(
    policy: EffortPolicy,
    base: ThinkingLevel,
    signals: EffortSignals,
) -> ThinkingLevel {
    match policy {
        EffortPolicy::Fixed => base,
        EffortPolicy::Adaptive if signals.consecutive_failures >= 2 => step_up(base),
        EffortPolicy::Adaptive if signals.mutations == 0 => step_down(base),
        EffortPolicy::Adaptive => base,
    }
}

/// Resolve a request effort without allowing optional advice to lower the
/// deterministic choice or override an explicit fixed/off policy.
pub fn resolve_request_effort(
    policy: EffortPolicy,
    base: ThinkingLevel,
    signals: EffortSignals,
    advice: Option<ThinkingLevel>,
) -> (ThinkingLevel, EffortSource) {
    if base == ThinkingLevel::Off || policy == EffortPolicy::Fixed {
        return (request_level(policy, base, signals), EffortSource::Fixed);
    }
    let deterministic = request_level(policy, base, signals);
    let Some(advice) = advice else {
        return (deterministic, EffortSource::Deterministic);
    };
    if level_rank(advice) > level_rank(deterministic) {
        (advice, EffortSource::JevAdvice)
    } else {
        (deterministic, EffortSource::Deterministic)
    }
}

pub fn favor_high_effort(level: ThinkingLevel) -> ThinkingLevel {
    step_up(level)
}

fn level_rank(level: ThinkingLevel) -> u8 {
    match level {
        ThinkingLevel::Off => 0,
        ThinkingLevel::Minimal => 1,
        ThinkingLevel::Low => 2,
        ThinkingLevel::Medium => 3,
        ThinkingLevel::High => 4,
        ThinkingLevel::Xhigh => 5,
        ThinkingLevel::Max => 6,
    }
}

fn step_up(level: ThinkingLevel) -> ThinkingLevel {
    use ThinkingLevel::*;
    match level {
        Off => Off,
        Minimal => Low,
        Low => Medium,
        Medium => High,
        High => Xhigh,
        Xhigh | Max => Max,
    }
}

fn step_down(level: ThinkingLevel) -> ThinkingLevel {
    use ThinkingLevel::*;
    match level {
        Off => Off,
        Minimal => Minimal,
        Low | Medium => Low,
        High => Medium,
        Xhigh => High,
        Max => Xhigh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_protocol::ThinkingLevel::*;

    fn signals(mutations: u64, consecutive_failures: u32) -> EffortSignals {
        EffortSignals {
            mutations,
            consecutive_failures,
        }
    }

    #[test]
    fn fixed_always_sends_the_configured_level() {
        assert_eq!(
            request_level(EffortPolicy::Fixed, Medium, signals(0, 5)),
            Medium
        );
    }

    #[test]
    fn adaptive_reads_lower_edits_at_base_and_climbs_after_failures() {
        let p = EffortPolicy::Adaptive;
        assert_eq!(request_level(p, Medium, signals(0, 0)), Low);
        assert_eq!(request_level(p, Medium, signals(1, 0)), Medium);
        assert_eq!(request_level(p, Medium, signals(1, 2)), High);
        assert_eq!(request_level(p, Low, signals(0, 0)), Low);
        assert_eq!(request_level(p, Off, signals(0, 3)), Off);
    }

    #[test]
    fn parse_accepts_known_names_only() {
        assert_eq!(
            EffortPolicy::parse(" Adaptive "),
            Some(EffortPolicy::Adaptive)
        );
        assert_eq!(EffortPolicy::parse("fixed"), Some(EffortPolicy::Fixed));
        assert_eq!(EffortPolicy::parse("fast"), None);
    }

    #[test]
    fn advice_can_only_raise_adaptive_effort() {
        let (level, source) =
            resolve_request_effort(EffortPolicy::Adaptive, Medium, signals(0, 0), Some(High));
        assert_eq!((level, source), (High, EffortSource::JevAdvice));

        let (level, source) =
            resolve_request_effort(EffortPolicy::Adaptive, Medium, signals(1, 0), Some(Low));
        assert_eq!((level, source), (Medium, EffortSource::Deterministic));
    }

    #[test]
    fn fixed_and_off_ignore_advice() {
        assert_eq!(
            resolve_request_effort(EffortPolicy::Fixed, Medium, signals(0, 0), Some(Max)),
            (Medium, EffortSource::Fixed)
        );
        assert_eq!(
            resolve_request_effort(EffortPolicy::Adaptive, Off, signals(0, 0), Some(Max)),
            (Off, EffortSource::Fixed)
        );
    }
}
