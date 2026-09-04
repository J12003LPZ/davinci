//! Emacs-style kill ring matching TS `kill-ring.ts`.

#[derive(Debug, Clone, Default)]
pub struct KillRing {
    ring: Vec<String>,
}

impl KillRing {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, text: &str, prepend: bool, accumulate: bool) {
        if text.is_empty() {
            return;
        }
        if accumulate && !self.ring.is_empty() {
            let last = self.ring.pop().expect("nonempty");
            self.ring.push(if prepend {
                format!("{text}{last}")
            } else {
                format!("{last}{text}")
            });
        } else {
            self.ring.push(text.to_string());
        }
    }

    pub fn peek(&self) -> Option<&str> {
        self.ring.last().map(String::as_str)
    }

    pub fn rotate(&mut self) {
        if self.ring.len() > 1 {
            let last = self.ring.pop().expect("len > 1");
            self.ring.insert(0, last);
        }
    }

    pub fn len(&self) -> usize {
        self.ring.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn push_peek_accumulate_and_rotate_match_ts() {
        let mut ring = KillRing::new();
        ring.push("world", false, false);
        ring.push("hello ", true, true);
        assert_eq!(ring.peek(), Some("hello world"));
        ring.push("!", false, false);
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.peek(), Some("!"));
        ring.rotate();
        assert_eq!(ring.peek(), Some("hello world"));
        ring.rotate();
        assert_eq!(ring.peek(), Some("!"));
        ring.push("", false, false);
        assert_eq!(ring.len(), 2);
    }
}
