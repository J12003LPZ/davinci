//! Fallback for Windows console hosts that discard bracketed-paste markers.
//! Ordinary typing waits at most 20ms. A burst stays a draft through a 500ms
//! quiet window, so its embedded Enter events cannot submit the composer.
use std::time::{Duration, Instant};

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

pub(super) const TYPING_WAIT: Duration = Duration::from_millis(20);
const PASTE_WAIT: Duration = Duration::from_millis(500);
const MAX_BUFFERED_KEYS: usize = 65_536;

#[derive(Debug, Default)]
pub(super) struct PasteBurst {
    keys: Vec<KeyEvent>,
    last: Option<Instant>,
    pasting: bool,
}

impl PasteBurst {
    pub fn pending(&self) -> bool {
        !self.keys.is_empty()
    }

    pub fn feed(&mut self, event: Event, now: Instant) -> Vec<Event> {
        let mut ready = self.idle(now);
        let Event::Key(key) = event else {
            ready.extend(self.flush());
            ready.push(event);
            return ready;
        };
        if key.kind == KeyEventKind::Release {
            return ready;
        }
        let printable = matches!(key.code, KeyCode::Char(_))
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
        let whitespace = matches!(key.code, KeyCode::Enter | KeyCode::Tab);
        if printable || whitespace {
            self.keys.push(key);
            self.last = Some(now);
            let trailing_short_enter = self.keys.len() < 8
                && matches!(self.keys.last().map(|key| key.code), Some(KeyCode::Enter));
            self.pasting |= self.keys.len() >= 8
                || (self.keys.len() > 1
                    && !trailing_short_enter
                    && self
                        .keys
                        .iter()
                        .any(|key| matches!(key.code, KeyCode::Enter | KeyCode::Tab)));
            if self.keys.len() == MAX_BUFFERED_KEYS {
                ready.extend(self.flush());
                self.pasting = true;
                self.last = Some(now);
            }
        } else {
            ready.extend(self.flush());
            ready.push(Event::Key(key));
        }
        ready
    }

    pub fn idle(&mut self, now: Instant) -> Vec<Event> {
        let wait = if self.pasting {
            PASTE_WAIT
        } else {
            TYPING_WAIT
        };
        if self
            .last
            .is_some_and(|last| now.duration_since(last) >= wait)
        {
            self.flush()
        } else {
            Vec::new()
        }
    }

    pub fn flush(&mut self) -> Vec<Event> {
        self.last = None;
        let keys = std::mem::take(&mut self.keys);
        if std::mem::take(&mut self.pasting) {
            let text = keys
                .into_iter()
                .filter_map(|key| match key.code {
                    KeyCode::Char(ch) => Some(ch),
                    KeyCode::Enter => Some('\n'),
                    KeyCode::Tab => Some('\t'),
                    _ => None,
                })
                .collect();
            vec![Event::Paste(text)]
        } else {
            keys.into_iter().map(Event::Key).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn delayed_console_chunks_and_newlines_stay_one_paste() {
        let mut burst = PasteBurst::default();
        let start = Instant::now();
        for ch in "abcdefgh".chars() {
            assert!(burst.feed(key(KeyCode::Char(ch)), start).is_empty());
        }
        assert!(burst.idle(start + Duration::from_millis(300)).is_empty());
        let second = start + Duration::from_millis(300);
        assert!(burst.feed(key(KeyCode::Enter), second).is_empty());
        assert!(burst.feed(key(KeyCode::Char('界')), second).is_empty());
        assert_eq!(
            burst.idle(second + PASTE_WAIT),
            vec![Event::Paste("abcdefgh\n界".into())]
        );
        assert!(burst
            .feed(key(KeyCode::Enter), second + PASTE_WAIT)
            .is_empty());
        assert_eq!(
            burst.idle(second + PASTE_WAIT + TYPING_WAIT),
            vec![key(KeyCode::Enter)]
        );
    }

    #[test]
    fn ordinary_typing_and_short_multiline_paste() {
        let start = Instant::now();
        let mut burst = PasteBurst::default();
        assert!(burst.feed(key(KeyCode::Char('a')), start).is_empty());
        assert_eq!(
            burst.idle(start + TYPING_WAIT),
            vec![key(KeyCode::Char('a'))]
        );
        assert!(burst
            .feed(key(KeyCode::Enter), start + TYPING_WAIT)
            .is_empty());
        assert_eq!(
            burst.idle(start + TYPING_WAIT * 2),
            vec![key(KeyCode::Enter)]
        );
        burst.feed(key(KeyCode::Enter), start + TYPING_WAIT * 2);
        burst.feed(key(KeyCode::Char('b')), start + TYPING_WAIT * 2);
        assert_eq!(
            burst.idle(start + TYPING_WAIT * 2 + PASTE_WAIT),
            vec![Event::Paste("\nb".into())]
        );
    }

    #[test]
    fn short_typed_burst_ending_in_enter_still_submits() {
        let start = Instant::now();
        let mut burst = PasteBurst::default();
        assert!(burst.feed(key(KeyCode::Char('o')), start).is_empty());
        assert!(burst.feed(key(KeyCode::Char('k')), start).is_empty());
        assert!(burst.feed(key(KeyCode::Enter), start).is_empty());
        assert_eq!(
            burst.idle(start + TYPING_WAIT),
            vec![
                key(KeyCode::Char('o')),
                key(KeyCode::Char('k')),
                key(KeyCode::Enter),
            ]
        );
    }

    #[test]
    fn bounded_chunks_and_control_c_never_replay_pasted_enter() {
        let start = Instant::now();
        let mut burst = PasteBurst::default();
        for _ in 0..MAX_BUFFERED_KEYS - 1 {
            assert!(burst.feed(key(KeyCode::Char('x')), start).is_empty());
        }
        assert!(
            matches!(&burst.feed(key(KeyCode::Enter), start)[..], [Event::Paste(text)] if text.ends_with('\n'))
        );
        assert!(burst.feed(key(KeyCode::Enter), start).is_empty());
        let cancel = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(
            burst.feed(cancel.clone(), start),
            vec![Event::Paste("\n".into()), cancel]
        );
    }
}
