//! Spacer matching `vendor/pi/packages/tui/src/components/spacer.ts`.

use crate::render::Component;

pub struct Spacer {
    lines: usize,
}

impl Spacer {
    pub fn new(lines: usize) -> Self {
        Self { lines }
    }

    pub fn set_lines(&mut self, lines: usize) {
        self.lines = lines;
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self::new(1)
    }
}

impl Component for Spacer {
    fn render(&self, _width: usize) -> Vec<String> {
        vec![String::new(); self.lines]
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacer_renders_empty_lines() {
        assert_eq!(Spacer::new(0).render(10), Vec::<String>::new());
        assert_eq!(Spacer::default().render(10), vec![""]);
        assert_eq!(Spacer::new(3).render(4), vec!["", "", ""]);
    }
}
