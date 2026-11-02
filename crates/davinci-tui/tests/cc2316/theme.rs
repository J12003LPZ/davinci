//! P06 and P06b: the reference palette and the `/theme` picker.
//! Colour values were measured from the styled captures of the pinned build.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use davinci_tui::davinci::{
    app::{self, Flow},
    model::{Choice, Model},
    theme::{ColorDepth, Theme},
    views::theme_picker,
};
use ratatui::style::{Color, Modifier};

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

fn on(depth: ColorDepth) -> Theme {
    Theme::reference(depth, false)
}

#[test]
fn dark_truecolor_uses_the_measured_inks() {
    let cc = on(ColorDepth::TrueColor).cc();
    assert_eq!(cc.permission, rgb(0xB1B9F9));
    assert_eq!(cc.inactive, rgb(0x999999));
    assert_eq!(cc.prompt_border, rgb(0x888888));
    assert_eq!(cc.subtle, rgb(0x505050));
    assert_eq!(cc.success, rgb(0x4EBA65));
    assert_eq!(cc.auto_mode, rgb(0xFFC107));
    assert_eq!(cc.diff_text, rgb(0xF8F8F2));
    // Syntax on: darker tints than the 2.1.281 frames the base palette came from.
    assert_eq!(cc.diff_add_bg, rgb(0x022800));
    assert_eq!(cc.diff_add_word_bg, rgb(0x044700));
    assert_eq!(cc.diff_del_bg, rgb(0x3D0100));
    assert_eq!(cc.diff_del_word_bg, rgb(0x5C0200));
    assert_eq!(cc.diff_add, rgb(0x50C850));
    assert_eq!(cc.diff_del, rgb(0xDC5A5A));
    assert_eq!(cc.code_keyword, rgb(0x66D9EF));
    assert_eq!(cc.code_function, rgb(0xA6E22E));
    assert_eq!(cc.code_string, rgb(0xE6DB74));
    assert_eq!(cc.inactive_modifier, Modifier::empty());
}

#[test]
fn syntax_off_swaps_the_diff_tints_for_the_plain_ones() {
    let theme = on(ColorDepth::TrueColor);
    let plain = theme.diff_palette(false);
    assert_eq!(plain.diff_add_bg, rgb(0x225C2B));
    assert_eq!(plain.diff_add_word_bg, rgb(0x38A660));
    assert_eq!(plain.diff_del_bg, rgb(0x7A2936));
    assert_eq!(plain.diff_del_word_bg, rgb(0xB3596B));
    assert_eq!(plain.diff_text, rgb(0xFFFFFF));
    assert_eq!(theme.diff_palette(true), theme.cc());
}

#[test]
fn light_truecolor_uses_the_measured_inks() {
    let light = on(ColorDepth::TrueColor).with_name("light");
    assert!(light.reference);
    let cc = light.cc();
    assert_eq!(cc.permission, rgb(0x5769F7));
    assert_eq!(cc.inactive, rgb(0x666666));
    assert_eq!(cc.prompt_border, rgb(0x8A8A8A));
    assert_eq!(cc.subtle, rgb(0xAFAFAF));
    assert_eq!(cc.success, rgb(0x2C7A39));
    assert_eq!(cc.auto_mode, rgb(0x966C1E));
    assert_eq!(cc.user_bg, rgb(0xF0F0F0));
    // The reference paints no ground: the terminal's own light one shows.
    assert_eq!(light.background, Color::Reset);
    let plain = light.diff_palette(false);
    assert_eq!(plain.diff_add_bg, rgb(0x69DB7C));
    assert_eq!(plain.diff_add_word_bg, rgb(0x2F9D44));
    assert_eq!(plain.diff_del_bg, rgb(0xFFA8B4));
    assert_eq!(plain.diff_del_word_bg, rgb(0xD1454B));
    assert_eq!(plain.diff_text, rgb(0x000000));
}

