//! TypeScript `packages/tui/src/terminal-colors.ts`.

pub const OSC11_QUERY: &str = "\u{1b}]11;?\u{07}";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalColorScheme {
    Dark,
    Light,
}

fn hex_to_rgb(hex: &str) -> Option<RgbColor> {
    let normalized = hex.strip_prefix('#').unwrap_or(hex);
    if normalized.len() != 6 {
        return None;
    }
    Some(RgbColor {
        r: u8::from_str_radix(&normalized[0..2], 16).ok()?,
        g: u8::from_str_radix(&normalized[2..4], 16).ok()?,
        b: u8::from_str_radix(&normalized[4..6], 16).ok()?,
    })
}

fn parse_osc_hex_channel(channel: &str) -> Option<u8> {
    if channel.is_empty() || !channel.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let max = 16u32.pow(channel.len() as u32).saturating_sub(1);
    if max == 0 {
        return None;
    }
    let value = u32::from_str_radix(channel, 16).ok()?;
    Some(((f64::from(value) / f64::from(max)) * 255.0).round() as u8)
}

fn osc11_payload(data: &str) -> Option<&str> {
    let rest = data.strip_prefix("\u{1b}]11;")?;
    rest.strip_suffix('\u{07}')
        .or_else(|| rest.strip_suffix("\u{1b}\\"))
}

pub fn is_osc11_background_color_response(data: &str) -> bool {
    osc11_payload(data).is_some()
}

pub fn parse_osc11_background_color(data: &str) -> Option<RgbColor> {
    let value = osc11_payload(data)?.trim();
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() == 6 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return hex_to_rgb(value);
        }
        if hex.len() == 12 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            let r = parse_osc_hex_channel(&hex[0..4])?;
            let g = parse_osc_hex_channel(&hex[4..8])?;
            let b = parse_osc_hex_channel(&hex[8..12])?;
            return Some(RgbColor { r, g, b });
        }
        return None;
    }
    let rgb_value = value
        .strip_prefix("rgba:")
        .or_else(|| value.strip_prefix("RGBA:"))
        .or_else(|| value.strip_prefix("rgb:"))
        .or_else(|| value.strip_prefix("RGB:"))
        .unwrap_or(value);
    let mut parts = rgb_value.split('/');
    let red = parts.next()?;
    let green = parts.next()?;
    let blue = parts.next()?;
    Some(RgbColor {
        r: parse_osc_hex_channel(red)?,
        g: parse_osc_hex_channel(green)?,
        b: parse_osc_hex_channel(blue)?,
    })
}

pub fn parse_terminal_color_scheme_report(data: &str) -> Option<TerminalColorScheme> {
    if data.is_empty() {
        return None;
    }
    let mut rest = data;
    let mut last = None;
    while let Some(after) = rest.strip_prefix("\u{1b}[?997;") {
        let (digit, tail) = after.split_at(1);
        if !tail.starts_with('n') {
            return None;
        }
        last = Some(match digit {
            "2" => TerminalColorScheme::Light,
            "1" => TerminalColorScheme::Dark,
            _ => return None,
        });
        rest = &tail[1..];
    }
    if rest.is_empty() {
        last
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_osc11_hex_and_rgb_fixtures() {
        assert_eq!(
            parse_osc11_background_color("\u{1b}]11;rgb:0000/8000/ffff\u{07}"),
            Some(RgbColor {
                r: 0,
                g: 128,
                b: 255
            })
        );
        assert_eq!(
            parse_osc11_background_color("\u{1b}]11;#ffffff\u{1b}\\"),
            Some(RgbColor {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(
            parse_osc11_background_color("\u{1b}]11;#000000\u{07}"),
            Some(RgbColor { r: 0, g: 0, b: 0 })
        );
        assert_eq!(
            parse_osc11_background_color("x\u{1b}]11;#ffffff\u{07}"),
            None
        );
        assert_eq!(
            parse_osc11_background_color("\u{1b}]10;#ffffff\u{07}"),
            None
        );
        assert_eq!(
            parse_osc11_background_color("\u{1b}]11;#ffffff\u{07}x"),
            None
        );
        assert!(is_osc11_background_color_response(
            "\u{1b}]11;#000000\u{07}"
        ));
        assert!(!is_osc11_background_color_response("hello"));
    }

    #[test]
    fn parses_color_scheme_reports() {
        assert_eq!(
            parse_terminal_color_scheme_report("\u{1b}[?997;1n"),
            Some(TerminalColorScheme::Dark)
        );
        assert_eq!(
            parse_terminal_color_scheme_report("\u{1b}[?997;2n"),
            Some(TerminalColorScheme::Light)
        );
        assert_eq!(
            parse_terminal_color_scheme_report("\u{1b}[?997;2n\u{1b}[?997;1n\u{1b}[?997;1n"),
            Some(TerminalColorScheme::Dark)
        );
        assert_eq!(
            parse_terminal_color_scheme_report("\u{1b}[?997;1n\u{1b}[?997;2n\u{1b}[?997;2n"),
            Some(TerminalColorScheme::Light)
        );
        assert!(parse_terminal_color_scheme_report("\u{1b}[?997;3n").is_none());
        assert!(parse_terminal_color_scheme_report("\u{1b}[?996n").is_none());
        assert!(parse_terminal_color_scheme_report("x\u{1b}[?997;1n").is_none());
    }
}
