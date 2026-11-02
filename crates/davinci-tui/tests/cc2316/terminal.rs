//! P05: the `tui.reference` setting and the terminal modes it asks for.
use davinci_tui::davinci::runtime::{
    reference_enabled, reference_mouse_on, REFERENCE_MODES_OFF, REFERENCE_MODES_ON,
};
use davinci_tui::{interactive_settings_list, InteractiveSettingsConfig};

/// Every DEC private mode in `sequence` that ends in `final_byte`.
fn modes(sequence: &str, final_byte: char) -> Vec<u32> {
    sequence
        .split("\x1b[?")
        .skip(1)
        .filter_map(|part| {
            let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
            part[digits.len()..]
                .starts_with(final_byte)
                .then(|| digits.parse().unwrap())
        })
        .collect()
}

#[test]
fn the_reference_is_off_unless_asked_for() {
    assert!(!reference_enabled(None, false));
    assert!(reference_enabled(None, true));
}

#[test]
fn the_environment_overrides_the_setting_both_ways() {
    for on in ["1", "true", "YES", " on "] {
        assert!(reference_enabled(Some(on), false), "{on:?}");
    }
    for off in ["0", "false", "No", "off"] {
        assert!(!reference_enabled(Some(off), true), "{off:?}");
    }
}

#[test]
fn an_empty_or_unreadable_value_leaves_the_setting_alone() {
    for other in ["", "  ", "maybe"] {
        assert!(reference_enabled(Some(other), true), "{other:?}");
        assert!(!reference_enabled(Some(other), false), "{other:?}");
    }
}

#[test]
fn the_reference_asks_for_click_drag_move_and_sgr_mouse() {
    let mut on = modes(REFERENCE_MODES_ON, 'h');
    on.sort_unstable();
    assert_eq!(on, [1000, 1002, 1003, 1006]);
    assert!(modes(REFERENCE_MODES_ON, 'l').is_empty());
}

#[test]
fn every_mode_the_reference_turns_on_has_a_matching_off() {
    let mut on = modes(REFERENCE_MODES_ON, 'h');
    let mut off = modes(REFERENCE_MODES_OFF, 'l');
    assert!(
        off.contains(&2026),
        "a frame left open must not freeze the screen"
    );
    off.retain(|mode| *mode != 2026);
    on.sort_unstable();
    off.sort_unstable();
    assert_eq!(on, off);
    assert!(modes(REFERENCE_MODES_OFF, 'h').is_empty());
}

#[test]
fn nothing_is_written_with_the_setting_off_or_the_mouse_disabled() {
    assert_eq!(reference_mouse_on(false, true), "");
    assert_eq!(reference_mouse_on(true, false), "");
    assert_eq!(reference_mouse_on(true, true), REFERENCE_MODES_ON);
}

#[test]
fn config_lists_the_setting_off_and_says_when_it_applies() {
    let list = interactive_settings_list(&InteractiveSettingsConfig::default());
    let row = list
        .items
        .iter()
        .find(|item| item.id == "tui.reference")
        .expect("tui.reference is in /config");
    assert_eq!(row.current_value, "false");
    assert_eq!(row.values, ["true", "false"]);
    let text = format!(
        "{} {}",
        row.label,
        row.description.clone().unwrap_or_default()
    );
    assert!(text.contains("next start"), "{text}");
}