#[test]
fn the_colorblind_theme_has_its_own_measured_inks() {
    let safe = on(ColorDepth::TrueColor).with_name("daltonized");
    assert!(safe.reference && safe.daltonized);
    let cc = safe.cc();
    // The mark's colour is DaVinci's choice and does not follow the capture.
    assert_eq!(cc.claude, on(ColorDepth::TrueColor).cc().claude);
    assert_eq!(cc.permission, rgb(0x99CCFF));
    assert_eq!(cc.success, rgb(0x3399FF));
    assert_eq!(cc.diff_add, rgb(0x51A0C8));
    assert_eq!(cc.diff_add_bg, rgb(0x001B29));
    assert_eq!(cc.diff_add_word_bg, rgb(0x003047));
    let plain = safe.diff_palette(false);
    assert_eq!(plain.diff_add_bg, rgb(0x004466));
    assert_eq!(plain.diff_add_word_bg, rgb(0x0077B3));
    assert_eq!(plain.diff_del_bg, rgb(0x660000));
    assert_eq!(plain.diff_del_word_bg, rgb(0xB30000));
}

#[test]
fn ansi_emits_only_named_colours_and_dims_the_quiet_text() {
    let theme = on(ColorDepth::TrueColor).with_name("ansi");
    assert!(theme.reference);
    let cc = theme.cc();
    assert_eq!(cc.inactive, Color::Gray);
    assert_eq!(cc.auto_mode, Color::LightYellow);
    assert_eq!(cc.inactive_modifier, Modifier::DIM);
    let colours = [
        theme.background,
        theme.surface,
        theme.surface_alt,
        theme.border,
        theme.text,
        theme.muted,
        theme.primary,
        theme.secondary,
        theme.success,
        theme.warning,
        theme.error,
        cc.claude,
        cc.claude_shimmer,
        cc.permission,
        cc.inactive,
        cc.inactive_shimmer,
        cc.subtle,
        cc.prompt_border,
        cc.text,
        cc.user_bg,
        cc.bash,
        cc.bash_bg,
        cc.success,
        cc.error,
        cc.auto_mode,
        cc.plan_mode,
        cc.accept_edits,
        cc.diff_text,
        cc.diff_add,
        cc.diff_add_bg,
        cc.diff_add_word_bg,
        cc.diff_del,
        cc.diff_del_bg,
        cc.diff_del_word_bg,
        cc.code_keyword,
        cc.code_string,
        cc.code_macro,
        cc.code_function,
        cc.selection_bg,
    ];
    for colour in colours.into_iter().chain(cc.category) {
        assert!(
            !matches!(colour, Color::Rgb(..) | Color::Indexed(_)),
            "{colour:?} is not one of the sixteen"
        );
    }
    let plain = theme.diff_palette(false);
    assert!(!matches!(
        plain.diff_add_bg,
        Color::Rgb(..) | Color::Indexed(_)
    ));
}

#[test]
fn a_256_colour_terminal_gets_no_truecolor() {
    let theme = on(ColorDepth::Ansi256);
    for cc in [
        theme.cc(),
        theme.with_name("light").cc(),
        theme.diff_palette(false),
    ] {
        for colour in [
            cc.claude,
            cc.permission,
            cc.diff_add_bg,
            cc.diff_add_word_bg,
            cc.code_keyword,
        ] {
            assert!(!matches!(colour, Color::Rgb(..)), "{colour:?}");
        }
    }
}

#[test]
fn no_color_has_no_reference_palette() {
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Basic,
    ] {
        let theme = Theme::reference(depth, true);
        assert_eq!(theme, Theme::da_vinci(depth, true), "{depth:?}");
        assert!(!theme.reference);
        assert!(!theme.with_name("light").reference);
        assert!(!theme.with_name("daltonized").reference);
        assert_eq!(theme.diff_palette(false), theme.cc());
    }
}

