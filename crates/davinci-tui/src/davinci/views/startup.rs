//! Compact welcome screen with workspace facts and useful entry points.
//!
//! Keeps startup discovery from the native shell; upstream interaction lives in
//! vendor/davinci/packages/coding-agent/src/modes/interactive/interactive-mode.ts.

use std::sync::OnceLock;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::davinci::model::{Model, Startup};
use crate::davinci::theme::Variant;
use crate::davinci::ui::{blank, clip_ellipsis, mix, span, truncate_run};

/// The masthead D, drawn in quadrant blocks on a 16×6 sub-cell grid: a
/// two-cell stem, hairline bars, and a bowl that rounds off through ▜ ▌ ▟
/// instead of meeting the stem at square corners. Every row is the same
/// eleven columns, so the text beside it lines up. Only an 8-color or
/// colorless terminal still draws it: the painter needs real colors.
pub const MARK: [&str; 3] = [" ██▀▀▀▜▄   ", " ██    █▌  ", " ██▄▄▄▟▀   "];

/// Terracotta lit from above: the top row catches the light, the bottom
/// sits in shade. Truecolor only; other palettes draw the flat accent.
fn mark_inks(model: &Model) -> [Color; 3] {
    let cc = model.theme.cc();
    const SHADE: Color = Color::Rgb(0x8E, 0x3F, 0x27);
    [
        mix(cc.claude, cc.claude_shimmer, 0.45),
        cc.claude,
        mix(cc.claude, SHADE, 0.4),
    ]
}

/// The painter's sprite is `PET_W` × `2 * PET_ROWS` pixels, two to a cell
/// through ▀ and ▄, so a pixel is square in a 1:2 terminal cell.
pub const PET_W: usize = 18;
pub const PET_ROWS: usize = 5;
/// One column of margin before the sprite, two between it and the text.
const PET_INDENT: &str = " ";
const PET_GAP: &str = "  ";
/// The column the masthead text starts in beside the painter.
pub const PET_TEXT_COLUMN: u16 = (1 + PET_W + 2) as u16;

