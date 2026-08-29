//! TypeScript `ScrollView` viewport state used by `TuiAltScreen`.

use crate::component::Component;
use crate::constrained_layout::Node;
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollbarMode {
    Hidden,
    Auto,
    Always,
}

impl ScrollbarMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "hidden" => Some(Self::Hidden),
            "auto" => Some(Self::Auto),
            "always" => Some(Self::Always),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Overscroll {
    Chain,
    Contain,
}

pub struct ViewportScrollOptions {
    pub follow_end: bool,
    pub primary: bool,
    pub overscroll: Overscroll,
    pub scrollbar: ScrollbarMode,
    pub scrollbar_hide_delay_ms: u64,
}

impl Default for ViewportScrollOptions {
    fn default() -> Self {
        Self {
            follow_end: false,
            primary: false,
            overscroll: Overscroll::Chain,
            scrollbar: ScrollbarMode::Hidden,
            scrollbar_hide_delay_ms: 1000,
        }
    }
}

pub struct ViewportScrollState {
    pub child: Node,
    pub follow_end: bool,
    pub primary: bool,
    pub overscroll: Overscroll,
    pub scrollbar: ScrollbarMode,
    pub scrollbar_style: fn(&str) -> String,
    pub scrollbar_hide_delay_ms: u64,
    pub scroll_top: usize,
    pub content_height: usize,
    pub viewport_height: usize,
    pub following_end: bool,
    pub follow_suppressed_at_end: bool,
    pub transient_scrollbar_visible: bool,
    pub scrollbar_active: bool,
    pub hide_at_ms: Option<u64>,
}

#[derive(Clone)]
pub struct ViewportScroll {
    pub inner: Rc<RefCell<ViewportScrollState>>,
}

