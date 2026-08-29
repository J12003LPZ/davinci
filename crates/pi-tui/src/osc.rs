//! OSC 11 / color-scheme / COLORFGBG detection matching TS `terminal-colors.ts`
//! and `detectTerminalThemeForAuto`.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub const OSC_11_QUERY: &str = "\x1b]11;?\x07";
pub const COLOR_SCHEME_QUERY: &str = "\x1b[?996n";

const BASIC_ANSI: [[u8; 3]; 16] = [
    [0, 0, 0],
    [128, 0, 0],
    [0, 128, 0],
    [128, 128, 0],
    [0, 0, 128],
    [128, 0, 128],
    [0, 128, 128],
    [192, 192, 192],
    [128, 128, 128],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [0, 0, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

fn parse_osc_hex_channel(channel: &str) -> Option<u8> {
    if channel.is_empty() || !channel.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let max = 16u32.pow(channel.len() as u32).saturating_sub(1);
    if max == 0 {
        return None;
    }
    let value = u32::from_str_radix(channel, 16).ok()?;
    Some(((value * 255 + max / 2) / max) as u8)
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

pub fn is_osc11_background_color_response(data: &str) -> bool {
    parse_osc11_background_color(data).is_some()
        || (data.starts_with("\x1b]11;") && (data.ends_with('\x07') || data.ends_with("\x1b\\")))
}

pub fn parse_osc11_background_color(data: &str) -> Option<RgbColor> {
    let value = data.strip_prefix("\x1b]11;").and_then(|rest| {
        rest.strip_suffix('\x07')
            .or_else(|| rest.strip_suffix("\x1b\\"))
    })?;
    if data.as_bytes().first() != Some(&0x1b) {
        return None;
    }
    let value = value.trim();
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
    let r = parse_osc_hex_channel(parts.next()?)?;
    let g = parse_osc_hex_channel(parts.next()?)?;
    let b = parse_osc_hex_channel(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(RgbColor { r, g, b })
}

pub fn parse_terminal_color_scheme_report(data: &str) -> Option<&'static str> {
    if !data.starts_with("\x1b[?997;") || !data.ends_with('n') {
        return None;
    }
    let mut last = None;
    let mut rest = data;
    while let Some(stripped) = rest.strip_prefix("\x1b[?997;") {
        let (code, after) = stripped.split_once('n')?;
        if code != "1" && code != "2" {
            return None;
        }
        last = Some(if code == "2" { "light" } else { "dark" });
        rest = after;
    }
    if !rest.is_empty() {
        return None;
    }
    last
}

fn luminance(rgb: RgbColor) -> f64 {
    let to_linear = |channel: u8| {
        let value = f64::from(channel) / 255.0;
        if value <= 0.03928 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * to_linear(rgb.r) + 0.7152 * to_linear(rgb.g) + 0.0722 * to_linear(rgb.b)
}

pub fn get_theme_for_rgb(rgb: RgbColor) -> &'static str {
    if luminance(rgb) >= 0.5 {
        "light"
    } else {
        "dark"
    }
}

fn ansi256_rgb(index: u8) -> RgbColor {
    if index < 16 {
        let [r, g, b] = BASIC_ANSI[index as usize];
        return RgbColor { r, g, b };
    }
    if index < 232 {
        let cube = index - 16;
        let to = |n: u8| if n == 0 { 0 } else { 55 + n * 40 };
        return RgbColor {
            r: to(cube / 36),
            g: to((cube % 36) / 6),
            b: to(cube % 6),
        };
    }
    let gray = 8 + (index - 232) * 10;
    RgbColor {
        r: gray,
        g: gray,
        b: gray,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeDetection {
    pub theme: String,
    pub source: String,
    pub confidence: String,
}

/// Dedicated OSC drain independent of the crossterm key loop.
/// Reads `PI_OSC_DRAIN_REPLY`, then polls `PI_OSC_TTY`, then `/dev/tty`,
/// until data arrives or `timeout_ms` elapses — matching TS `queryTerminalBackgroundColor`.
pub fn drain_osc_tty(timeout_ms: u64) -> Option<String> {
    if let Ok(reply) = std::env::var("PI_OSC_DRAIN_REPLY") {
        if !reply.is_empty() {
            return Some(reply);
        }
    }
    if let Ok(path) = std::env::var("PI_OSC_TTY") {
        if let Some(data) = poll_path(&path, timeout_ms) {
            return Some(data);
        }
    }
    if std::env::var("PI_OSC_TTY").is_err() {
        return read_tty_timeout("/dev/tty", timeout_ms);
    }
    None
}

fn poll_path(path: &str, timeout_ms: u64) -> Option<String> {
    let started = std::time::Instant::now();
    loop {
        if let Ok(data) = std::fs::read_to_string(path) {
            if !data.is_empty() {
                return Some(data);
            }
        }
        if started.elapsed() >= std::time::Duration::from_millis(timeout_ms) {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn read_tty_timeout(path: &str, timeout_ms: u64) -> Option<String> {
    let path = path.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut file = match std::fs::File::open(&path) {
            Ok(file) => file,
            Err(_) => return,
        };
        let mut buf = [0u8; 4096];
        if let Ok(n) = std::io::Read::read(&mut file, &mut buf) {
            if n > 0 {
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).into_owned());
            }
        }
    });
    rx.recv_timeout(std::time::Duration::from_millis(timeout_ms.max(1)))
        .ok()
}

pub fn query_terminal_background_color(timeout_ms: u64) -> Option<RgbColor> {
    if let Some(data) = drain_osc_tty(timeout_ms) {
        return parse_osc11_background_color(&data);
    }
    None
}

pub fn detect_terminal_background_from_env(colorfgbg: Option<&str>) -> ThemeDetection {
    if let Some(colorfgbg) = colorfgbg.filter(|value| !value.is_empty()) {
        let bg = colorfgbg
            .split(';')
            .rev()
            .find_map(|part| part.trim().parse::<u8>().ok());
        if let Some(bg) = bg {
            return ThemeDetection {
                theme: get_theme_for_rgb(ansi256_rgb(bg)).to_string(),
                source: "COLORFGBG".into(),
                confidence: "high".into(),
            };
        }
    }
    ThemeDetection {
        theme: "dark".into(),
        source: "fallback".into(),
        confidence: "low".into(),
    }
}

pub fn detect_terminal_theme_for_auto(
    color_scheme_reply: Option<&str>,
    osc11_reply: Option<&str>,
    colorfgbg: Option<&str>,
) -> ThemeDetection {
    if let Some(scheme) = color_scheme_reply.and_then(parse_terminal_color_scheme_report) {
        return ThemeDetection {
            theme: scheme.into(),
            source: "color-scheme".into(),
            confidence: "high".into(),
        };
    }
    if let Some(rgb) = osc11_reply.and_then(parse_osc11_background_color) {
        return ThemeDetection {
            theme: get_theme_for_rgb(rgb).to_string(),
            source: "terminal background".into(),
            confidence: "high".into(),
        };
    }
    detect_terminal_background_from_env(colorfgbg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_osc11_matches_ts() {
        assert_eq!(
            parse_osc11_background_color("\x1b]11;rgb:0000/8000/ffff\x07"),
            Some(RgbColor {
                r: 0,
                g: 128,
                b: 255
            })
        );
        assert_eq!(
            parse_osc11_background_color("\x1b]11;#ffffff\x1b\\"),
            Some(RgbColor {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert_eq!(
            parse_osc11_background_color("\x1b]11;#000000\x07"),
            Some(RgbColor { r: 0, g: 0, b: 0 })
        );
        assert!(parse_osc11_background_color("x\x1b]11;#ffffff\x07").is_none());
        assert!(parse_osc11_background_color("\x1b]10;#ffffff\x07").is_none());
        assert!(parse_osc11_background_color("\x1b]11;#ffffff\x07x").is_none());
    }

    #[test]
    fn parse_color_scheme_matches_ts() {
        assert_eq!(
            parse_terminal_color_scheme_report("\x1b[?997;1n"),
            Some("dark")
        );
        assert_eq!(
            parse_terminal_color_scheme_report("\x1b[?997;2n"),
            Some("light")
        );
        assert_eq!(
            parse_terminal_color_scheme_report("\x1b[?997;2n\x1b[?997;1n\x1b[?997;1n"),
            Some("dark")
        );
        assert_eq!(
            parse_terminal_color_scheme_report("\x1b[?997;1n\x1b[?997;2n\x1b[?997;2n"),
            Some("light")
        );
        assert!(parse_terminal_color_scheme_report("\x1b[?997;3n").is_none());
        assert!(parse_terminal_color_scheme_report("x\x1b[?997;1n").is_none());
    }

    #[test]
    fn colorfgbg_and_auto_theme_match_ts() {
        assert_eq!(
            detect_terminal_background_from_env(Some("0;15")).theme,
            "light"
        );
        assert_eq!(
            detect_terminal_background_from_env(Some("15;0")).theme,
            "dark"
        );
        assert_eq!(
            detect_terminal_background_from_env(Some("0;7;15")).theme,
            "light"
        );
        assert_eq!(
            detect_terminal_background_from_env(Some("")).source,
            "fallback"
        );
        assert_eq!(
            detect_terminal_theme_for_auto(Some("\x1b[?997;2n"), None, Some("15;0")).theme,
            "light"
        );
        assert_eq!(
            detect_terminal_theme_for_auto(None, Some("\x1b]11;#ffffff\x07"), Some("15;0")).theme,
            "light"
        );
        assert_eq!(
            detect_terminal_theme_for_auto(None, None, Some("15;0")).source,
            "COLORFGBG"
        );
    }

    #[test]
    fn drain_osc_tty_reads_path_with_timeout() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("osc");
        std::fs::write(&path, "\x1b]11;#ffffff\x07").expect("write");
        std::env::set_var("PI_OSC_TTY", path.display().to_string());
        std::env::remove_var("PI_OSC_DRAIN_REPLY");
        let drained = drain_osc_tty(50).expect("drain");
        assert_eq!(
            query_terminal_background_color(0),
            Some(RgbColor {
                r: 255,
                g: 255,
                b: 255
            })
        );
        assert!(drained.contains("11;"));
        std::env::remove_var("PI_OSC_TTY");
    }

    #[test]
    fn drain_osc_tty_times_out_when_path_empty() {
        std::env::remove_var("PI_OSC_DRAIN_REPLY");
        std::env::set_var("PI_OSC_TTY", "/no/such/pi-osc-tty");
        assert!(drain_osc_tty(15).is_none());
        std::env::remove_var("PI_OSC_TTY");
    }
}