#[test]
fn an_explicit_non_reference_choice_keeps_its_own_palette() {
    let base = on(ColorDepth::TrueColor);
    // DaVinci's own theme and any name the shell does not model stay as today.
    // The session still wants the reference; the theme just doesn't use it.
    let own = |theme: Theme| Theme {
        wants_reference: true,
        ..theme
    };
    assert!(!base.with_name("vox").reference);
    assert_eq!(
        base.with_name("vox"),
        own(Theme::da_vinci(ColorDepth::TrueColor, false).with_name("vox"))
    );
    for custom in ["pi", "my-custom", ""] {
        let theme = base.with_name(custom);
        assert!(!theme.reference, "{custom:?}");
        assert_eq!(
            theme,
            own(Theme::da_vinci(ColorDepth::TrueColor, false)),
            "{custom:?}"
        );
    }
    // A built-in choice made while the reference is on keeps it.
    assert!(base.with_name("dark").reference);
    assert!(base.with_name("light-daltonized").reference);
    assert!(base.dim().reference);
}

#[test]
fn with_the_flag_off_the_da_vinci_palette_is_untouched() {
    let theme = Theme::da_vinci(ColorDepth::TrueColor, false);
    assert!(!theme.reference);
    assert!(!theme.with_name("light").reference);
    assert!(!theme.with_name("ansi").reference);
    let cc = theme.cc();
    assert_eq!(cc.diff_add_bg, rgb(0x225C2B));
    assert_eq!(cc.diff_del_bg, rgb(0x7A2936));
    assert_eq!(cc.prompt_border, rgb(0x888888));
    assert_eq!(theme.with_name("light").cc().prompt_border, rgb(0x999999));
    assert_eq!(cc.inactive_modifier, Modifier::empty());
    assert_eq!(theme.diff_palette(false), cc);
    assert_ne!(on(ColorDepth::TrueColor).cc(), cc);
    // The values the reference replaces, as they were before it existed.
    assert_eq!(cc.permission, rgb(0xB1B9F9));
    assert_eq!(cc.auto_mode, rgb(0xFFC107));
    assert_eq!(cc.diff_text, rgb(0xF8F8F2));
    assert_eq!(cc.code_keyword, Color::Blue);
    assert_eq!(cc.code_string, Color::Red);
    assert_eq!(cc.code_function, Color::Reset);
    assert_eq!(cc.diff_add_word_bg, Color::Reset);
    assert_eq!(cc.diff_del_word_bg, Color::Reset);
    // At every depth and for every name nothing of the reference leaks in.
    for depth in [
        ColorDepth::TrueColor,
        ColorDepth::Ansi256,
        ColorDepth::Basic,
    ] {
        let base = Theme::da_vinci(depth, false);
        for name in [
            "dark",
            "light",
            "vox",
            "ansi",
            "daltonized",
            "light-daltonized",
        ] {
            let cc = base.with_name(name).cc();
            assert_eq!(cc.inactive_modifier, Modifier::empty(), "{depth:?} {name}");
            assert_eq!(cc.code_function, Color::Reset, "{depth:?} {name}");
            assert_eq!(cc.diff_add_word_bg, Color::Reset, "{depth:?} {name}");
            assert_eq!(cc.diff_del_word_bg, Color::Reset, "{depth:?} {name}");
            assert!(!base.with_name(name).wants_reference, "{depth:?} {name}");
        }
    }
}

// ---- P06b: the picker ------------------------------------------------------

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn picker_model() -> Model {
    let mut model = Model::new(on(ColorDepth::TrueColor), 120, 46, false);
    model.reference = true;
    model.composer.set_text("a draft in progress");
    model
}

#[test]
fn the_picker_only_opens_for_the_reference() {
    let mut model = Model::new(
        Theme::da_vinci(ColorDepth::TrueColor, false),
        120,
        46,
        false,
    );
    assert!(!theme_picker::open(&mut model, "dark"));
    assert!(model.theme_picker.is_none());
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    let picker = model.theme_picker.as_ref().unwrap();
    assert_eq!(theme_picker::ENTRIES[picker.index].0, "dark");
}

