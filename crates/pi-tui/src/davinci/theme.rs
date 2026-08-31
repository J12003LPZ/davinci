//! The only place a color literal is allowed (`docs/ui/design.md` §2).
//!
//! Tokens are resolved once at startup into whatever the terminal understands,
//! so widgets pass `theme.primary` to a `Style` and never think about it.
//!
//! Copper (`primary`) carries state. Verdigris (`secondary`) carries *where
//! something is* — branch, path, symbol — and never *what is happening*. That
//! split is what keeps the palette from reading as decoration.
//!
//! Mirrors `docs/ui/davinci_tui/lib/davinci/theme.ex`.

use ratatui::style::{Color, Modifier};

/// State glyph vocabulary (design.md §4). Color reinforces, never replaces.
pub mod glyph {
    pub const DONE: &str = "✓";
    pub const ACTIVE: &str = "◉";
    pub const QUEUED: &str = "○";
    pub const SKIPPED: &str = "◌";
    pub const FAILED: &str = "×";
    pub const ATTENTION: &str = "!";
    pub const DELTA: &str = "Δ";
    pub const READ: &str = "↳";
    pub const SEARCH: &str = "⌕";
    pub const AGENT: &str = "◆";
    pub const PROMPT: &str = "›";
    pub const USER: &str = ">";
    pub const TICK: &str = "·";
    /// Studio, collapsed to one line below 100 columns (design.md §6).
    pub const COLLAPSED: &str = "⟐";

    /// One 4-frame spinner, 250ms per frame (design.md §8).
    pub const SPINNER: [&str; 4] = ["◜", "◝", "◞", "◟"];
    /// Proportion pie used by the narrow status bar.
    pub const PIE: [&str; 4] = ["◐", "◑", "◒", "◓"];

    /// Proportion meter, drawn as filled run + tip + empty run (design.md §9).
    pub const METER_FILLED: &str = "━";
    pub const METER_TIP: &str = "╸";
    pub const METER_EMPTY: &str = "─";
}

/// Every state that has a glyph. Color is always secondary to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Done,
    Active,
    Queued,
    Skipped,
    Failed,
    Attention,
    Delta,
    Read,
    Search,
    Agent,
    Prompt,
    User,
    Tick,
}

impl State {
    pub fn glyph(self) -> &'static str {
        match self {
            State::Done => glyph::DONE,
            State::Active => glyph::ACTIVE,
            State::Queued => glyph::QUEUED,
            State::Skipped => glyph::SKIPPED,
            State::Failed => glyph::FAILED,
            State::Attention => glyph::ATTENTION,
            State::Delta => glyph::DELTA,
            State::Read => glyph::READ,
            State::Search => glyph::SEARCH,
            State::Agent => glyph::AGENT,
            State::Prompt => glyph::PROMPT,
            State::User => glyph::USER,
            State::Tick => glyph::TICK,
        }
    }
}

/// What the terminal can actually render. Resolved once, at startup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorDepth {
    /// 24-bit. The design's palette is authored here.
    TrueColor,
    /// xterm-256 indices, nearest neighbours of the truecolor tokens.
    Ansi256,
    /// 8 named colors. Below 16 colors the ramp drops to `NO_COLOR` (§2).
    Basic,
}

/// One row of the §2 table, in whichever encoding the terminal understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: Color,
    pub surface: Color,
    pub surface_alt: Color,
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub primary: Color,
    pub secondary: Color,
    pub success: Color,
    pub warning: Color,
    pub error: Color,
    /// Bold under `NO_COLOR`, where active glyphs are pure white and bold (§9).
    pub emphasis: Modifier,
    pub no_color: bool,
    pub dimmed: bool,
}

/// The eleven tokens, before they are wrapped in a `Theme`.
struct Ramp {
    background: Color,
    surface: Color,
    surface_alt: Color,
    border: Color,
    text: Color,
    muted: Color,
    primary: Color,
    secondary: Color,
    success: Color,
    warning: Color,
    error: Color,
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb(
        ((hex >> 16) & 0xFF) as u8,
        ((hex >> 8) & 0xFF) as u8,
        (hex & 0xFF) as u8,
    )
}

/// design.md §2, verbatim.
const TRUECOLOR: Ramp = Ramp {
    background: rgb(0x0B1011),
    surface: rgb(0x101719),
    surface_alt: rgb(0x0E1416),
    border: rgb(0x453A27),
    text: rgb(0xDDD5C4),
    muted: rgb(0x80796D),
    primary: rgb(0xD58A32),
    secondary: rgb(0x52A89C),
    success: rgb(0x74A879),
    warning: rgb(0xD5A047),
    error: rgb(0xC4593F),
};

