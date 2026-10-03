/// SGR mouse report matching `vendor/pi/packages/tui` mouse tracking.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Down,
    Up,
    Drag,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Wheel,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub button: MouseButton,
    pub x: u16,
    pub y: u16,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// TS `ENABLE_ALL_MOTION_MOUSE` from `tui-alt-screen.ts`.
pub const MOUSE_ENABLE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1004h\x1b[?1006h";
/// TS `DISABLE_MOUSE` from `tui-alt-screen.ts`.
pub const MOUSE_DISABLE: &str = "\x1b[?1006l\x1b[?1004l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

/// Parse an SGR mouse sequence such as `\x1b[<0;10;5M`.
pub fn parse_mouse_sgr(input: &str) -> Option<MouseEvent> {
    let rest = input
        .strip_prefix("\x1b[<")
        .or_else(|| input.strip_prefix("\u{1b}[<"))
        .or_else(|| input.strip_prefix("[<"))?;
    let end = rest.chars().last()?;
    if end != 'M' && end != 'm' {
        return None;
    }
    let body = &rest[..rest.len() - 1];
    let mut parts = body.split(';');
    let btn: u16 = parts.next()?.parse().ok()?;
    let x: u16 = parts.next()?.parse().ok()?;
    let y: u16 = parts.next()?.parse().ok()?;
    let shift = btn & 4 != 0;
    let alt = btn & 8 != 0;
    let ctrl = btn & 16 != 0;
    let code = btn & !0b11100;
    let (kind, button) = if code == 64 {
        (MouseKind::ScrollUp, MouseButton::Wheel)
    } else if code == 65 {
        (MouseKind::ScrollDown, MouseButton::Wheel)
    } else if code & 32 != 0 {
        (
            MouseKind::Drag,
            match code & 3 {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                _ => MouseButton::None,
            },
        )
    } else if end == 'm' {
        (MouseKind::Up, button_from_code(code))
    } else {
        (MouseKind::Down, button_from_code(code))
    };
    Some(MouseEvent {
        kind,
        button,
        x: x.saturating_sub(1),
        y: y.saturating_sub(1),
        ctrl,
        alt,
        shift,
    })
}

fn button_from_code(code: u16) -> MouseButton {
    match code & 3 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        2 => MouseButton::Right,
        _ => MouseButton::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_click_and_scroll() {
        let click = parse_mouse_sgr("\x1b[<0;12;4M").unwrap();
        assert_eq!(click.kind, MouseKind::Down);
        assert_eq!(click.button, MouseButton::Left);
        assert_eq!(click.x, 11);
        assert_eq!(click.y, 3);
        let up = parse_mouse_sgr("\x1b[<0;12;4m").unwrap();
        assert_eq!(up.kind, MouseKind::Up);
        let scroll = parse_mouse_sgr("\x1b[<64;1;1M").unwrap();
        assert_eq!(scroll.kind, MouseKind::ScrollUp);
    }
}