#[test]
fn it_lists_only_themes_davinci_has() {
    let names: Vec<_> = theme_picker::ENTRIES.iter().map(|entry| entry.0).collect();
    assert_eq!(
        names,
        [
            "auto",
            "dark",
            "light",
            "vox",
            "daltonized",
            "light-daltonized",
            "ansi"
        ]
    );
    // Each one resolves to a palette; none is invented for the picker.
    let base = on(ColorDepth::TrueColor);
    for name in names {
        let _ = base.with_name(name);
    }
}

#[test]
fn moving_previews_each_theme_and_escape_restores_the_old_one() {
    let mut model = picker_model();
    let before = model.theme;
    assert!(theme_picker::open(&mut model, "dark"));
    assert_eq!(
        app::handle_key(&mut model, key(KeyCode::Down)),
        Flow::Continue
    );
    assert_eq!(model.theme, before.with_name("light"));
    assert_eq!(
        app::handle_key(&mut model, key(KeyCode::Down)),
        Flow::Continue
    );
    assert_eq!(model.theme, before.with_name("vox"));
    assert_eq!(
        app::handle_key(&mut model, key(KeyCode::Up)),
        Flow::Continue
    );
    assert_eq!(model.theme, before.with_name("light"));
    assert_eq!(
        app::handle_key(&mut model, key(KeyCode::Esc)),
        Flow::Continue
    );
    assert_eq!(model.theme, before);
    assert!(model.theme_picker.is_none());
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

#[test]
fn enter_applies_the_highlighted_theme_by_name() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    app::handle_key(&mut model, key(KeyCode::Down));
    let flow = app::handle_key(&mut model, key(KeyCode::Enter));
    assert_eq!(flow, Flow::Choose(Choice::Theme("light".into())));
    assert!(model.theme_picker.is_none());
    assert!(model.theme.reference);
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

#[test]
fn ctrl_t_toggles_the_syntax_line_inside_the_picker_only() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    assert!(model.theme_picker.as_ref().unwrap().syntax);
    let ctrl_t = KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL);
    app::handle_key(&mut model, ctrl_t);
    assert!(!model.theme_picker.as_ref().unwrap().syntax);
    let rows = theme_picker::lines(&model)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(
        rows.iter()
            .any(|row| row.contains("Syntax highlighting disabled")),
        "{rows:?}"
    );
    app::handle_key(&mut model, ctrl_t);
    let rows = theme_picker::lines(&model)
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    assert!(
        rows.iter().any(|row| row.contains("ctrl+t to disable")),
        "{rows:?}"
    );
}

#[test]
fn typed_characters_do_not_reach_the_composer_while_it_is_open() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    for ch in "hello /q".chars() {
        app::handle_key(&mut model, key(KeyCode::Char(ch)));
    }
    app::handle_key(&mut model, key(KeyCode::Backspace));
    app::handle_key(&mut model, key(KeyCode::Tab));
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
    assert!(model.theme_picker.is_some());
}

#[test]
fn a_resize_keeps_the_selection_and_the_draft() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    app::handle_key(&mut model, key(KeyCode::Down));
    app::handle_key(&mut model, key(KeyCode::Down));
    let index = model.theme_picker.as_ref().unwrap().index;
    for (width, height) in [(80, 24), (40, 12), (160, 50)] {
        model.width = width;
        model.height = height;
        let rows = theme_picker::lines(&model);
        assert!(!rows.is_empty());
        assert!(
            rows.iter().all(|row| row.width() <= usize::from(width)),
            "{width}x{height}"
        );
    }
    assert_eq!(model.theme_picker.as_ref().unwrap().index, index);
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

// ---- review round: input, palette memory, depth, preview, short screens -----

fn ctrl(ch: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
}