/// "Never blur, never tint — just drop the ramp" (design.md §2).
const TRUECOLOR_DIM: Ramp = Ramp {
    background: rgb(0x0B1011),
    surface: rgb(0x0E1416),
    surface_alt: rgb(0x0B1011),
    border: rgb(0x2B2519),
    text: rgb(0x3F3A31),
    muted: rgb(0x5D564C),
    primary: rgb(0x6B512C),
    secondary: rgb(0x2F5F59),
    success: rgb(0x435F45),
    warning: rgb(0x6B512C),
    error: rgb(0x633127),
};

/// xterm-256 nearest neighbours of the truecolor table.
const ANSI256: Ramp = Ramp {
    background: Color::Indexed(233),
    surface: Color::Indexed(235),
    surface_alt: Color::Indexed(234),
    border: Color::Indexed(58),
    text: Color::Indexed(187),
    muted: Color::Indexed(102),
    primary: Color::Indexed(173),
    secondary: Color::Indexed(73),
    success: Color::Indexed(108),
    warning: Color::Indexed(179),
    error: Color::Indexed(167),
};

const ANSI256_DIM: Ramp = Ramp {
    background: Color::Indexed(233),
    surface: Color::Indexed(234),
    surface_alt: Color::Indexed(233),
    border: Color::Indexed(236),
    text: Color::Indexed(239),
    muted: Color::Indexed(59),
    primary: Color::Indexed(94),
    secondary: Color::Indexed(66),
    success: Color::Indexed(65),
    warning: Color::Indexed(94),
    error: Color::Indexed(95),
};

/// `NO_COLOR`: greyscale ramp, active glyphs pure white and bold (design.md §9).
const GREY: Ramp = Ramp {
    background: rgb(0x0B0B0B),
    surface: rgb(0x1C1C1C),
    surface_alt: rgb(0x121212),
    border: rgb(0x5A5A5A),
    text: rgb(0xE6E6E6),
    muted: rgb(0x9A9A9A),
    primary: rgb(0xFFFFFF),
    secondary: rgb(0xCFCFCF),
    success: rgb(0xFFFFFF),
    warning: rgb(0xFFFFFF),
    error: rgb(0xFFFFFF),
};

const GREY_DIM: Ramp = Ramp {
    background: rgb(0x0B0B0B),
    surface: rgb(0x161616),
    surface_alt: rgb(0x101010),
    border: rgb(0x333333),
    text: rgb(0x6E6E6E),
    muted: rgb(0x4C4C4C),
    primary: rgb(0x9A9A9A),
    secondary: rgb(0x5F5F5F),
    success: rgb(0x9A9A9A),
    warning: rgb(0x9A9A9A),
    error: rgb(0x9A9A9A),
};

/// Eight named colors. Kept legible rather than faithful.
const BASIC: Ramp = Ramp {
    background: Color::Reset,
    surface: Color::Reset,
    surface_alt: Color::Reset,
    border: Color::DarkGray,
    text: Color::White,
    muted: Color::Gray,
    primary: Color::Yellow,
    secondary: Color::Cyan,
    success: Color::Green,
    warning: Color::Yellow,
    error: Color::Red,
};

const BASIC_DIM: Ramp = Ramp {
    background: Color::Reset,
    surface: Color::Reset,
    surface_alt: Color::Reset,
    border: Color::DarkGray,
    text: Color::DarkGray,
    muted: Color::DarkGray,
    primary: Color::DarkGray,
    secondary: Color::DarkGray,
    success: Color::DarkGray,
    warning: Color::DarkGray,
    error: Color::DarkGray,
};

const BASIC_GREY: Ramp = Ramp {
    background: Color::Reset,
    surface: Color::Reset,
    surface_alt: Color::Reset,
    border: Color::DarkGray,
    text: Color::White,
    muted: Color::Gray,
    primary: Color::White,
    secondary: Color::Gray,
    success: Color::White,
    warning: Color::White,
    error: Color::White,
};

impl Theme {
    /// Build the da Vinci theme for a negotiated color depth.
    pub fn da_vinci(depth: ColorDepth, no_color: bool) -> Self {
        let ramp = match (depth, no_color) {
            (_, true) if depth == ColorDepth::Basic => &BASIC_GREY,
            (ColorDepth::TrueColor, false) => &TRUECOLOR,
            (ColorDepth::Ansi256, false) => &ANSI256,
            (ColorDepth::Basic, false) => &BASIC,
            (_, true) => &GREY,
        };
        Self::from_ramp(ramp, no_color, false)
    }

