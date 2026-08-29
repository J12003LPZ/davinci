use crate::render::Component;

/// ScrollView matching `vendor/pi/packages/tui/src/components/scroll-view.ts`.
pub struct ScrollView<T: Component> {
    pub child: T,
    pub scroll_top: usize,
    pub follow_end: bool,
    pub scrollbar: &'static str,
}

impl<T: Component> ScrollView<T> {
    pub fn new(child: T) -> Self {
        Self {
            child,
            scroll_top: 0,
            follow_end: false,
            scrollbar: "hidden",
        }
    }

    pub fn scroll_to(&mut self, top: usize) {
        self.scroll_top = top;
        self.follow_end = false;
    }
}

impl<T: Component> Component for ScrollView<T> {
    fn render(&self, width: usize) -> Vec<String> {
        let lines = self.child.render(width);
        if self.scroll_top == 0 {
            lines
        } else {
            lines.into_iter().skip(self.scroll_top).collect()
        }
    }

    fn handle_input(&mut self, data: &str) {
        self.child.handle_input(data);
    }

    fn invalidate(&mut self) {
        self.child.invalidate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::Text;

    #[test]
    fn skips_scrolled_lines() {
        let mut view = ScrollView::new(Text {
            value: "a\nb\nc".into(),
        });
        view.scroll_to(1);
        let lines = view.render(10);
        assert_eq!(lines[0], "b");
    }
}
