//! The only place a color literal is allowed (`docs/ui/design.md` §2).
//!
//! Tokens are resolved at startup and when settings change into whatever the terminal understands,
//! so widgets pass `theme.primary` to a `Style` and never think about it.
//!
//! Neutral terminal surfaces and distinct semantic colors. The built-in dark
//! and light names keep their configuration identity while replacing the old
//! sepia defaults. Vox remains an explicit optional palette, not the default.
//! Exact Claude Code parity is tracked separately from these implementation tokens.

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
    pub const PROMPT: &str = "❯";
    pub const USER: &str = ">";
    pub const TICK: &str = "·";
    /// The elbow a tool line hangs from, so a turn's calls read as one branch
    /// under the agent mark rather than as loose rows.
    pub const BRANCH: &str = "⎿";
    /// Studio, collapsed to one line below 100 columns (design.md §6).
    pub const COLLAPSED: &str = "⟐";

    /// One 4-frame spinner, 250ms per frame (design.md §8).
    pub const SPINNER: [&str; 4] = ["◜", "◝", "◞", "◟"];
    /// Proportion pie used by the narrow status bar.
    pub const PIE: [&str; 4] = ["◐", "◑", "◒", "◓"];

    /// Proportion meter, drawn as filled run + tip + empty run (design.md §9).
    pub const METER_FILLED: &str = "━";
    pub const METER_TIP: &str = "◸";
    pub const METER_EMPTY: &str = "─";
}

/// Every state that has a glyph. Color is always secondary to this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum State {
    #[default]
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

/// Dark terminal palette: selection, success, warning and failure remain distinct.
const TRUECOLOR: Ramp = Ramp {
    background: rgb(0x1A1A1A),
    surface: rgb(0x242424),
    surface_alt: rgb(0x202020),
    border: rgb(0x666666),
    text: rgb(0xE6E6E6),
    muted: rgb(0xAAAAAA),
    primary: rgb(0xD97757),
    secondary: rgb(0x80BFFF),
    success: rgb(0x8CCF82),
    warning: rgb(0xE8C46A),
    error: rgb(0xFF8A8A),
};

/// Light terminal palette with readable semantic colors on neutral surfaces.
const LIGHT: Ramp = Ramp {
    background: rgb(0xFAFAFA),
    surface: rgb(0xF0F0F0),
    surface_alt: rgb(0xE8E8E8),
    border: rgb(0x808080),
    text: rgb(0x242424),
    muted: rgb(0x606060),
    primary: rgb(0xA9431E),
    secondary: rgb(0x205CB5),
    success: rgb(0x28632F),
    warning: rgb(0x785600),
    error: rgb(0xA92C2C),
};

const LIGHT_256: Ramp = Ramp {
    background: Color::Indexed(255),
    surface: Color::Indexed(254),
    surface_alt: Color::Indexed(253),
    border: Color::Indexed(244),
    text: Color::Indexed(235),
    muted: Color::Indexed(240),
    primary: Color::Indexed(88),
    secondary: Color::Indexed(25),
    success: Color::Indexed(22),
    warning: Color::Indexed(58),
    error: Color::Indexed(124),
};

const LIGHT_BASIC: Ramp = Ramp {
    background: Color::White,
    surface: Color::Gray,
    surface_alt: Color::White,
    border: Color::DarkGray,
    text: Color::Black,
    muted: Color::Black,
    primary: Color::Red,
    secondary: Color::Blue,
    success: Color::Blue,
    warning: Color::Red,
    error: Color::Red,
};

/// "Never blur, never tint — just drop the ramp" (design.md §2).
const TRUECOLOR_DIM: Ramp = Ramp {
    background: rgb(0x1A1A1A),
    surface: rgb(0x242424),
    surface_alt: rgb(0x202020),
    border: rgb(0x444444),
    text: rgb(0x888888),
    muted: rgb(0x777777),
    primary: rgb(0xA0A0A0),
    secondary: rgb(0x888888),
    success: rgb(0x888888),
    warning: rgb(0x888888),
    error: rgb(0x888888),
};