    /// The layer behind a modal (design.md §2): drop the ramp, keep the glyphs.
    pub fn dim(&self) -> Self {
        if self.dimmed {
            return *self;
        }
        let ramp = match (self.depth_hint(), self.no_color) {
            (ColorDepth::Basic, true) => &BASIC_DIM,
            (ColorDepth::Basic, false) => &BASIC_DIM,
            (_, true) => &GREY_DIM,
            (ColorDepth::Ansi256, false) => &ANSI256_DIM,
            (ColorDepth::TrueColor, false) => &TRUECOLOR_DIM,
        };
        let mut dimmed = Self::from_ramp(ramp, self.no_color, true);
        // Nothing is emphasised behind a modal; the modal is what is in hand.
        dimmed.emphasis = Modifier::empty();
        dimmed
    }

    /// Color that reinforces a state glyph. Never the only signal.
    pub fn state_color(&self, state: State) -> Color {
        match state {
            State::Done => self.success,
            State::Active => self.primary,
            State::Queued => self.border,
            State::Skipped => self.muted,
            State::Failed => self.error,
            State::Attention => self.warning,
            State::Delta => self.primary,
            State::Read | State::Search => self.secondary,
            State::Agent | State::Prompt => self.primary,
            State::User => self.muted,
            State::Tick => self.border,
        }
    }