fn text_of(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

#[test]
fn a_paste_does_not_reach_the_composer_while_the_picker_is_open() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    model.paste("typed fast on windows\nand a second line");
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
    assert!(model.theme_picker.is_some());
    // Once it is closed the composer takes a paste again.
    app::handle_key(&mut model, key(KeyCode::Esc));
    model.paste("!");
    assert_eq!(model.composer.editor().get_text(), "a draft in progress!");
}

#[test]
fn the_reference_palette_returns_after_a_vox_start() {
    let tc = ColorDepth::TrueColor;
    let vox = Theme::da_vinci(tc, false)
        .with_name("vox")
        .with_reference(true);
    assert!(!vox.reference, "vox itself stays DaVinci's own");
    // The settings row cycling vox -> dark goes through `with_name`.
    let dark = vox.with_name("dark");
    assert!(dark.reference);
    assert_eq!(dark.cc(), on(tc).cc());
    let light = vox.with_name("light");
    assert!(light.reference);
    assert_eq!(light, on(tc).with_name("light"));
    // A custom name does not end the session's wish either.
    assert!(vox.with_name("pi").with_name("dark").reference);
    // Off stays off.
    let native = Theme::da_vinci(tc, false).with_name("vox");
    assert!(!native.with_name("dark").reference);
}

#[test]
fn picking_dark_in_the_picker_after_a_vox_start_gives_the_reference_palette() {
    let mut model = picker_model();
    model.theme = Theme::da_vinci(ColorDepth::TrueColor, false)
        .with_name("vox")
        .with_reference(true);
    assert!(theme_picker::open(&mut model, "vox"));
    app::handle_key(&mut model, key(KeyCode::Up));
    app::handle_key(&mut model, key(KeyCode::Up));
    assert_eq!(
        theme_picker::ENTRIES[model.theme_picker.as_ref().unwrap().index].0,
        "dark"
    );
    assert!(model.theme.reference, "the preview");
    let flow = app::handle_key(&mut model, key(KeyCode::Enter));
    assert_eq!(flow, Flow::Choose(Choice::Theme("dark".into())));
    assert!(model.theme.reference, "the applied theme");
    assert_eq!(model.theme.cc(), on(ColorDepth::TrueColor).cc());
}

#[test]
fn leaving_ansi_returns_to_the_terminals_own_depth() {
    for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256] {
        let ansi = on(depth).with_name("ansi");
        for name in ["dark", "light", "daltonized", "light-daltonized", "vox"] {
            assert_eq!(
                ansi.with_name(name),
                on(depth).with_name(name),
                "{depth:?} {name}"
            );
        }
    }
}

#[test]
fn the_picker_previews_and_applies_at_the_terminals_depth_after_ansi() {
    let mut model = picker_model();
    model.theme = on(ColorDepth::TrueColor).with_name("ansi");
    assert!(theme_picker::open(&mut model, "ansi"));
    app::handle_key(&mut model, key(KeyCode::Up));
    assert!(
        matches!(model.theme.cc().permission, Color::Rgb(..)),
        "light-daltonized previews in colour"
    );
    app::handle_key(&mut model, key(KeyCode::Up));
    assert_eq!(
        theme_picker::ENTRIES[model.theme_picker.as_ref().unwrap().index].0,
        "daltonized"
    );
    assert_eq!(model.theme.cc().permission, rgb(0x99CCFF), "the preview");
    app::handle_key(&mut model, key(KeyCode::Up));
    app::handle_key(&mut model, key(KeyCode::Up));
    app::handle_key(&mut model, key(KeyCode::Up));
    assert_eq!(
        theme_picker::ENTRIES[model.theme_picker.as_ref().unwrap().index].0,
        "dark"
    );
    app::handle_key(&mut model, key(KeyCode::Enter));
    assert_eq!(model.theme, on(ColorDepth::TrueColor).with_name("dark"));
}