impl ViewportScroll {
    pub fn new(child: Node, options: ViewportScrollOptions) -> Self {
        let follow_end = options.follow_end;
        Self {
            inner: Rc::new(RefCell::new(ViewportScrollState {
                child,
                follow_end,
                primary: options.primary,
                overscroll: options.overscroll,
                scrollbar: options.scrollbar,
                scrollbar_style: |text| format!("\u{1b}[100m{text}\u{1b}[49m"),
                scrollbar_hide_delay_ms: options.scrollbar_hide_delay_ms,
                scroll_top: 0,
                content_height: 0,
                viewport_height: 0,
                following_end: follow_end,
                follow_suppressed_at_end: false,
                transient_scrollbar_visible: false,
                scrollbar_active: false,
                hide_at_ms: None,
            })),
        }
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn scroll_top(&self) -> usize {
        self.inner.borrow().scroll_top
    }

    pub fn is_following_end(&self) -> bool {
        self.inner.borrow().following_end
    }

    pub fn viewport_height(&self) -> usize {
        self.inner.borrow().viewport_height
    }

    pub fn is_scrollbar_visible(&self) -> bool {
        let state = self.inner.borrow();
        match state.scrollbar {
            ScrollbarMode::Always => state.viewport_height > 0,
            ScrollbarMode::Auto => {
                state.content_height > state.viewport_height && state.transient_scrollbar_visible
            }
            ScrollbarMode::Hidden => false,
        }
    }

    pub fn content_width(&self, width: usize) -> usize {
        let state = self.inner.borrow();
        if state.scrollbar == ScrollbarMode::Always && width > 1 {
            width - 1
        } else {
            width
        }
    }

    pub fn tick(&self, now_ms: u64) -> bool {
        let mut state = self.inner.borrow_mut();
        if let Some(hide_at) = state.hide_at_ms {
            if now_ms >= hide_at && !state.scrollbar_active {
                state.hide_at_ms = None;
                if state.transient_scrollbar_visible {
                    state.transient_scrollbar_visible = false;
                    return true;
                }
            }
        }
        false
    }

    fn mark_scrollbar_activity(state: &mut ViewportScrollState, now_ms: u64) {
        if state.scrollbar != ScrollbarMode::Auto || state.content_height <= state.viewport_height {
            return;
        }
        state.transient_scrollbar_visible = true;
        if state.scrollbar_active {
            state.hide_at_ms = None;
            return;
        }
        state.hide_at_ms = Some(now_ms.saturating_add(state.scrollbar_hide_delay_ms));
    }

    pub fn set_scrollbar_active(&self, active: bool, now_ms: u64) {
        let mut state = self.inner.borrow_mut();
        if active == state.scrollbar_active {
            return;
        }
        state.scrollbar_active = active;
        Self::mark_scrollbar_activity(&mut state, now_ms);
    }

    pub fn scroll_to(&self, scroll_top: usize, disable_follow: bool, now_ms: u64) -> bool {
        let mut state = self.inner.borrow_mut();
        let max_scroll_top = state.content_height.saturating_sub(state.viewport_height);
        let next = scroll_top.min(max_scroll_top);
        let next_follow_suppressed = disable_follow && next == max_scroll_top;
        let next_following_end =
            !next_follow_suppressed && state.follow_end && next == max_scroll_top;
        if next == state.scroll_top
            && next_following_end == state.following_end
            && next_follow_suppressed == state.follow_suppressed_at_end
        {
            return false;
        }
        let moved = next != state.scroll_top;
        state.scroll_top = next;
        state.following_end = next_following_end;
        state.follow_suppressed_at_end = next_follow_suppressed;
        if moved {
            Self::mark_scrollbar_activity(&mut state, now_ms);
        }
        true
    }

    pub fn scroll_by(&self, lines: isize, now_ms: u64) -> isize {
        if lines == 0 {
            return 0;
        }
        let mut state = self.inner.borrow_mut();
        let max_scroll_top = state.content_height.saturating_sub(state.viewport_height) as isize;
        let start = if state.following_end {
            max_scroll_top
        } else {
            state.scroll_top as isize
        };
        let next = (start + lines).clamp(0, max_scroll_top);
        let moved = next - start;
        let was_following = state.following_end;
        state.scroll_top = next as usize;
        state.following_end = state.follow_end && next == max_scroll_top;
        state.follow_suppressed_at_end = false;
        if moved != 0 {
            Self::mark_scrollbar_activity(&mut state, now_ms);
        }
        if moved != 0 || state.following_end != was_following {
            return lines - moved;
        }
        lines - moved
    }

    pub fn scroll_to_start(&self, now_ms: u64) -> bool {
        let mut state = self.inner.borrow_mut();
        let following = state.follow_end && state.content_height <= state.viewport_height;
        let changed = state.scroll_top != 0 || state.following_end != following;
        state.scroll_top = 0;
        state.following_end = following;
        state.follow_suppressed_at_end = false;
        if changed {
            Self::mark_scrollbar_activity(&mut state, now_ms);
        }
        changed
    }

    pub fn scroll_to_end(&self, now_ms: u64) -> bool {
        let mut state = self.inner.borrow_mut();
        let next = state.content_height.saturating_sub(state.viewport_height);
        let changed = state.scroll_top != next || state.following_end != state.follow_end;
        state.scroll_top = next;
        state.following_end = state.follow_end;
        state.follow_suppressed_at_end = false;
        if changed {
            Self::mark_scrollbar_activity(&mut state, now_ms);
        }
        changed
    }

    pub fn update_layout(&self, content_height: usize, viewport_height: usize) {
        let mut state = self.inner.borrow_mut();
        state.content_height = content_height;
        state.viewport_height = viewport_height;
        let max_scroll_top = content_height.saturating_sub(viewport_height);
        if state.following_end {
            state.scroll_top = max_scroll_top;
        } else {
            state.scroll_top = state.scroll_top.min(max_scroll_top);
        }
        if state.scroll_top < max_scroll_top {
            state.follow_suppressed_at_end = false;
        }
        if state.follow_end && state.scroll_top == max_scroll_top && !state.follow_suppressed_at_end
        {
            state.following_end = true;
        }
        if content_height <= viewport_height {
            state.transient_scrollbar_visible = false;
            state.hide_at_ms = None;
        }
    }
}

impl Component for ViewportScroll {
    fn render(&self, width: usize) -> Vec<String> {
        let content_width = self.content_width(width);
        let lines = self.inner.borrow().child.render(content_width);
        if content_width == width {
            lines
        } else {
            lines.into_iter().map(|line| format!("{line} ")).collect()
        }
    }
}
