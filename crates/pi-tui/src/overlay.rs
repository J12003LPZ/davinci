use crate::render::Component;

/// Overlay host matching TS `OverlayHandle` / overlay stack.
pub struct Overlay {
    pub title: String,
    child: Box<dyn Component>,
}

impl Overlay {
    pub fn new(title: impl Into<String>, child: Box<dyn Component>) -> Self {
        Self {
            title: title.into(),
            child,
        }
    }
}

impl Component for Overlay {
    fn render(&self, width: usize) -> Vec<String> {
        let inner = width.saturating_sub(2);
        let mut lines = vec![format!("┌{}┐", "─".repeat(inner))];
        let title = format!(" {} ", self.title);
        lines.push(format!("│{title:<inner$}│"));
        for line in self.child.render(inner.saturating_sub(2)) {
            let mut padded = format!("│ {line}");
            while crate::render::visible_width(&padded) < width.saturating_sub(1) {
                padded.push(' ');
            }
            padded.push('│');
            lines.push(padded);
        }
        lines.push(format!("└{}┘", "─".repeat(inner)));
        lines
    }

    fn handle_input(&mut self, data: &str) {
        self.child.handle_input(data);
    }

    fn invalidate(&mut self) {
        self.child.invalidate();
    }
}