#[test]
fn without_the_setting_ansi_still_sticks_and_no_color_still_wins() {
    let tc = ColorDepth::TrueColor;
    // Today's behaviour, unchanged: the flag off leaves no way back from ansi.
    let stuck = Theme::da_vinci(tc, false)
        .with_name("ansi")
        .with_name("dark");
    assert_eq!(stuck, Theme::da_vinci(ColorDepth::Basic, false));
    let plain = Theme::reference(tc, true)
        .with_name("ansi")
        .with_name("dark");
    assert!(plain.no_color);
    assert!(!plain.reference);
}

#[test]
fn ctrl_c_closes_the_picker_like_escape() {
    let mut model = picker_model();
    let before = model.theme;
    assert!(theme_picker::open(&mut model, "dark"));
    app::handle_key(&mut model, key(KeyCode::Down));
    assert_ne!(model.theme, before);
    // Not an interrupt: the host would count it towards quitting.
    assert_eq!(app::handle_key(&mut model, ctrl('c')), Flow::Continue);
    assert!(model.theme_picker.is_none());
    assert_eq!(model.theme, before);
    assert_eq!(model.composer.editor().get_text(), "a draft in progress");
}

#[test]
fn a_stored_theme_the_list_lacks_shows_no_check_mark() {
    for stored in ["pi", "my-custom"] {
        let mut model = picker_model();
        let before = model.theme;
        assert!(theme_picker::open(&mut model, stored));
        let picker = model.theme_picker.as_ref().unwrap();
        assert_eq!(theme_picker::ENTRIES[picker.index].0, "dark", "highlight");
        let rows = text_of(&theme_picker::lines(&model));
        assert!(rows.iter().all(|row| !row.contains('✔')), "{rows:?}");
        app::handle_key(&mut model, key(KeyCode::Down));
        app::handle_key(&mut model, key(KeyCode::Esc));
        assert_eq!(model.theme, before, "esc restores the real theme");
    }
    // A listed name keeps its mark.
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "light"));
    let rows = text_of(&theme_picker::lines(&model));
    assert_eq!(rows.iter().filter(|row| row.contains('✔')).count(), 1);
}

#[test]
fn the_preview_sample_is_davincis_own_with_four_rows() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    let rows = text_of(&theme_picker::lines(&model));
    let all = rows.join("\n");
    for borrowed in ["greet", "console", "Hello", "World", "function"] {
        assert!(!all.contains(borrowed), "{borrowed:?} is the reference's");
    }
    let shape: Vec<_> = ["1 ", "2 -", "2 +", "3 "]
        .iter()
        .map(|gutter| {
            rows.iter()
                .position(|row| row.trim_start().starts_with(gutter))
                .unwrap_or_else(|| panic!("no row starting {gutter:?}: {rows:?}"))
        })
        .collect();
    assert!(
        shape.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "{shape:?}"
    );
}

#[test]
fn the_changed_rows_are_tinted_to_the_edge_at_any_width() {
    for width in [60u16, 80, 120, 200] {
        let mut model = picker_model();
        model.width = width;
        assert!(theme_picker::open(&mut model, "dark"));
        let lines = theme_picker::lines(&model);
        for gutter in ["2 -", "2 +"] {
            let line = lines
                .iter()
                .find(|line| line.to_string().trim_start().starts_with(gutter))
                .unwrap();
            assert_eq!(line.width(), usize::from(width), "{gutter} at {width}");
        }
    }
}

fn picker_on(theme: Theme) -> Model {
    let mut model = picker_model();
    model.theme = theme;
    assert!(theme_picker::open(&mut model, "dark"));
    model
}

/// The preview's row that starts with `gutter`, as its spans.
fn preview_row(model: &Model, gutter: &str) -> Vec<ratatui::text::Span<'static>> {
    theme_picker::lines(model)
        .into_iter()
        .find(|line| line.to_string().trim_start().starts_with(gutter))
        .unwrap_or_else(|| panic!("no preview row {gutter:?}"))
        .spans
}

