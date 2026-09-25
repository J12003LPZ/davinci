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
        EffortSignals { mutations, consecutive_failures }
    }

    #[test]
    fn fixed_always_sends_the_configured_level() {
        assert_eq!(request_level(EffortPolicy::Fixed, Medium, signals(0, 5)), Medium);
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
        assert_eq!(EffortPolicy::parse(" Adaptive "), Some(EffortPolicy::Adaptive));
        assert_eq!(EffortPolicy::parse("fixed"), Some(EffortPolicy::Fixed));
        assert_eq!(EffortPolicy::parse("fast"), None);
    }
}
