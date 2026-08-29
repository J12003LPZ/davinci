//! ScrollView matching `vendor/pi/packages/tui/src/components/scroll-view.ts`.

use std::rc::Rc;

use crate::render::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollViewScrollbar {
    Hidden,
    Auto,
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollFollow {
    None,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollOverscroll {
    Chain,
    Contain,
}

pub struct ScrollViewOptions {
    pub axis: Option<String>,
    pub follow: ScrollFollow,
    pub primary: bool,
    pub overscroll: ScrollOverscroll,
    pub scrollbar: ScrollViewScrollbar,
    pub scrollbar_style: Rc<dyn Fn(&str) -> String>,
    pub scrollbar_hide_delay_ms: u64,
}

impl Default for ScrollViewOptions {
    fn default() -> Self {
        Self {
            axis: None,
            follow: ScrollFollow::None,
            primary: false,
            overscroll: ScrollOverscroll::Chain,
            scrollbar: ScrollViewScrollbar::Hidden,
            scrollbar_style: Rc::new(|text: &str| format!("\x1b[100m{text}\x1b[49m")),
            scrollbar_hide_delay_ms: 1000,
        }
    }
}

pub struct ScrollView {
    child: Box<dyn Component>,
    follow_end: bool,
    pub primary: bool,
    pub overscroll: ScrollOverscroll,
    scrollbar_style: Rc<dyn Fn(&str) -> String>,
    current_scrollbar: ScrollViewScrollbar,
    scrollbar_hide_delay_ms: u64,
    current_scroll_top: usize,
    content_height: usize,
    current_viewport_height: usize,
    following_end: bool,
    follow_suppressed_at_end: bool,
    transient_scrollbar_visible: bool,
    scrollbar_active: bool,
    hide_at_ms: Option<u64>,
    now_ms: u64,
}

impl ScrollView {
    pub fn new(child: Box<dyn Component>, options: ScrollViewOptions) -> Result<Self, String> {
        if let Some(axis) = &options.axis {
            if axis != "vertical" {
                return Err(format!("Unsupported ScrollView axis: {axis}"));
            }
        }
        let follow_end = options.follow == ScrollFollow::End;
        Ok(Self {
            child,
            follow_end,
            primary: options.primary,
            overscroll: options.overscroll,
            scrollbar_style: options.scrollbar_style,
            current_scrollbar: options.scrollbar,
            scrollbar_hide_delay_ms: options.scrollbar_hide_delay_ms,
            current_scroll_top: 0,
            content_height: 0,
            current_viewport_height: 0,
            following_end: follow_end,
            follow_suppressed_at_end: false,
            transient_scrollbar_visible: false,
            scrollbar_active: false,
            hide_at_ms: None,
            now_ms: 0,
        })
    }

    pub fn child_mut(&mut self) -> &mut dyn Component {
        &mut *self.child
    }

    pub fn scroll_top(&self) -> usize {
        self.current_scroll_top
    }

    pub fn is_following_end(&self) -> bool {
        self.following_end
    }

    pub fn viewport_height(&self) -> usize {
        self.current_viewport_height
    }

    pub fn scrollbar(&self) -> ScrollViewScrollbar {
        self.current_scrollbar
    }

    pub fn is_scrollbar_visible(&self) -> bool {
        match self.current_scrollbar {
            ScrollViewScrollbar::Always => self.current_viewport_height > 0,
            ScrollViewScrollbar::Auto => {
                self.content_height > self.current_viewport_height
                    && self.transient_scrollbar_visible
            }
            ScrollViewScrollbar::Hidden => false,
        }
    }

    pub fn scrollbar_style(&self) -> Rc<dyn Fn(&str) -> String> {
        self.scrollbar_style.clone()
    }

    pub fn style_scrollbar_cell(&self, text: &str) -> String {
        (self.scrollbar_style)(text)
    }

    pub fn get_content_width(&self, width: usize) -> usize {
        if self.current_scrollbar == ScrollViewScrollbar::Always && width > 1 {
            width - 1
        } else {
            width
        }
    }

    pub fn tick(&mut self, ms: u64) {
        self.now_ms += ms;
        if let Some(at) = self.hide_at_ms {
            if self.now_ms >= at {
                self.hide_at_ms = None;
                self.transient_scrollbar_visible = false;
            }
        }
    }

    fn mark_scrollbar_activity(&mut self) {
        if self.current_scrollbar != ScrollViewScrollbar::Auto
            || self.content_height <= self.current_viewport_height
        {
            return;
        }
        self.transient_scrollbar_visible = true;
        if self.scrollbar_active {
            return;
        }
        self.hide_at_ms = Some(self.now_ms.saturating_add(self.scrollbar_hide_delay_ms));
    }

    fn hide_transient_scrollbar(&mut self) {
        self.transient_scrollbar_visible = false;
        self.hide_at_ms = None;
    }

    pub fn set_scrollbar(&mut self, scrollbar: ScrollViewScrollbar) {
        if scrollbar == self.current_scrollbar {
            return;
        }
        self.current_scrollbar = scrollbar;
        if scrollbar != ScrollViewScrollbar::Auto {
            self.hide_transient_scrollbar();
        } else if self.scrollbar_active {
            self.mark_scrollbar_activity();
        }
    }

    pub fn set_scrollbar_active(&mut self, active: bool) {
        if active == self.scrollbar_active {
            return;
        }
        self.scrollbar_active = active;
        self.mark_scrollbar_activity();
    }

    pub fn scroll_to(&mut self, scroll_top: usize, disable_follow: bool) {
        let max_scroll_top = self
            .content_height
            .saturating_sub(self.current_viewport_height);
        let next = scroll_top.min(max_scroll_top);
        let next_follow_suppressed = disable_follow && next == max_scroll_top;
        let next_following_end =
            !next_follow_suppressed && self.follow_end && next == max_scroll_top;
        if next == self.current_scroll_top
            && next_following_end == self.following_end
            && next_follow_suppressed == self.follow_suppressed_at_end
        {
            return;
        }
        let moved = next != self.current_scroll_top;
        self.current_scroll_top = next;
        self.following_end = next_following_end;
        self.follow_suppressed_at_end = next_follow_suppressed;
        if moved {
            self.mark_scrollbar_activity();
        }
    }

    pub fn scroll_by(&mut self, lines: isize) -> isize {
        if lines == 0 {
            return 0;
        }
        let max_scroll_top = self
            .content_height
            .saturating_sub(self.current_viewport_height);
        let start = if self.following_end {
            max_scroll_top
        } else {
            self.current_scroll_top
        };
        let next = start.saturating_add_signed(lines).min(max_scroll_top);
        let moved = next as isize - start as isize;
        self.current_scroll_top = next;
        self.following_end = self.follow_end && next == max_scroll_top;
        self.follow_suppressed_at_end = false;
        if moved != 0 {
            self.mark_scrollbar_activity();
        }
        lines - moved
    }

    pub fn scroll_to_start(&mut self) {
        self.current_scroll_top = 0;
        self.following_end = self.follow_end && self.content_height <= self.current_viewport_height;
        self.follow_suppressed_at_end = false;
        self.mark_scrollbar_activity();
    }

    pub fn scroll_to_end(&mut self) {
        let next = self
            .content_height
            .saturating_sub(self.current_viewport_height);
        self.current_scroll_top = next;
        self.following_end = self.follow_end;
        self.follow_suppressed_at_end = false;
        self.mark_scrollbar_activity();
    }

    pub fn update_layout(&mut self, content_height: usize, viewport_height: usize) {
        self.content_height = content_height;
        self.current_viewport_height = viewport_height;
        let max_scroll_top = self
            .content_height
            .saturating_sub(self.current_viewport_height);
        if self.following_end {
            self.current_scroll_top = max_scroll_top;
        } else {
            self.current_scroll_top = self.current_scroll_top.min(max_scroll_top);
        }
        if self.current_scroll_top < max_scroll_top {
            self.follow_suppressed_at_end = false;
        }
        if self.follow_end
            && self.current_scroll_top == max_scroll_top
            && !self.follow_suppressed_at_end
        {
            self.following_end = true;
        }
        if self.content_height <= self.current_viewport_height {
            self.hide_transient_scrollbar();
        }
    }
}

impl Component for ScrollView {
    fn render(&self, width: usize) -> Vec<String> {
        let content_width = self.get_content_width(width);
        let lines = self.child.render(content_width);
        if content_width == width {
            lines
        } else {
            lines.into_iter().map(|line| format!("{line} ")).collect()
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
    fn rejects_unsupported_axis() {
        let result = ScrollView::new(
            Box::new(Text { value: "x".into() }),
            ScrollViewOptions {
                axis: Some("horizontal".into()),
                ..ScrollViewOptions::default()
            },
        );
        let err = match result {
            Ok(_) => panic!("expected unsupported axis"),
            Err(message) => message,
        };
        assert!(err.contains("Unsupported ScrollView axis"));
    }
}
