//! What a worker did, as a bounded, safe, ordered list a host can page through.
//!
//! The store keeps display summaries, not raw payloads: every summary is one
//! line, stripped of terminal control sequences, redacted for credentials and
//! capped at [`MAX_SUMMARY_BYTES`]. Each worker keeps at most
//! [`MAX_EVENTS_PER_WORKER`] of them (oldest dropped first, and counted), so a
//! chatty worker cannot grow memory without bound. Sequence numbers are
//! monotonic per store, so a host that remembers the last one it saw can ask
//! for what came after, and is told when part of that history is gone instead
//! of being handed a made-up continuation.
//!
//! Hidden reasoning never enters: callers pass tool names, arguments and
//! outcomes, not model thinking.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::ids::AgentId;

/// Recent entries kept per worker.
pub const MAX_EVENTS_PER_WORKER: usize = 200;
/// One entry's summary, in bytes.
pub const MAX_SUMMARY_BYTES: usize = 2048;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerActivityKind {
    Started,
    ToolStarted,
    ToolFinished,
    Finished,
    /// A message from the user or lead reached, or was refused by, the worker.
    Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerActivityEvent {
    /// Per store, strictly increasing.
    pub sequence: u64,
    pub agent_id: AgentId,
    pub generation: u64,
    pub timestamp_ms: i64,
    pub kind: WorkerActivityKind,
    /// Correlates a `ToolFinished` with its `ToolStarted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub summary: String,
    /// `ok` or `error` for a finished tool; the terminal state for `Finished`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}

/// One page of history after a cursor.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActivityPage {
    pub events: Vec<WorkerActivityEvent>,
    /// The cursor asked for history that has since been dropped: events
    /// between it and the first one returned are gone.
    pub gap: bool,
    /// How many of this worker's events were dropped in total.
    pub dropped: u64,
    /// Sequence of the newest event this worker has, 0 when none.
    pub latest: u64,
}

#[derive(Default)]
struct Ring {
    events: VecDeque<WorkerActivityEvent>,
    dropped: u64,
}

#[derive(Default)]
struct Inner {
    next: u64,
    rings: HashMap<AgentId, Ring>,
}

/// Shared by every clone; lives next to the registry.
#[derive(Clone, Default)]
pub struct WorkerActivityStore {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for WorkerActivityStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WorkerActivityStore")
    }
}

impl WorkerActivityStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record one entry. `summary` is made safe here; callers need not.
    /// Returns its sequence number.
    pub fn append(
        &self,
        agent_id: AgentId,
        generation: u64,
        kind: WorkerActivityKind,
        tool_call_id: Option<&str>,
        summary: &str,
        outcome: Option<&str>,
    ) -> u64 {
        let summary = safe_summary(summary);
        let timestamp_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0);
        let mut inner = self.lock();
        inner.next += 1;
        let sequence = inner.next;
        let ring = inner.rings.entry(agent_id).or_default();
        ring.events.push_back(WorkerActivityEvent {
            sequence,
            agent_id,
            generation,
            timestamp_ms,
            kind,
            tool_call_id: tool_call_id.map(safe_summary),
            summary,
            outcome: outcome.map(safe_summary),
        });
        while ring.events.len() > MAX_EVENTS_PER_WORKER {
            ring.events.pop_front();
            ring.dropped += 1;
        }
        sequence
    }

    /// Up to `max` of one worker's events with a sequence above `after`,
    /// oldest first. Never returns another worker's events.
    pub fn after(&self, agent_id: AgentId, after: u64, max: usize) -> ActivityPage {
        let inner = self.lock();
        let Some(ring) = inner.rings.get(&agent_id) else {
            return ActivityPage::default();
        };
        let latest = ring.events.back().map_or(0, |event| event.sequence);
        let oldest = ring.events.front().map_or(0, |event| event.sequence);
        // Events were dropped and the caller's cursor is older than what is left.
        // Clients can supply u64::MAX as a cursor. Saturating arithmetic
        // avoids an overflow panic in debug builds (and wraparound in release).
        let gap = ring.dropped > 0 && after.saturating_add(1) < oldest;
        ActivityPage {
            events: ring
                .events
                .iter()
                .filter(|event| event.sequence > after)
                .take(max)
                .cloned()
                .collect(),
            gap,
            dropped: ring.dropped,
            latest,
        }
    }

    /// How many events one worker currently holds.
    pub fn len(&self, agent_id: AgentId) -> usize {
        self.lock()
            .rings
            .get(&agent_id)
            .map_or(0, |ring| ring.events.len())
    }

    pub fn is_empty(&self, agent_id: AgentId) -> bool {
        self.len(agent_id) == 0
    }

    /// Forget a finished worker's history. The registry keeps its record.
    pub fn evict(&self, agent_id: AgentId) {
        self.lock().rings.remove(&agent_id);
    }

    /// Drop every worker not in `keep`.
    pub fn retain(&self, keep: impl Fn(&AgentId) -> bool) {
        self.lock().rings.retain(|id, _| keep(id));
    }
}

