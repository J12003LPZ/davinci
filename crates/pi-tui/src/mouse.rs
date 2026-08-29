//! SGR mouse protocol (1006) matching TypeScript alt-screen mouse handling.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    Move,
    Release,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub button: MouseButton,
    pub col: usize,
    pub row: usize,
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

/// Parse `\x1b[<b;x;yM` / `\x1b[<b;x;ym`.
pub fn parse_sgr_mouse(raw: &str) -> Option<MouseEvent> {
    let rest = raw.strip_prefix("\u{1b}[<")?;
    let end = rest.chars().last()?;
    if end != 'M' && end != 'm' {
        return None;
    }
    let body = &rest[..rest.len() - 1];
    let mut parts = body.split(';');
    let btn: u16 = parts.next()?.parse().ok()?;
    let col: usize = parts.next()?.parse().ok()?;
    let row: usize = parts.next()?.parse().ok()?;
    let shift = btn & 4 != 0;
    let alt = btn & 8 != 0;
    let ctrl = btn & 16 != 0;
    let code = btn & !0b11100;
    let button = if end == 'm' {
        MouseButton::Release
    } else {
        match code {
            0 => MouseButton::Left,
            1 => MouseButton::Middle,
            2 => MouseButton::Right,
            64 => MouseButton::WheelUp,
            65 => MouseButton::WheelDown,
            32..=35 => MouseButton::Move,
            _ => MouseButton::Move,
        }
    };
    Some(MouseEvent {
        button,
        col: col.saturating_sub(1),
        row: row.saturating_sub(1),
        shift,
        alt,
        ctrl,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub col: usize,
    pub row: usize,
    pub width: usize,
    pub height: usize,
}

impl Rect {
    pub fn contains(&self, col: usize, row: usize) -> bool {
        col >= self.col
            && col < self.col + self.width
            && row >= self.row
            && row < self.row + self.height
    }
}

pub fn overlay_rect(
    term_width: usize,
    term_height: usize,
    overlay_width: usize,
    overlay_height: usize,
    anchor: &str,
    offset_x: i32,
    offset_y: i32,
) -> Rect {
    let w = overlay_width.min(term_width);
    let h = overlay_height.min(term_height);
    let (mut col, mut row) = match anchor {
        "top-left" => (0, 0),
        "top-right" => (term_width.saturating_sub(w), 0),
        "bottom-left" => (0, term_height.saturating_sub(h)),
        "bottom-right" => (term_width.saturating_sub(w), term_height.saturating_sub(h)),
        "top-center" => (term_width.saturating_sub(w) / 2, 0),
        "bottom-center" => (
            term_width.saturating_sub(w) / 2,
            term_height.saturating_sub(h),
        ),
        "left-center" => (0, term_height.saturating_sub(h) / 2),
        "right-center" => (
            term_width.saturating_sub(w),
            term_height.saturating_sub(h) / 2,
        ),
        _ => (
            term_width.saturating_sub(w) / 2,
            term_height.saturating_sub(h) / 2,
        ),
    };
    col = (col as i32 + offset_x).max(0) as usize;
    row = (row as i32 + offset_y).max(0) as usize;
    Rect {
        col,
        row,
        width: w,
        height: h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_click_and_hits_overlay() {
        let ev = parse_sgr_mouse("\u{1b}[<0;10;5M").unwrap();
        assert_eq!(ev.button, MouseButton::Left);
        assert_eq!(ev.col, 9);
        assert_eq!(ev.row, 4);
        let rect = overlay_rect(80, 24, 20, 6, "center", 0, 0);
        assert!(rect.contains(40, 12));
        assert!(!rect.contains(0, 0));
    }
}