/// The idle loop, one frame per 250ms tick: a one-pixel sink, a brush flick,
/// a blink and sparkles that never share a frame. Frame 0 is the rest pose,
/// the one shown whenever the painter is not playing. `.` is the terminal's
/// own background; every other letter is a key of [`pixel`].
const FRAMES: [[&str; 2 * PET_ROWS]; 8] = [
    // rest
    [
        "......BBBo........",
        "....bBBBBBBo...TT.",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S..",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        "....WKKKKKW..W....",
        ".LL..wAAAAwWW.....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // sparkle, left
    [
        "......BBBo........",
        ".+..bBBBBBBo...TT.",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S..",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        "....WKKKKKW..W....",
        ".LL..wAAAAwWW.....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // sink: head, beret and brush drop a pixel onto the apron
    [
        "..................",
        ".+....BBBo........",
        "....bBBBBBBo...TT.",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S..",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        ".LL.WKKKKKW..W....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // sink, brush flick
    [
        "..................",
        "......BBBo......TT",
        "....bBBBBBBo...S..",
        "...bBBBBBBBBo..S.+",
        "....WKKKKKKW...S..",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        ".LL.WKKKKKW..W....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // rise, flick held
    [
        "......BBBo......TT",
        "....bBBBBBBo...S..",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S.+",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        "....WKKKKKW..W....",
        ".LL..wAAAAwWW.....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // sparkle, top
    [
        "......BBBo...+....",
        "....bBBBBBBo...TT.",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S..",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        "....WKKKKKW..W....",
        ".LL..wAAAAwWW.....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // blink: the upper half of each eye goes dark
    [
        "......BBBo........",
        "....bBBBBBBo...TT.",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S..",
        "..gWKKKKKKKWg.S...",
        "..gWKEKKKEKWg.S...",
        "....WKKKKKW..W....",
        ".LL..wAAAAwWW.....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
    // rest
    [
        "......BBBo........",
        "....bBBBBBBo...TT.",
        "...bBBBBBBBBo..S..",
        "....WKKKKKKW...S..",
        "..gWKEKKKEKWg.S...",
        "..gWKEKKKEKWg.S...",
        "....WKKKKKW..W....",
        ".LL..wAAAAwWW.....",
        "LYQL.wAQRAw.......",
        ".lRl..W..W........",
    ],
];

/// Three loops after launch (6s), then the painter rests.
const STARTUP_TICKS: u64 = 3 * FRAMES.len() as u64;
/// A keystroke holds the painter still for a second, as it holds the caret.
const TYPING_HOLD_TICKS: u64 = 4;

/// The frame the painter shows now. It plays for the first loops after
/// launch and while a turn is working, and rests otherwise. It also rests
/// while the user types, while the window is in the background, and when
/// animation is off. A resting painter draws the same cells every tick, so
/// the shared tick's redraw writes nothing for it.
pub fn pet_frame(model: &Model) -> usize {
    let playing = model.tick < STARTUP_TICKS || model.working.is_some();
    let typing = model.caret_moved_at > 0
        && model.tick.wrapping_sub(model.caret_moved_at) < TYPING_HOLD_TICKS;
    if !playing || typing || !model.animate || !model.terminal_focused {
        return 0;
    }
    (model.tick % FRAMES.len() as u64) as usize
}

/// The sprite's colors on a dark ground. A light ground swaps the cream
/// shell for a darker one so the robot keeps its outline.
fn pixel(key: u8, light: bool) -> Option<(u8, u8, u8)> {
    Some(match key {
        b'B' => (0xE0, 0x6E, 0x3C),
        b'b' => (0xA8, 0x4A, 0x26),
        b'o' => (0xF0, 0x93, 0x5E),
        b'W' if light => (0xC9, 0xBB, 0xA5),
        b'W' => (0xEE, 0xE4, 0xD4),
        b'w' if light => (0x9C, 0x90, 0x7E),
        b'w' => (0xB4, 0xA8, 0x96),
        b'g' => (0x6E, 0x66, 0x5C),
        b'K' => (0x1C, 0x16, 0x14),
        b'E' => (0xF2, 0x7A, 0x3A),
        b'A' if light => (0xDC, 0xCF, 0xBC),
        b'A' => (0xF6, 0xEE, 0xE0),
        b'R' => (0xD8, 0x4A, 0x3A),
        b'Q' => (0x4A, 0x7E, 0xC8),
        b'Y' => (0xE8, 0xB8, 0x3A),
        b'S' => (0x8A, 0x52, 0x30),
        b'T' => (0xF5, 0x9A, 0x48),
        b'L' => (0xB0, 0x6A, 0x3A),
        b'l' => (0x7A, 0x44, 0x24),
        b'+' if light => (0xE0, 0x90, 0x58),
        b'+' => (0xF3, 0xC4, 0x9A),
        _ => return None,
    })
}

/// The nearest xterm-256 color: the 6×6×6 cube or the gray ramp.
fn xterm256((r, g, b): (u8, u8, u8)) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let dist = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    let level = |c: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(c)).abs())
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let gray_i = ((i32::from(r) + i32::from(g) + i32::from(b)) / 3 - 8)
        .clamp(0, 230)
        .wrapping_div(10) as u8;
    let gray = 8 + 10 * gray_i;
    if dist((gray, gray, gray)) < dist(cube) {
        232 + gray_i
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

#[derive(Clone, Copy)]
enum Palette {
    Dark,
    Light,
    Dark256,
    Light256,
}

/// Which palette the painter takes, or `None` for a terminal with fewer
/// than 256 colors, which keeps the one-ink D.
fn palette(model: &Model) -> Option<Palette> {
    let light = model.theme.variant == Variant::Light;
    match model.theme.cc().claude {
        Color::Rgb(..) if light => Some(Palette::Light),
        Color::Rgb(..) => Some(Palette::Dark),
        Color::Indexed(_) if light => Some(Palette::Light256),
        Color::Indexed(_) => Some(Palette::Dark256),
        _ => None,
    }
}

fn ink(key: u8, palette: Palette) -> Option<Color> {
    let light = matches!(palette, Palette::Light | Palette::Light256);
    let (r, g, b) = pixel(key, light)?;
    Some(match palette {
        Palette::Dark | Palette::Light => Color::Rgb(r, g, b),
        Palette::Dark256 | Palette::Light256 => Color::Indexed(xterm256((r, g, b))),
    })
}

/// Runs of one glyph, sliced from these so every span stays `'static`.
const TOPS: &str = "▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀";
const BOTTOMS: &str = "▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄";
const FULLS: &str = "██████████████████";
const SPACES: &str = "                  ";

/// One terminal row of one frame: each cell pairs two pixels, and cells that
/// draw alike are merged into one span. An empty pixel leaves the terminal's
/// own background, so the painter sits on any ground.
fn pet_row(frame: usize, row: usize, palette: Palette) -> Vec<Span<'static>> {
    let top = FRAMES[frame][2 * row].as_bytes();
    let bottom = FRAMES[frame][2 * row + 1].as_bytes();
    let cells = (0..PET_W).map(|x| match (ink(top[x], palette), ink(bottom[x], palette)) {
        (None, None) => (SPACES, Style::default()),
        (Some(t), None) => (TOPS, Style::default().fg(t)),
        (None, Some(b)) => (BOTTOMS, Style::default().fg(b)),
        (Some(t), Some(b)) if t == b => (FULLS, Style::default().fg(t)),
        (Some(t), Some(b)) => (TOPS, Style::default().fg(t).bg(b)),
    });
    let mut runs: Vec<(&'static str, Style, usize)> = Vec::new();
    for (glyphs, style) in cells {
        match runs.last_mut() {
            Some((g, s, n)) if *g == glyphs && *s == style => *n += 1,
            _ => runs.push((glyphs, style, 1)),
        }
    }
    let mut out = vec![Span::raw(PET_INDENT)];
    out.extend(runs.into_iter().map(|(glyphs, style, n)| {
        let step = glyphs.len() / PET_W;
        Span::styled(&glyphs[..step * n], style)
    }));
    out
}

type Sheet = [[Vec<Span<'static>>; PET_ROWS]; FRAMES.len()];

/// Every frame, built once per palette on first use. Each draw clones the
/// rows it shows, which copies style values and borrows the glyphs.
fn sheet(palette: Palette) -> &'static Sheet {
    static SHEETS: [OnceLock<Sheet>; 4] = [const { OnceLock::new() }; 4];
    SHEETS[palette as usize].get_or_init(|| {
        std::array::from_fn(|frame| std::array::from_fn(|row| pet_row(frame, row, palette)))
    })
}

/// The model and who serves it, by the names the catalog and the provider
/// registry give them, or by their raw ids before the shell resolves those.
fn model_and_provider(model: &Model) -> (&str, &str) {
    fn named<'a>(title: &'a str, id: &'a str) -> &'a str {
        if title.is_empty() {
            id
        } else {
            title
        }
    }
    (
        named(&model.model_title, &model.model_name),
        named(&model.provider_title, &model.active_provider),
    )
}

/// Compact identity block, also kept above a short conversation. The path and
/// selected model come from the running session, never from sample copy.
pub fn banner(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let th = &model.theme;
    let cc = th.cc();
    let mut name = span("DaVinci", th.text);
    name.style = name.style.add_modifier(Modifier::BOLD);
    let (title, provider) = model_and_provider(model);
    let mut serving = vec![span(title, th.text)];
    if !provider.is_empty() {
        serving.push(span(" • ", cc.inactive));
        serving.push(span(provider, cc.inactive));
    }
    // The art, the row its text starts on, and the column the text starts in.
    let (art, first, column): (Vec<Vec<Span<'static>>>, usize, u16) = match palette(model) {
        Some(palette) => (
            sheet(palette)[pet_frame(model)].to_vec(),
            1,
            PET_TEXT_COLUMN,
        ),
        None => (
            MARK.iter()
                .zip(mark_inks(model))
                .map(|(row, ink)| vec![span(*row, ink)])
                .collect(),
            0,
            11,
        ),
    };
    let pet = column == PET_TEXT_COLUMN;
    let mut text = [
        vec![
            name,
            span(format!(" v{}", env!("CARGO_PKG_VERSION")), cc.inactive),
        ],
        serving,
        // The whole path, as the shell has it: the folder the session works in.
        vec![span(
            clip_ellipsis(&info.cwd, model.width.saturating_sub(column + 1)),
            cc.inactive,
        )],
    ];
    let mut rows: Vec<Vec<Span<'static>>> = art
        .into_iter()
        .enumerate()
        .map(|(i, mut row)| {
            if let Some(words) = i.checked_sub(first).and_then(|i| text.get_mut(i)) {
                if pet {
                    row.push(Span::raw(PET_GAP));
                }
                row.append(words);
            }
            row
        })
        .collect();
    rows.push(vec![]);
    rows.push(vec![
        span("  Switch models anytime with ", th.text),
        span("/model", cc.permission),
        span(". Type ", th.text),
        span("?", cc.permission),
        span(" for shortcuts.", th.text),
    ]);
    rows.into_iter()
        .map(|row| Line::from(truncate_run(row, model.width)))
        .collect()
}

pub fn lines(model: &Model, info: &Startup) -> Vec<Line<'static>> {
    let mut out = vec![blank()];
    out.extend(banner(model, info));
    out
}

/// Row count includes discovered resources.
pub fn height(model: &Model) -> usize {
    lines(model, &model.startup).len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::model::Working;
    use crate::davinci::theme::{ColorDepth, Theme};
    use crate::davinci::ui::run_width;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::widgets::{Paragraph, Widget};

    fn model(width: u16) -> Model {
        Model::new(
            Theme::da_vinci(ColorDepth::TrueColor, false),
            width,
            52,
            true,
        )
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    /// The banner as the terminal would hold it.
    fn painted(m: &Model) -> Buffer {
        let rows = banner(m, &m.startup);
        let area = Rect::new(0, 0, m.width, rows.len() as u16);
        let mut buffer = Buffer::empty(area);
        Paragraph::new(rows).render(area, &mut buffer);
        buffer
    }

    /// Cells that differ between two paints: what a frame diff would write.
    fn changed(a: &Buffer, b: &Buffer) -> Vec<(u16, u16)> {
        let area = a.area;
        (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| a[(x, y)] != b[(x, y)])
            .collect()
    }

    #[test]
    fn welcome_is_bounded_and_reports_its_actual_height() {
        for width in [1, 40, 64, 80, 100, 160] {
            let mut m = model(width);
            m.startup.found = vec!["loaded 1 context file · 41 skills".into()];
            let rows = lines(&m, &m.startup);
            assert_eq!(rows.len(), height(&m));
            assert!(rows.iter().all(|row| run_width(&row.spans) <= width));
        }
    }

    #[test]
    fn welcome_matches_reference_masthead_with_model_hint() {
        let mut m = model(100);
        m.startup.restored = true;
        m.startup.found = vec!["loaded 1 context file · 41 skills".into()];
        let rows: Vec<String> = lines(&m, &m.startup).iter().map(text).collect();
        assert_eq!(rows.len(), 8);
        assert!(rows[0].is_empty());
        let drawn = rows.join("\n");
        assert!(drawn.contains("DaVinci"));
        assert!(drawn.contains(&m.model_name));
        assert!(drawn.contains(&m.startup.cwd));
        assert!(!drawn.contains("session restored"));
        assert!(!drawn.contains("new session"));
        assert!(!drawn.contains("/help"));
        assert!(drawn.contains("/model"));
    }

    #[test]
    fn every_frame_is_the_same_size_and_drawn_only_in_the_palette() {
        for (i, frame) in FRAMES.iter().enumerate() {
            for row in frame {
                assert_eq!(row.len(), PET_W, "frame {i}: {row:?}");
                for key in row.bytes() {
                    assert!(
                        key == b'.' || pixel(key, false).is_some(),
                        "frame {i} uses {:?}",
                        key as char
                    );
                }
            }
        }
        assert_eq!(FRAMES[0], FRAMES[7], "the loop starts and ends at rest");
        assert!(FRAMES.windows(2).take(6).all(|pair| pair[0] != pair[1]));
    }

    #[test]
    fn the_painter_sits_left_of_the_masthead_text_on_its_middle_rows() {
        use unicode_width::UnicodeWidthStr;
        let mut m = model(100);
        m.startup.cwd = "/work/davinci".into();
        m.model_name = "gpt-6-luna".into();
        let rows = banner(&m, &m.startup);
        assert_eq!(rows.len(), PET_ROWS + 2);
        for (i, row) in rows[..PET_ROWS].iter().enumerate() {
            let line = text(row);
            let art: String = line.chars().take(1 + PET_W).collect();
            assert_eq!(art.width(), 1 + PET_W, "row {i}: {line:?}");
            let words: String = line.chars().skip(PET_TEXT_COLUMN as usize).collect();
            assert_eq!(!words.is_empty(), (1..=3).contains(&i), "row {i}: {line:?}");
            if !words.is_empty() {
                let gap: String = line.chars().skip(1 + PET_W).take(2).collect();
                assert_eq!(gap, "  ", "row {i}: text starts at its column");
            } else {
                assert_eq!(line.width(), 1 + PET_W, "row {i}: no trailing gap");
            }
        }
        assert!(text(&rows[1]).ends_with(&format!("DaVinci v{}", env!("CARGO_PKG_VERSION"))));
        assert!(text(&rows[2]).contains(&m.model_name));
    }

    #[test]
    fn the_masthead_names_the_model_its_provider_and_the_whole_path() {
        let mut m = model(100);
        m.model_name = "gpt-6-luna".into();
        m.active_provider = "openai-codex".into();
        m.startup.cwd = r"C:\Users\sergi".into();
        let rows: Vec<String> = banner(&m, &m.startup).iter().map(text).collect();
        assert!(
            rows[2].ends_with("gpt-6-luna • openai-codex"),
            "{:?}",
            rows[2]
        );

        m.model_title = "GPT-6-Luna".into();
        m.provider_title = "OpenAI Codex".into();
        let rows: Vec<String> = banner(&m, &m.startup).iter().map(text).collect();
        assert!(
            rows[2].ends_with("GPT-6-Luna • OpenAI Codex"),
            "{:?}",
            rows[2]
        );
        assert!(rows[3].ends_with(r"C:\Users\sergi"), "{:?}", rows[3]);
        assert!(!rows.join("\n").contains("effort"));

        // A model with no provider reads as the model alone.
        m.provider_title.clear();
        m.active_provider.clear();
        let rows: Vec<String> = banner(&m, &m.startup).iter().map(text).collect();
        assert!(rows[2].ends_with("GPT-6-Luna"), "{:?}", rows[2]);
    }

    #[test]
    fn a_half_block_cell_carries_two_pixels_and_leaves_empty_ones_to_the_terminal() {
        // Rest frame, row 0: pixels 6..9 are beret over brim, 15 and 16 are
        // empty over the brush tip, 0 is empty over empty.
        let row = pet_row(0, 0, Palette::Dark);
        let cells: Vec<(String, Style)> = row[1..]
            .iter()
            .flat_map(|s| s.content.chars().map(|c| (c.to_string(), s.style)))
            .collect();
        assert_eq!(cells.len(), PET_W);
        assert_eq!(cells[0], (" ".into(), Style::default()));
        let tip = ink(b'T', Palette::Dark).unwrap();
        assert_eq!(cells[15], ("▄".into(), Style::default().fg(tip)));
        let beret = ink(b'B', Palette::Dark).unwrap();
        assert_eq!(cells[6], ("█".into(), Style::default().fg(beret)));
        let light = ink(b'o', Palette::Dark).unwrap();
        assert_eq!(cells[9], ("▀".into(), Style::default().fg(light).bg(beret)));
    }

    #[test]
    fn it_plays_after_launch_and_while_working_and_rests_otherwise() {
        let mut m = model(100);
        let frames: Vec<usize> = (0..8)
            .map(|t| {
                m.tick = t;
                pet_frame(&m)
            })
            .collect();
        assert_eq!(frames, (0..8).collect::<Vec<_>>());

        m.tick = STARTUP_TICKS + 3;
        assert_eq!(pet_frame(&m), 0, "idle after the opening loops");
        m.working = Some(Working::default());
        assert_eq!(pet_frame(&m), 3, "a working turn plays it again");
    }

    #[test]
    fn typing_unfocus_and_reduced_motion_each_hold_the_rest_pose() {
        let mut m = model(100);
        m.working = Some(Working::default());
        m.tick = 40;
        assert_eq!(pet_frame(&m), 0, "frame 40 % 8 is rest; step off it");
        m.tick = 42;
        assert_eq!(pet_frame(&m), 2);

        m.caret_moved_at = 41;
        assert_eq!(pet_frame(&m), 0, "typing holds it");
        m.tick = 41 + TYPING_HOLD_TICKS;
        assert_ne!(pet_frame(&m), 0, "a second after the last key it plays");

        m.terminal_focused = false;
        assert_eq!(pet_frame(&m), 0, "a background window holds it");
        m.terminal_focused = true;
        m.animate = false;
        assert_eq!(pet_frame(&m), 0, "reduced motion holds it");
    }

    #[test]
    fn a_tick_rewrites_only_cells_inside_the_painter() {
        let mut m = model(100);
        m.working = Some(Working::default());
        let mut changed_cells = 0;
        for tick in 0..8 {
            m.tick = tick;
            let before = painted(&m);
            m.tick = tick + 1;
            let after = painted(&m);
            for (x, y) in changed(&before, &after) {
                assert!(
                    x >= 1 && (x as usize) < 1 + PET_W && (y as usize) < PET_ROWS,
                    "tick {tick} changed ({x}, {y}) outside the painter"
                );
                changed_cells += 1;
            }
        }
        assert!(changed_cells > 0, "the loop moves");
    }

    #[test]
    fn a_resting_painter_costs_no_cells_on_a_tick() {
        let mut m = model(100);
        m.tick = STARTUP_TICKS;
        let before = painted(&m);
        m.tick += 1;
        assert!(changed(&before, &painted(&m)).is_empty());

        // Typing during a working turn: still no cell moves for the painter.
        m.working = Some(Working::default());
        m.caret_moved_at = m.tick;
        let before = painted(&m);
        m.tick += 1;
        assert!(changed(&before, &painted(&m)).is_empty());
    }

    #[test]
    fn the_sheet_is_built_once_per_palette() {
        let a = sheet(Palette::Dark) as *const Sheet;
        let b = sheet(Palette::Dark) as *const Sheet;
        assert_eq!(a, b);
        assert!(!std::ptr::eq(a, sheet(Palette::Light)));
    }

    #[test]
    fn a_256_color_terminal_gets_the_nearest_cube_colors() {
        assert_eq!(xterm256((0, 0, 0)), 16);
        assert_eq!(xterm256((255, 255, 255)), 231);
        assert_eq!(xterm256((0xE0, 0x6E, 0x3C)), 167, "beret orange");
        assert_eq!(xterm256((0x80, 0x80, 0x80)), 244, "mid gray uses the ramp");

        let flat = Model::new(Theme::da_vinci(ColorDepth::Ansi256, false), 100, 52, true);
        let rows = banner(&flat, &flat.startup);
        let art = rows[..PET_ROWS]
            .iter()
            .flat_map(|row| row.spans.iter())
            .filter(|s| s.content.chars().all(|c| "▀▄█ ".contains(c)));
        for span in art {
            for color in [span.style.fg, span.style.bg].into_iter().flatten() {
                assert!(matches!(color, Color::Indexed(_)), "{color:?}");
            }
        }
    }

    #[test]
    fn a_light_ground_darkens_the_shell_so_it_keeps_its_outline() {
        assert_ne!(ink(b'W', Palette::Dark), ink(b'W', Palette::Light));
        assert_eq!(ink(b'B', Palette::Dark), ink(b'B', Palette::Light));
    }

    #[test]
    fn an_eight_color_terminal_keeps_the_d_lit_flat() {
        let basic = Model::new(Theme::da_vinci(ColorDepth::Basic, false), 100, 52, true);
        let rows = banner(&basic, &basic.startup);
        assert_eq!(rows.len(), 5);
        assert!(text(&rows[0]).starts_with(MARK[0]));
        assert!(text(&rows[0]).contains("DaVinci"));
        let accent = Some(basic.theme.cc().claude);
        assert!(rows[..3].iter().all(|row| row.spans[0].style.fg == accent));

        let none = Model::new(Theme::da_vinci(ColorDepth::TrueColor, true), 100, 52, true);
        assert!(text(&banner(&none, &none.startup)[0]).starts_with(MARK[0]));
    }

    #[test]
    fn banner_uses_compact_identity_and_model_hint() {
        let m = model(100);
        let rows = banner(&m, &m.startup);
        let art = rows.iter().map(text).collect::<Vec<_>>().join("\n");
        assert_eq!(rows.len(), 7);
        assert!(art.contains("DaVinci"));
        assert!(art.contains(&m.model_name));
        assert!(!art.contains("▓"));
        assert!(!art.contains("CODE / TOOLS / CONTEXT"));
    }
}
