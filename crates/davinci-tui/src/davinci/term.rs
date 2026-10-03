//! Terminal capability negotiation.
//!
//! The design's palette is truecolor (design.md §2). Unlike termbox, crossterm
//! can emit 24-bit color, so the reference implementation's 256-color
//! negotiation is a floor rather than a ceiling here: truecolor when the
//! terminal advertises it, the nearest xterm-256 indices when it advertises
//! 256, eight named colors below that.
//!
//! `NO_COLOR` and `--no-animation` are honoured here so no other module has to
//! read the environment.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/term.ex`.

use super::theme::ColorDepth;

/// What the terminal advertises, from the environment.
pub fn detect_color_depth() -> ColorDepth {
    color_depth_from(
        std::env::var("COLORTERM").ok().as_deref(),
        std::env::var("TERM").ok().as_deref(),
        std::env::var("WT_SESSION").ok().as_deref(),
    )
}

/// Pure form of [`detect_color_depth`], so the mapping can be tested.
///
/// `wt_session` is Windows Terminal's marker: it is truecolor but sets neither
/// `COLORTERM` nor a useful `TERM`.
pub fn color_depth_from(
    colorterm: Option<&str>,
    term: Option<&str>,
    wt_session: Option<&str>,
) -> ColorDepth {
    if matches!(colorterm, Some("truecolor") | Some("24bit")) {
        return ColorDepth::TrueColor;
    }
    if wt_session.is_some_and(|value| !value.is_empty()) {
        return ColorDepth::TrueColor;
    }
    match term {
        Some(term) if term.contains("direct") => ColorDepth::TrueColor,
        Some(term) if term.contains("256") => ColorDepth::Ansi256,
        Some("dumb") | None => ColorDepth::Basic,
        Some(_) => ColorDepth::Basic,
    }
}

/// <https://no-color.org>: any non-empty value other than `0` turns color off.
pub fn no_color() -> bool {
    no_color_from(std::env::var("NO_COLOR").ok().as_deref())
}

/// Pure form of [`no_color`].
pub fn no_color_from(value: Option<&str>) -> bool {
    !matches!(value, None | Some("") | Some("0"))
}

/// Two animations exist, and both stop here (design.md §8).
pub fn animate(args: &[String]) -> bool {
    animate_from(
        args,
        std::env::var("PI_NO_ANIMATION").ok().as_deref(),
        std::env::var("PI_REDUCED_MOTION").ok().as_deref(),
    )
}

/// Pure form of [`animate`].
pub fn animate_from(
    args: &[String],
    no_animation: Option<&str>,
    reduced_motion: Option<&str>,
) -> bool {
    if args.iter().any(|arg| arg == "--no-animation") {
        return false;
    }
    !(truthy(no_animation) || truthy(reduced_motion))
}

fn truthy(value: Option<&str>) -> bool {
    !matches!(value, None | Some("") | Some("0"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colorterm_wins_and_gives_truecolor() {
        assert_eq!(
            color_depth_from(Some("truecolor"), Some("xterm"), None),
            ColorDepth::TrueColor
        );
        assert_eq!(
            color_depth_from(Some("24bit"), Some("dumb"), None),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn windows_terminal_is_truecolor_without_advertising_it() {
        assert_eq!(
            color_depth_from(None, None, Some("abc-123")),
            ColorDepth::TrueColor
        );
        assert_eq!(color_depth_from(None, None, Some("")), ColorDepth::Basic);
    }

    #[test]
    fn a_256_color_term_resolves_to_the_index_ramp() {
        assert_eq!(
            color_depth_from(None, Some("xterm-256color"), None),
            ColorDepth::Ansi256
        );
        assert_eq!(
            color_depth_from(None, Some("screen-256color"), None),
            ColorDepth::Ansi256
        );
    }

    #[test]
    fn a_direct_color_term_resolves_to_truecolor() {
        assert_eq!(
            color_depth_from(None, Some("xterm-direct"), None),
            ColorDepth::TrueColor
        );
    }

    #[test]
    fn anything_else_falls_back_to_eight_colors() {
        assert_eq!(
            color_depth_from(None, Some("dumb"), None),
            ColorDepth::Basic
        );
        assert_eq!(
            color_depth_from(None, Some("vt100"), None),
            ColorDepth::Basic
        );
        assert_eq!(color_depth_from(None, None, None), ColorDepth::Basic);
    }

    #[test]
    fn no_color_follows_the_no_color_convention() {
        assert!(!no_color_from(None));
        assert!(!no_color_from(Some("")));
        assert!(!no_color_from(Some("0")));
        assert!(no_color_from(Some("1")));
        assert!(no_color_from(Some("true")));
        assert!(no_color_from(Some("anything")));
    }

    #[test]
    fn animation_stops_on_the_flag_or_either_environment_marker() {
        let none: Vec<String> = Vec::new();
        let flagged = vec!["--no-animation".to_string()];
        assert!(animate_from(&none, None, None));
        assert!(!animate_from(&flagged, None, None));
        assert!(!animate_from(&none, Some("1"), None));
        assert!(!animate_from(&none, None, Some("1")));
        assert!(animate_from(&none, Some("0"), Some("")));
    }
}