/// Indexed fallback with neutral surfaces and readable semantic colors.
const ANSI256: Ramp = Ramp {
    background: Color::Indexed(234),
    surface: Color::Indexed(235),
    surface_alt: Color::Indexed(234),
    border: Color::Indexed(242),
    text: Color::Indexed(254),
    muted: Color::Indexed(248),
    primary: Color::Indexed(216),
    secondary: Color::Indexed(117),
    success: Color::Indexed(114),
    warning: Color::Indexed(222),
    error: Color::Indexed(210),
};

const ANSI256_DIM: Ramp = Ramp {
    background: Color::Indexed(234),
    surface: Color::Indexed(235),
    surface_alt: Color::Indexed(234),
    border: Color::Indexed(238),
    text: Color::Indexed(244),
    muted: Color::Indexed(243),
    primary: Color::Indexed(247),
    secondary: Color::Indexed(244),
    success: Color::Indexed(244),
    warning: Color::Indexed(244),
    error: Color::Indexed(244),
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

/// Named terminal colors. Kept legible rather than faithful.
const BASIC: Ramp = Ramp {
    background: Color::Reset,
    surface: Color::Reset,
    surface_alt: Color::Reset,
    border: Color::DarkGray,
    text: Color::White,
    muted: Color::Gray,
    primary: Color::Yellow,
    secondary: Color::Gray,
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
    /// Saturated crimson identifies the optional editorial collage palette.
    pub fn is_vox(&self) -> bool {
        matches!(
            self.surface,
            Color::Rgb(141, 21, 15) | Color::Indexed(88) | Color::Red
        )
    }

    /// Apply the built-in setting while retaining terminal capabilities.
    /// Legacy/custom names retain the native dark palette.
    pub fn with_name(&self, name: &str) -> Self {
        if name == "vox" && !self.no_color {
            let ink = match self.depth_hint() {
                ColorDepth::TrueColor => rgb(0xE2BE9E),
                ColorDepth::Ansi256 => Color::Indexed(223),
                ColorDepth::Basic => Color::White,
            };
            return Self {
                background: match self.depth_hint() {
                    ColorDepth::TrueColor => rgb(0x1D1516),
                    ColorDepth::Ansi256 => Color::Indexed(233),
                    ColorDepth::Basic => Color::Reset,
                },
                surface_alt: match self.depth_hint() {
                    ColorDepth::TrueColor => rgb(0x182033),
                    ColorDepth::Ansi256 => Color::Indexed(234),
                    ColorDepth::Basic => Color::Reset,
                },
                border: match self.depth_hint() {
                    ColorDepth::TrueColor => rgb(0x9C6C4F),
                    ColorDepth::Ansi256 => Color::Indexed(137),
                    ColorDepth::Basic => Color::DarkGray,
                },
                primary: match self.depth_hint() {
                    ColorDepth::TrueColor => rgb(0xF3D90D),
                    ColorDepth::Ansi256 => Color::Indexed(220),
                    ColorDepth::Basic => Color::Yellow,
                },
                warning: match self.depth_hint() {
                    ColorDepth::TrueColor => rgb(0xF3D90D),
                    ColorDepth::Ansi256 => Color::Indexed(220),
                    ColorDepth::Basic => Color::Yellow,
                },
                text: ink,
                muted: ink,
                secondary: ink,
                success: ink,
                error: ink,
                surface: match self.depth_hint() {
                    ColorDepth::TrueColor => rgb(0x8D150F),
                    ColorDepth::Ansi256 => Color::Indexed(88),
                    ColorDepth::Basic => Color::Red,
                },
                ..Self::da_vinci(self.depth_hint(), false)
            };
        }
        if name != "light" {
            return Self::da_vinci(self.depth_hint(), self.no_color);
        }
        let ramp = match self.depth_hint() {
            ColorDepth::TrueColor => &LIGHT,
            ColorDepth::Ansi256 => &LIGHT_256,
            ColorDepth::Basic => &LIGHT_BASIC,
        };
        let light = Self::from_ramp(ramp, self.no_color, false);
        if self.no_color {
            let (paper, ink, surface) = match self.depth_hint() {
                ColorDepth::TrueColor => (rgb(0xE6E6E6), rgb(0x1C1C1C), rgb(0xCFCFCF)),
                ColorDepth::Ansi256 => (
                    Color::Indexed(254),
                    Color::Indexed(234),
                    Color::Indexed(252),
                ),
                ColorDepth::Basic => (Color::White, Color::Black, Color::Gray),
            };
            return Self {
                background: paper,
                surface,
                surface_alt: paper,
                text: ink,
                muted: ink,
                primary: ink,
                secondary: ink,
                success: ink,
                warning: ink,
                error: ink,
                border: ink,
                ..light
            };
        }
        light
    }

    fn is_light(&self) -> bool {
        matches!(
            self.background,
            Color::Rgb(250, 250, 250)
                | Color::Rgb(230, 230, 230)
                | Color::Indexed(255)
                | Color::Indexed(254)
                | Color::White
        )
    }

    /// Yellow headline scraps retain dark ink in both editorial variants.
    pub fn label_colors(&self, accent: bool) -> (Color, Color) {
        if self.is_light() && accent && !self.no_color {
            let paper = match self.depth_hint() {
                ColorDepth::TrueColor => TRUECOLOR.primary,
                ColorDepth::Ansi256 => ANSI256.primary,
                ColorDepth::Basic => Color::Yellow,
            };
            return (self.text, paper);
        }
        let ink = if self.background == Color::Reset {
            Color::Black
        } else {
            self.background
        };
        (ink, if accent { self.primary } else { self.text })
    }

    /// Model picker accent and selected-row fill, with terminal fallbacks.
    pub fn model_picker_colors(&self) -> (Color, Color) {
        if self.no_color {
            return (self.primary, self.surface);
        }
        (self.primary, self.surface)
    }

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
        if self.is_light() || self.is_vox() {
            return Self {
                text: self.muted,
                primary: self.muted,
                secondary: self.muted,
                success: self.muted,
                warning: self.muted,
                error: self.muted,
                emphasis: Modifier::empty(),
                dimmed: true,
                ..*self
            };
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

    /// The ink a run of code takes: keywords in blue-grey, strings in
    /// success, comments muted, numbers in warning, and everything else in
    /// the caller's `base` — the line's own colour in a Δ hunk, `text` in a
    /// fenced block. Colour reinforces; under `NO_COLOR` every role is the
    /// same ink and the code reads as it always did.
    pub fn syntax(&self, token: crate::davinci::views::highlight::Token, base: Color) -> Color {
        use crate::davinci::views::highlight::Token;
        match token {
            Token::Keyword => self.secondary,
            Token::String => self.success,
            Token::Comment => self.muted,
            Token::Number => self.warning,
            Token::Plain => base,
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

    #[test]
    fn vox_switches_and_preserves_terminal_capabilities() {
        for depth in [
            ColorDepth::TrueColor,
            ColorDepth::Ansi256,
            ColorDepth::Basic,
        ] {
            let dark = Theme::da_vinci(depth, false);
            let vox = dark.with_name("vox");
            assert!(vox.is_vox());
            assert_ne!(vox, dark);
            assert_eq!(vox.dim().background, vox.background);
            assert_eq!(vox.dim().surface, vox.surface);
            assert_eq!(vox.with_name("dark"), dark);
            let monochrome = Theme::da_vinci(depth, true);
            assert_eq!(monochrome.with_name("vox"), monochrome);
        }
    }

    #[test]
    fn light_theme_switches_back_without_losing_terminal_options() {
        for depth in [
            ColorDepth::TrueColor,
            ColorDepth::Ansi256,
            ColorDepth::Basic,
        ] {
            for no_color in [false, true] {
                let dark = Theme::da_vinci(depth, no_color);
                let light = dark.with_name("light");
                assert_ne!(light.background, dark.background);
                assert_eq!(light.no_color, no_color);
                assert_eq!(light.dim().background, light.background);
                assert_eq!(light.dim().dim(), light.dim());
                assert_eq!(light.with_name("dark"), dark);
            }
        }
    }

    #[test]
    fn readable_text_has_contrast_on_every_surface() {
        fn luminance(color: Color) -> f64 {
            let (r, g, b) = match color {
                Color::Rgb(r, g, b) => (r, g, b),
                Color::Indexed(index @ 16..=231) => {
                    let levels = [0, 95, 135, 175, 215, 255];
                    let index = (index - 16) as usize;
                    (levels[index / 36], levels[index / 6 % 6], levels[index % 6])
                }
                Color::Indexed(index @ 232..=255) => {
                    let value = 8 + (index - 232) * 10;
                    (value, value, value)
                }
                _ => panic!("expected an authored color"),
            };
            [r, g, b]
                .into_iter()
                .zip([0.2126, 0.7152, 0.0722])
                .map(|(value, weight)| {
                    let value = f64::from(value) / 255.0;
                    weight
                        * if value <= 0.04045 {
                            value / 12.92
                        } else {
                            ((value + 0.055) / 1.055).powf(2.4)
                        }
                })
                .sum()
        }
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
            for theme in [
                Theme::da_vinci(depth, false),
                Theme::da_vinci(depth, false).with_name("light"),
                Theme::da_vinci(depth, false).with_name("vox"),
            ] {
                for background in [theme.background, theme.surface, theme.surface_alt] {
                    for foreground in [
                        theme.text,
                        theme.muted,
                        theme.primary,
                        theme.secondary,
                        theme.success,
                        theme.warning,
                        theme.error,
                    ] {
                        let a = luminance(foreground);
                        let b = luminance(background);
                        let contrast = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                        assert!(
                            contrast >= 4.5,
                            "{foreground:?} on {background:?}: {contrast:.2}"
                        );
                    }
                }
            }
        }
    }

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
    fn neutral_defaults_have_distinct_semantic_roles() {
        let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
        assert_eq!(theme.background, rgb(0x1A1A1A));
        assert_eq!(theme.text, rgb(0xE6E6E6));
        assert_eq!(theme.muted, rgb(0xAAAAAA));
        assert_eq!(theme.primary, rgb(0xD97757));
        assert_eq!(theme.secondary, rgb(0x80BFFF));
        assert_eq!(theme.success, rgb(0x8CCF82));
        assert_eq!(theme.warning, rgb(0xE8C46A));
        assert_eq!(theme.error, rgb(0xFF8A8A));
        assert_ne!(theme.primary, theme.error);
        assert_ne!(theme.success, theme.text);
    }

    #[test]
    fn ansi256_preserves_neutral_surfaces_and_semantic_roles() {
        let theme = Theme::da_vinci(ColorDepth::Ansi256, false);
        assert_eq!(theme.primary, Color::Indexed(216));
        assert_eq!(theme.secondary, Color::Indexed(117));
        assert_eq!(theme.border, Color::Indexed(242));
        for surface in [theme.background, theme.surface, theme.surface_alt] {
            assert!(matches!(surface, Color::Indexed(232..=255)));
        }
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
        assert_eq!(State::Prompt.glyph(), "❯");
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
        assert_eq!(dimmed.text, rgb(0x888888));
        assert_eq!(dimmed.muted, rgb(0x777777));
        assert_eq!(dimmed.primary, rgb(0xA0A0A0));
        assert_eq!(dimmed.border, rgb(0x444444));
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