#[test]
fn a_changed_word_keeps_the_rows_tint_where_its_own_is_unmeasured() {
    let themes = [
        ("256 dark", on(ColorDepth::Ansi256), true),
        (
            "256 light",
            on(ColorDepth::Ansi256).with_name("light"),
            true,
        ),
        ("light", on(ColorDepth::TrueColor).with_name("light"), true),
        ("ansi", on(ColorDepth::TrueColor).with_name("ansi"), false),
    ];
    for (label, theme, tinted) in themes {
        let model = picker_on(theme);
        for gutter in ["2 -", "2 +"] {
            // indent, gutter, text before, the changed word, text after, fill
            let spans = preview_row(&model, gutter);
            let row_bg = spans[1].style.bg;
            assert!(spans.len() >= 6, "{spans:?}");
            for span in &spans[1..] {
                assert_eq!(span.style.bg, row_bg, "{label} {gutter}: {span:?}");
            }
            if tinted {
                assert_ne!(row_bg, Some(Color::Reset), "{label} {gutter}");
            }
        }
    }
}

#[test]
fn an_unmeasured_function_ink_is_the_diff_text() {
    // indent, number, keyword, space, function name, signature
    let name = |theme: Theme| preview_row(&picker_on(theme), "1 ")[4].style.fg;
    for theme in [
        on(ColorDepth::Ansi256),
        on(ColorDepth::TrueColor).with_name("light"),
        on(ColorDepth::TrueColor).with_name("ansi"),
    ] {
        assert_eq!(name(theme), Some(theme.cc().diff_text));
        assert_ne!(name(theme), Some(Color::Reset));
    }
    assert_eq!(name(on(ColorDepth::TrueColor)), Some(rgb(0xA6E22E)));
}

fn composed(model: &Model, height: u16) -> Vec<String> {
    text_of(&app::compose_frame(model, height).lines)
}

#[test]
fn a_short_terminal_keeps_the_highlighted_theme_and_the_hint() {
    for height in [10u16, 4] {
        for index in 0..theme_picker::ENTRIES.len() {
            let mut model = picker_model();
            model.height = height;
            assert!(theme_picker::open(&mut model, "dark"));
            model.theme_picker.as_mut().unwrap().index = index;
            let rows = composed(&model, height);
            assert_eq!(rows.len(), usize::from(height));
            let label = theme_picker::ENTRIES[index].1;
            assert!(
                rows.iter()
                    .any(|row| row.contains('❯') && row.contains(label)),
                "h={height} index={index}: {rows:?}"
            );
            assert!(
                rows.iter().any(|row| row.contains("Enter to select")),
                "h={height} index={index}: {rows:?}"
            );
        }
    }
}

#[test]
fn the_preview_goes_first_then_the_blank_rows() {
    let mut model = picker_model();
    assert!(theme_picker::open(&mut model, "dark"));
    let full = composed(&model, 30);
    assert!(full.iter().any(|row| row.contains("Syntax theme")));
    // Too short for the preview: it goes whole, every theme stays.
    let rows = composed(&model, 16);
    assert!(rows.iter().all(|row| !row.contains("Syntax")), "{rows:?}");
    for (_, label) in theme_picker::ENTRIES {
        assert!(rows.iter().any(|row| row.contains(label)), "{label}");
    }
    assert!(rows.iter().any(|row| row.contains("Enter to select")));
    // Shorter still: the blank rows go before any theme does.
    let rows = composed(&model, 11);
    for (_, label) in theme_picker::ENTRIES {
        assert!(rows.iter().any(|row| row.contains(label)), "{label}");
    }
    assert!(rows.iter().all(|row| !row.trim().is_empty()), "{rows:?}");
    // Nothing panics on a screen too small to be useful.
    for height in 0..=3u16 {
        let rows = composed(&model, height);
        assert!(rows.len() <= usize::from(height), "h={height}");
    }
}
