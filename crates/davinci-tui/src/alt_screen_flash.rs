//! Transient flash stack matching `vendor/pi/packages/tui/src/components/alt-screen-flash.ts`.

use crate::ansi::truncate_to_width;
use crate::render::Component;

const DEFAULT_DURATION_MS: u64 = 1000;

struct FlashEntry {
    #[allow(dead_code)]
    id: u64,
    message: String,
    hide_at_ms: u64,
}

pub struct AltScreenFlashContainer {
    entries: Vec<FlashEntry>,
    next_id: u64,
    now_ms: u64,
}

impl AltScreenFlashContainer {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 0,
            now_ms: 0,
        }
    }

    pub fn flash(&mut self, message: impl Into<String>, duration_ms: Option<u64>) {
        let id = self.next_id;
        self.next_id += 1;
        let duration = duration_ms.unwrap_or(DEFAULT_DURATION_MS);
        self.entries.push(FlashEntry {
            id,
            message: message.into(),
            hide_at_ms: self.now_ms.saturating_add(duration),
        });
        let _ = id;
    }

    pub fn tick(&mut self, ms: u64) -> bool {
        self.now_ms = self.now_ms.saturating_add(ms);
        let before = self.entries.len();
        self.entries.retain(|entry| self.now_ms < entry.hide_at_ms);
        before != self.entries.len()
    }

    pub fn dispose(&mut self) {
        self.entries.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for AltScreenFlashContainer {
    fn default() -> Self {
        Self::new()
    }
}

impl Component for AltScreenFlashContainer {
    fn render(&self, width: usize) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| {
                let message = truncate_to_width(&format!(" {} ", entry.message), width, "", false);
                format!("\x1b[7m{message}\x1b[27m")
            })
            .collect()
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flash_expires_on_tick() {
        let mut flashes = AltScreenFlashContainer::new();
        flashes.flash("Copied!", Some(1000));
        assert!(flashes.render(20)[0].contains("Copied!"));
        assert!(!flashes.tick(999));
        assert!(flashes.tick(2));
        assert!(flashes.is_empty());
    }
}