    /// The active Studio step's mark. Static under `--no-animation` (§8).
    pub fn spinner(&self, tick: u64, animate: bool) -> &'static str {
        if animate {
            glyph::SPINNER[(tick % 4) as usize]
        } else {
            glyph::ACTIVE
        }
    }

    /// Proportion pie for the narrow status bar. Still a meter, never a bare
    /// number (design.md §9).
    pub fn pie(&self, fraction: f64) -> &'static str {
        let index = ((fraction * 4.0) as isize).clamp(0, 3) as usize;
        glyph::PIE[index]
    }

    fn from_ramp(ramp: &Ramp, no_color: bool, dimmed: bool) -> Self {
        Self {
            background: ramp.background,
            surface: ramp.surface,
            surface_alt: ramp.surface_alt,
            border: ramp.border,
            text: ramp.text,
            muted: ramp.muted,
            primary: ramp.primary,
            secondary: ramp.secondary,
            success: ramp.success,
            warning: ramp.warning,
            error: ramp.error,
            emphasis: if no_color {
                Modifier::BOLD
            } else {
                Modifier::empty()
            },
            no_color,
            dimmed,
        }
    }

    /// Which encoding this theme was built in, recovered from its own tokens so
    /// `dim` does not need the depth passed back in.
    fn depth_hint(&self) -> ColorDepth {
        match self.text {
            Color::Rgb(..) => ColorDepth::TrueColor,
            Color::Indexed(..) => ColorDepth::Ansi256,
            _ => ColorDepth::Basic,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_STATES: [State; 13] = [
        State::Done,
        State::Active,
        State::Queued,
        State::Skipped,
        State::Failed,
        State::Attention,
        State::Delta,
        State::Read,
        State::Search,
        State::Agent,
        State::Prompt,
        State::User,
        State::Tick,
    ];

    #[test]
    fn truecolor_tokens_match_the_spec_table() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        assert_eq!(theme.background, Color::Rgb(0x0B, 0x10, 0x11));
        assert_eq!(theme.border, Color::Rgb(0x45, 0x3A, 0x27));
        assert_eq!(theme.text, Color::Rgb(0xDD, 0xD5, 0xC4));
        assert_eq!(theme.muted, Color::Rgb(0x80, 0x79, 0x6D));
        assert_eq!(theme.primary, Color::Rgb(0xD5, 0x8A, 0x32));
        assert_eq!(theme.secondary, Color::Rgb(0x52, 0xA8, 0x9C));
        assert_eq!(theme.success, Color::Rgb(0x74, 0xA8, 0x79));
        assert_eq!(theme.warning, Color::Rgb(0xD5, 0xA0, 0x47));
        assert_eq!(theme.error, Color::Rgb(0xC4, 0x59, 0x3F));
    }

    #[test]
    fn ansi256_is_the_nearest_neighbour_table() {
        let theme = Theme::da_vinci(ColorDepth::Ansi256, false);
        assert_eq!(theme.primary, Color::Indexed(173));
        assert_eq!(theme.secondary, Color::Indexed(73));
        assert_eq!(theme.border, Color::Indexed(58));
    }

    #[test]
    fn every_state_has_a_distinct_glyph() {
        let mut seen = Vec::new();
        for state in ALL_STATES {
            let glyph = state.glyph();
            assert!(!glyph.is_empty(), "{state:?} has no glyph");
            assert!(!seen.contains(&glyph), "{state:?} reuses glyph {glyph}");
            seen.push(glyph);
        }
        assert_eq!(seen.len(), 13);
    }

    #[test]
    fn glyphs_are_the_fixed_vocabulary() {
        assert_eq!(State::Done.glyph(), "✓");
        assert_eq!(State::Active.glyph(), "◉");
        assert_eq!(State::Queued.glyph(), "○");
        assert_eq!(State::Skipped.glyph(), "◌");
        assert_eq!(State::Failed.glyph(), "×");
        assert_eq!(State::Attention.glyph(), "!");
        assert_eq!(State::Delta.glyph(), "Δ");
        assert_eq!(State::Read.glyph(), "↳");
        assert_eq!(State::Search.glyph(), "⌕");
        assert_eq!(State::Agent.glyph(), "◆");
        assert_eq!(State::Prompt.glyph(), "›");
        assert_eq!(State::User.glyph(), ">");
        assert_eq!(State::Tick.glyph(), "·");
    }

    #[test]
    fn state_color_follows_the_spec_column() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        assert_eq!(theme.state_color(State::Done), theme.success);
        assert_eq!(theme.state_color(State::Active), theme.primary);
        assert_eq!(theme.state_color(State::Queued), theme.border);
        assert_eq!(theme.state_color(State::Skipped), theme.muted);
        assert_eq!(theme.state_color(State::Failed), theme.error);
        assert_eq!(theme.state_color(State::Attention), theme.warning);
        assert_eq!(theme.state_color(State::Delta), theme.primary);
        assert_eq!(theme.state_color(State::Read), theme.secondary);
        assert_eq!(theme.state_color(State::Search), theme.secondary);
        assert_eq!(theme.state_color(State::Agent), theme.primary);
    }

    #[test]
    fn no_color_is_greyscale_and_bold() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, true);
        assert!(theme.no_color);
        assert_eq!(theme.emphasis, Modifier::BOLD);
        for state in ALL_STATES {
            match theme.state_color(state) {
                Color::Rgb(r, g, b) => assert!(
                    r == g && g == b,
                    "{state:?} resolves to a non-grey color ({r},{g},{b})"
                ),
                other => panic!("{state:?} resolves to {other:?}"),
            }
        }
        assert_eq!(theme.border, Color::Rgb(0x5A, 0x5A, 0x5A));
        assert_eq!(theme.text, Color::Rgb(0xE6, 0xE6, 0xE6));
        assert_eq!(theme.primary, Color::Rgb(0xFF, 0xFF, 0xFF));
    }

    #[test]
    fn dim_drops_the_ramp_and_is_idempotent() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let dimmed = theme.dim();
        assert!(dimmed.dimmed);
        assert_ne!(dimmed.text, theme.text);
        assert_ne!(dimmed.muted, theme.muted);
        assert_ne!(dimmed.primary, theme.primary);
        assert_ne!(dimmed.border, theme.border);
        assert_eq!(dimmed.text, Color::Rgb(0x3F, 0x3A, 0x31));
        assert_eq!(dimmed.muted, Color::Rgb(0x5D, 0x56, 0x4C));
        assert_eq!(dimmed.primary, Color::Rgb(0x6B, 0x51, 0x2C));
        assert_eq!(dimmed.border, Color::Rgb(0x2B, 0x25, 0x19));
        assert_eq!(dimmed.dim(), dimmed);
    }

    #[test]
    fn dim_keeps_the_encoding_it_was_built_in() {
        assert!(matches!(
            Theme::da_vinci(ColorDepth::Ansi256, false).dim().text,
            Color::Indexed(_)
        ));
        assert!(matches!(
            Theme::da_vinci(ColorDepth::Basic, false).dim().text,
            Color::DarkGray
        ));
        assert!(Theme::da_vinci(ColorDepth::TrueColor, true).dim().no_color);
    }

    #[test]
    fn spinner_cycles_four_frames_and_freezes_when_still() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        let frames: Vec<&str> = (0..5).map(|t| theme.spinner(t, true)).collect();
        assert_eq!(frames, vec!["◜", "◝", "◞", "◟", "◜"]);
        assert_eq!(theme.spinner(7, false), "◉");
    }

    #[test]
    fn pie_is_a_proportion_not_a_number() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        assert_eq!(theme.pie(0.0), "◐");
        assert_eq!(theme.pie(0.21), "◐");
        assert_eq!(theme.pie(0.30), "◑");
        assert_eq!(theme.pie(0.64), "◒");
        assert_eq!(theme.pie(0.99), "◓");
        assert_eq!(theme.pie(1.0), "◓");
        assert_eq!(theme.pie(4.0), "◓");
    }
}