/// One line of plain, credential-free text no longer than
/// [`MAX_SUMMARY_BYTES`].
pub fn safe_summary(text: &str) -> String {
    let stripped = strip_terminal_controls(text);
    let redacted = super::contracts::redact_secrets(&stripped);
    let mut line = String::with_capacity(redacted.len().min(MAX_SUMMARY_BYTES));
    for ch in redacted.chars() {
        line.push(if ch == '\n' || ch == '\r' || ch == '\t' {
            ' '
        } else {
            ch
        });
    }
    if line.len() <= MAX_SUMMARY_BYTES {
        return line;
    }
    let mut cut = MAX_SUMMARY_BYTES - '…'.len_utf8();
    while !line.is_char_boundary(cut) {
        cut -= 1;
    }
    line.truncate(cut);
    line.push('…');
    line
}

/// Remove escape sequences (CSI, OSC, and the two-byte forms) and every other
/// control character except newline and tab.
pub fn strip_terminal_controls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek().copied() {
                // CSI: parameters, intermediates, one final byte.
                Some('[') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&next) {
                            break;
                        }
                    }
                }
                // OSC / DCS / PM / APC: up to BEL or ST (ESC \).
                Some(']') | Some('P') | Some('^') | Some('_') => {
                    chars.next();
                    while let Some(next) = chars.next() {
                        if next == '\u{7}' {
                            break;
                        }
                        if next == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                // Any other two-byte escape.
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\n' | '\t' => out.push(ch),
            // A lone C1 control (0x80..0x9f) is a terminal control too.
            ch if ch.is_control() => {}
            // Bidi overrides can reorder what the user reads.
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' => {}
            ch => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_and_escape_sequences_are_removed() {
        let hostile = "ok\u{1b}]52;c;ZXZpbA==\u{7}\u{1b}[2Jgo\u{8}\u{1b}\\x\r\nnext";
        let safe = safe_summary(hostile);
        assert!(!safe.chars().any(char::is_control), "{safe:?}");
        assert!(!safe.contains("ZXZpbA"), "{safe:?}");
        assert!(safe.starts_with("okgo"), "{safe:?}");
    }

    #[test]
    fn credentials_are_redacted_and_long_text_is_capped_on_a_char_boundary() {
        let safe = safe_summary("curl -H 'Authorization: Bearer abc123def456' https://x");
        assert!(!safe.contains("abc123def456"), "{safe}");
        let long = "é".repeat(5000);
        let capped = safe_summary(&long);
        assert!(capped.len() <= MAX_SUMMARY_BYTES);
        assert!(capped.ends_with('…'));
    }

    #[test]
    fn the_ring_is_bounded_and_counts_what_it_dropped() {
        let store = WorkerActivityStore::new();
        let agent = AgentId::new();
        for n in 0..10_000 {
            store.append(
                agent,
                1,
                WorkerActivityKind::ToolStarted,
                Some(&format!("c{n}")),
                &format!("Read(file{n}.rs)"),
                None,
            );
        }
        assert_eq!(store.len(agent), MAX_EVENTS_PER_WORKER);
        let page = store.after(agent, 0, 1000);
        assert_eq!(page.events.len(), MAX_EVENTS_PER_WORKER);
        assert_eq!(page.dropped, 10_000 - MAX_EVENTS_PER_WORKER as u64);
        assert_eq!(page.latest, 10_000);
        assert!(
            page.gap,
            "asking from the start reports the missing history"
        );
        assert_eq!(page.events.first().unwrap().summary, "Read(file9800.rs)");
    }

    #[test]
    fn maximum_cursor_does_not_overflow_when_history_was_dropped() {
        let store = WorkerActivityStore::new();
        let agent = AgentId::new();
        for _ in 0..=MAX_EVENTS_PER_WORKER {
            store.append(agent, 1, WorkerActivityKind::Message, None, "event", None);
        }
        assert!(store.after(agent, 0, 10).gap);
        for cursor in [u64::MAX - 1, u64::MAX] {
            let page = store.after(agent, cursor, 10);
            assert!(!page.gap, "a future cursor cannot be behind the retained history");
            assert!(page.events.is_empty());
            assert_eq!(page.dropped, 1);
        }
    }

    #[test]
    fn a_cursor_returns_only_newer_events_of_that_worker() {
        let store = WorkerActivityStore::new();
        let (a, b) = (AgentId::new(), AgentId::new());
        let first = store.append(a, 1, WorkerActivityKind::Started, None, "start a", None);
        store.append(b, 1, WorkerActivityKind::Started, None, "start b", None);
        let second = store.append(a, 1, WorkerActivityKind::ToolStarted, None, "Read(x)", None);
        let page = store.after(a, first, 10);
        assert_eq!(page.events.len(), 1);
        assert_eq!(page.events[0].sequence, second);
        assert!(!page.gap);
        assert!(store.after(a, second, 10).events.is_empty());
        // Sequences are monotonic across workers.
        assert!(second > first);
        assert!(store.after(AgentId::new(), 0, 10).events.is_empty());
    }

    #[test]
    fn a_page_is_limited_and_evicting_forgets_the_worker() {
        let store = WorkerActivityStore::new();
        let agent = AgentId::new();
        for n in 0..5 {
            store.append(
                agent,
                1,
                WorkerActivityKind::Message,
                None,
                &format!("m{n}"),
                None,
            );
        }
        assert_eq!(store.after(agent, 0, 2).events.len(), 2);
        store.evict(agent);
        assert!(store.is_empty(agent));
    }
}
