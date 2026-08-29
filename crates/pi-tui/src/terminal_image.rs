//! Terminal image protocol matching TypeScript `packages/tui/src/terminal-image.ts`.

use std::sync::{Mutex, OnceLock};

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use indexmap::IndexMap;

pub const KITTY_PREFIX: &str = "\x1b_G";
pub const ITERM2_PREFIX: &str = "\x1b]1337;File=";

const KITTY_CHUNK: usize = 4096;
const KITTY_PLACEMENT_CONTROL_KEYS: &[&str] = &[
    "i", "p", "x", "y", "w", "h", "X", "Y", "c", "r", "C", "U", "z", "P", "Q", "H", "V",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageProtocol {
    Kitty,
    ITerm2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalCapabilities {
    pub images: Option<ImageProtocol>,
    pub true_color: bool,
    pub hyperlinks: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageCellSize {
    pub columns: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KittyImageMetadata {
    pub image_id: u32,
    pub columns: u32,
    pub rows: u32,
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImagePlacement {
    pub image_id: u32,
    pub transmission_generation: u32,
    pub transmission_bytes: usize,
    pub estimated_decoded_bytes: u32,
    pub sequence: String,
    pub replacement_line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedImage {
    pub sequence: String,
    pub columns: u32,
    pub rows: u32,
    pub image_id: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct ImageRenderOptions {
    pub max_width_cells: Option<u32>,
    pub max_height_cells: Option<u32>,
    pub preserve_aspect_ratio: Option<bool>,
    pub image_id: Option<u32>,
    pub move_cursor: Option<bool>,
}

#[derive(Clone, Copy)]
struct RegisteredKittyImageMetadata {
    meta: KittyImageMetadata,
    transmission_generation: u32,
}

struct ImageState {
    cached: Option<TerminalCapabilities>,
    overrides: CapabilityOverrides,
    cell: CellDimensions,
    kitty_metadata: IndexMap<u32, RegisteredKittyImageMetadata>,
    kitty_generation: u32,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct CapabilityOverrides {
    images: Option<Option<ImageProtocol>>,
    true_color: Option<bool>,
    hyperlinks: Option<bool>,
}

impl Default for ImageState {
    fn default() -> Self {
        Self {
            cached: None,
            overrides: CapabilityOverrides::default(),
            cell: CellDimensions {
                width_px: 9,
                height_px: 18,
            },
            kitty_metadata: IndexMap::new(),
            kitty_generation: 0,
        }
    }
}

fn state() -> std::sync::MutexGuard<'static, ImageState> {
    static STATE: OnceLock<Mutex<ImageState>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(ImageState::default()))
        .lock()
        .expect("terminal image state")
}

pub fn is_image_line(line: &str) -> bool {
    line.contains(KITTY_PREFIX) || line.contains(ITERM2_PREFIX)
}

pub fn encode_kitty(
    base64_data: &str,
    columns: Option<u32>,
    rows: Option<u32>,
    image_id: Option<u32>,
    move_cursor: Option<bool>,
) -> String {
    let mut params = vec!["a=T".to_string(), "f=100".to_string(), "q=2".to_string()];
    if move_cursor == Some(false) {
        params.push("C=1".into());
    }
    if let Some(columns) = columns.filter(|c| *c != 0) {
        params.push(format!("c={columns}"));
    }
    if let Some(rows) = rows.filter(|r| *r != 0) {
        params.push(format!("r={rows}"));
    }
    if let Some(image_id) = image_id.filter(|id| *id != 0) {
        params.push(format!("i={image_id}"));
    }
    if base64_data.len() <= KITTY_CHUNK {
        return format!("\x1b_G{};{base64_data}\x1b\\", params.join(","));
    }
    let mut chunks = Vec::new();
    let mut offset = 0;
    let mut is_first = true;
    while offset < base64_data.len() {
        let end = (offset + KITTY_CHUNK).min(base64_data.len());
        let chunk = &base64_data[offset..end];
        let is_last = end >= base64_data.len();
        if is_first {
            chunks.push(format!("\x1b_G{},m=1;{chunk}\x1b\\", params.join(",")));
            is_first = false;
        } else if is_last {
            chunks.push(format!("\x1b_Gm=0;{chunk}\x1b\\"));
        } else {
            chunks.push(format!("\x1b_Gm=1;{chunk}\x1b\\"));
        }
        offset = end;
    }
    chunks.join("")
}

pub fn delete_kitty_image(image_id: u32) -> String {
    format!("\x1b_Ga=d,d=I,i={image_id},q=2\x1b\\")
}

pub fn delete_all_kitty_images() -> String {
    "\x1b_Ga=d,d=A,q=2\x1b\\".into()
}

pub fn delete_all_kitty_placements() -> String {
    "\x1b_Ga=d,d=a,q=2\x1b\\".into()
}

fn base64_decoded_len(data: &str) -> usize {
    let padding = data
        .as_bytes()
        .iter()
        .rev()
        .take_while(|b| **b == b'=')
        .count();
    data.len() / 4 * 3 - padding
}

pub fn encode_iterm2(
    base64_data: &str,
    width: Option<&str>,
    height: Option<&str>,
    name: Option<&str>,
    preserve_aspect_ratio: Option<bool>,
    inline: Option<bool>,
) -> String {
    let mut params = vec![
        format!("inline={}", if inline == Some(false) { 0 } else { 1 }),
        format!("size={}", base64_decoded_len(base64_data)),
    ];
    if let Some(width) = width {
        params.push(format!("width={width}"));
    }
    if let Some(height) = height {
        params.push(format!("height={height}"));
    }
    if let Some(name) = name {
        params.push(format!("name={}", STANDARD.encode(name.as_bytes())));
    }
    if preserve_aspect_ratio == Some(false) {
        params.push("preserveAspectRatio=0".into());
    }
    format!("\x1b]1337;File={}:{base64_data}\x07", params.join(";"))
}

pub fn register_kitty_image_metadata(metadata: KittyImageMetadata) {
    let mut state = state();
    state.kitty_generation += 1;
    let generation = state.kitty_generation;
    state.kitty_metadata.shift_remove(&metadata.image_id);
    state.kitty_metadata.insert(
        metadata.image_id,
        RegisteredKittyImageMetadata {
            meta: metadata,
            transmission_generation: generation,
        },
    );
    if state.kitty_metadata.len() > 1000 {
        state.kitty_metadata.shift_remove_index(0);
    }
}

fn registered_for_line(line: &str) -> Option<RegisteredKittyImageMetadata> {
    let controls = kitty_controls(line)?;
    let image_id: u32 = control_value(&controls, "i")?.parse().ok()?;
    state().kitty_metadata.get(&image_id).copied()
}

fn kitty_controls(line: &str) -> Option<String> {
    let start = line.find(KITTY_PREFIX)?;
    let after = &line[start + KITTY_PREFIX.len()..];
    let end = after.find(';')?;
    Some(after[..end].to_string())
}

fn control_value(controls: &str, key: &str) -> Option<String> {
    controls.split(',').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

pub fn get_kitty_image_metadata(line: &str) -> Option<KittyImageMetadata> {
    Some(registered_for_line(line)?.meta)
}

pub fn get_kitty_image_placement(line: &str) -> Option<KittyImagePlacement> {
    let match_start = line.find(KITTY_PREFIX)?;
    let controls = kitty_controls(line)?;
    let metadata = registered_for_line(line)?;
    let mut command_start = match_start;
    let mut command_controls = controls.clone();
    let mut transmission_end;
    loop {
        let search = command_start + KITTY_PREFIX.len();
        let terminator = line[search..].find("\x1b\\")?;
        transmission_end = search + terminator + 2;
        if control_value(&command_controls, "m")
            .as_deref()
            .is_none_or(|m| m != "1")
        {
            break;
        }
        command_start = transmission_end;
        if !line[command_start..].starts_with(KITTY_PREFIX) {
            return None;
        }
        let after = &line[command_start + KITTY_PREFIX.len()..];
        let controls_end = after.find(';')?;
        command_controls = after[..controls_end].to_string();
    }
    let placement_controls: Vec<&str> = controls
        .split(',')
        .filter(|control| {
            let key = control.split('=').next().unwrap_or("");
            KITTY_PLACEMENT_CONTROL_KEYS.contains(&key)
        })
        .collect();
    let sequence = format!("\x1b_Ga=p,q=2,{}\x1b\\", placement_controls.join(","));
    let replacement_line = format!(
        "{}{}{}",
        &line[..match_start],
        sequence,
        &line[transmission_end..]
    );
    Some(KittyImagePlacement {
        image_id: metadata.meta.image_id,
        transmission_generation: metadata.transmission_generation,
        transmission_bytes: transmission_end - match_start,
        estimated_decoded_bytes: metadata.meta.width_px * metadata.meta.height_px * 4,
        sequence,
        replacement_line,
    })
}

pub fn crop_kitty_image_line(line: &str, hidden_rows: u32, visible_rows: u32) -> String {
    let Some(metadata) = get_kitty_image_metadata(line) else {
        return line.to_string();
    };
    let Some(match_start) = line.find(KITTY_PREFIX) else {
        return line.to_string();
    };
    if hidden_rows >= metadata.rows || visible_rows == 0 {
        return line.to_string();
    }
    let cropped_rows = visible_rows.min(metadata.rows - hidden_rows);
    if hidden_rows == 0 && cropped_rows == metadata.rows {
        return line.to_string();
    }
    let source_y = metadata.height_px * hidden_rows / metadata.rows;
    let source_end = (metadata.height_px * (hidden_rows + cropped_rows)).div_ceil(metadata.rows);
    let source_height = 1.max(metadata.height_px.min(source_end).saturating_sub(source_y));
    let Some(controls) = kitty_controls(line) else {
        return line.to_string();
    };
    let mut next: Vec<String> = controls
        .split(',')
        .filter(|control| {
            !(control.starts_with("y=") || control.starts_with("h=") || control.starts_with("r="))
        })
        .map(str::to_string)
        .collect();
    next.push(format!("y={source_y}"));
    next.push(format!("h={source_height}"));
    next.push(format!("r={cropped_rows}"));
    let match_end = match_start + KITTY_PREFIX.len() + controls.len() + 1;
    format!(
        "{}\x1b_G{};{}",
        &line[..match_start],
        next.join(","),
        &line[match_end..]
    )
}

pub fn calculate_image_cell_size(
    image: ImageDimensions,
    max_width_cells: u32,
    max_height_cells: Option<u32>,
    cell: CellDimensions,
) -> ImageCellSize {
    let max_width = max_width_cells.max(1);
    let max_height = max_height_cells.map(|h| h.max(1));
    let image_width = image.width_px.max(1) as f64;
    let image_height = image.height_px.max(1) as f64;
    let width_scale = (max_width as f64 * cell.width_px as f64) / image_width;
    let height_scale = match max_height {
        Some(max_height) => (max_height as f64 * cell.height_px as f64) / image_height,
        None => width_scale,
    };
    let scale = width_scale.min(height_scale);
    let columns = ((image_width * scale) / cell.width_px as f64).ceil() as u32;
    let rows = ((image_height * scale) / cell.height_px as f64).ceil() as u32;
    ImageCellSize {
        columns: columns.max(1).min(max_width),
        rows: match max_height {
            Some(max_height) => rows.max(1).min(max_height),
            None => rows.max(1),
        },
    }
}

pub fn get_cell_dimensions() -> CellDimensions {
    state().cell
}

pub fn set_cell_dimensions(dims: CellDimensions) {
    state().cell = dims;
}

pub fn detect_capabilities(tmux_forwards_hyperlink: Option<fn() -> bool>) -> TerminalCapabilities {
    let hyperlinks_override = parse_bool_override(std::env::var("PI_HYPERLINKS").ok().as_deref());
    let probe = if hyperlinks_override.is_some() {
        None
    } else {
        tmux_forwards_hyperlink
    };
    let forwards = probe.map(|probe| probe()).unwrap_or(false);
    let mut detected = detect_from_environment(forwards);
    if let Some(hyperlinks) = hyperlinks_override {
        detected.hyperlinks = hyperlinks;
    }
    if let Ok(protocol) = std::env::var("PI_IMAGE_PROTOCOL") {
        match protocol.to_ascii_lowercase().as_str() {
            "kitty" => detected.images = Some(ImageProtocol::Kitty),
            "iterm2" => detected.images = Some(ImageProtocol::ITerm2),
            "none" | "0" => detected.images = None,
            _ => {}
        }
    }
    if let Some(true_color) = parse_bool_override(std::env::var("PI_TRUE_COLOR").ok().as_deref()) {
        detected.true_color = true_color;
    }
    detected
}

fn parse_bool_override(value: Option<&str>) -> Option<bool> {
    match value {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    }
}

fn env_lower(key: &str) -> String {
    std::env::var(key).unwrap_or_default().to_ascii_lowercase()
}

fn env_present(key: &str) -> bool {
    std::env::var(key).ok().filter(|v| !v.is_empty()).is_some()
}

fn detect_from_environment(tmux_forwards_hyperlink: bool) -> TerminalCapabilities {
    let term_program = env_lower("TERM_PROGRAM");
    let terminal_emulator = env_lower("TERMINAL_EMULATOR");
    let term = env_lower("TERM");
    let color_term = env_lower("COLORTERM");
    let has_true_color_hint = color_term == "truecolor" || color_term == "24bit";
    if env_present("TMUX") || term.starts_with("tmux") {
        return TerminalCapabilities {
            images: None,
            true_color: has_true_color_hint,
            hyperlinks: tmux_forwards_hyperlink,
        };
    }
    if term.starts_with("screen") {
        return TerminalCapabilities {
            images: None,
            true_color: has_true_color_hint,
            hyperlinks: false,
        };
    }
    if env_present("KITTY_WINDOW_ID") || term_program == "kitty" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }
    if term_program == "ghostty" || term.contains("ghostty") || env_present("GHOSTTY_RESOURCES_DIR")
    {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }
    if env_present("WEZTERM_PANE") || term_program == "wezterm" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }
    if term_program == "warpterminal"
        || env_present("WARP_SESSION_ID")
        || env_present("WARP_TERMINAL_SESSION_UUID")
    {
        return TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
    }
    if env_present("ITERM_SESSION_ID") || term_program == "iterm.app" {
        return TerminalCapabilities {
            images: Some(ImageProtocol::ITerm2),
            true_color: true,
            hyperlinks: true,
        };
    }
    if env_present("WT_SESSION") {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: true,
        };
    }
    if term_program == "vscode" || term_program == "alacritty" {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: true,
        };
    }
    if terminal_emulator == "jetbrains-jediterm" {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: false,
        };
    }
    if cfg!(windows) {
        return TerminalCapabilities {
            images: None,
            true_color: true,
            hyperlinks: false,
        };
    }
    TerminalCapabilities {
        images: None,
        true_color: has_true_color_hint,
        hyperlinks: false,
    }
}

pub fn get_capabilities() -> TerminalCapabilities {
    if let Some(caps) = state().cached {
        return caps;
    }
    let detected = detect_capabilities(None);
    let mut state = state();
    let mut caps = detected;
    if let Some(images) = state.overrides.images {
        caps.images = images;
    }
    if let Some(true_color) = state.overrides.true_color {
        caps.true_color = true_color;
    }
    if let Some(hyperlinks) = state.overrides.hyperlinks {
        caps.hyperlinks = hyperlinks;
    }
    state.cached = Some(caps);
    caps
}

pub fn reset_capabilities_cache() {
    state().cached = None;
}

pub fn set_capability_overrides(
    images: Option<Option<ImageProtocol>>,
    true_color: Option<bool>,
    hyperlinks: Option<bool>,
) {
    let next = CapabilityOverrides {
        images,
        true_color,
        hyperlinks,
    };
    let mut state = state();
    if state.overrides == next {
        return;
    }
    state.overrides = next;
    state.cached = None;
}

pub fn set_capabilities(caps: TerminalCapabilities) {
    state().cached = Some(caps);
}

pub fn render_image(
    base64_data: &str,
    image: ImageDimensions,
    options: ImageRenderOptions,
) -> Option<RenderedImage> {
    let caps = get_capabilities();
    let protocol = caps.images?;
    let max_width = options.max_width_cells.unwrap_or(80);
    let size = calculate_image_cell_size(
        image,
        max_width,
        options.max_height_cells,
        get_cell_dimensions(),
    );
    match protocol {
        ImageProtocol::Kitty => {
            if let Some(image_id) = options.image_id {
                register_kitty_image_metadata(KittyImageMetadata {
                    image_id,
                    columns: size.columns,
                    rows: size.rows,
                    width_px: image.width_px,
                    height_px: image.height_px,
                });
            }
            let sequence = encode_kitty(
                base64_data,
                Some(size.columns),
                Some(size.rows),
                options.image_id,
                options.move_cursor,
            );
            Some(RenderedImage {
                sequence,
                columns: size.columns,
                rows: size.rows,
                image_id: options.image_id,
            })
        }
        ImageProtocol::ITerm2 => {
            let sequence = encode_iterm2(
                base64_data,
                Some(&size.columns.to_string()),
                Some("auto"),
                None,
                options.preserve_aspect_ratio.or(Some(true)),
                None,
            );
            Some(RenderedImage {
                sequence,
                columns: size.columns,
                rows: size.rows,
                image_id: None,
            })
        }
    }
}

pub fn hyperlink(text: &str, url: &str) -> String {
    format!("\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENV_KEYS: &[&str] = &[
        "TERM",
        "TERM_PROGRAM",
        "TERMINAL_EMULATOR",
        "COLORTERM",
        "TMUX",
        "KITTY_WINDOW_ID",
        "GHOSTTY_RESOURCES_DIR",
        "WEZTERM_PANE",
        "ITERM_SESSION_ID",
        "WT_SESSION",
        "CMUX_WORKSPACE_ID",
        "WARP_SESSION_ID",
        "WARP_TERMINAL_SESSION_UUID",
        "PI_HYPERLINKS",
        "PI_IMAGE_PROTOCOL",
        "PI_TRUE_COLOR",
    ];

    fn with_env<T>(overrides: &[(&str, Option<&str>)], f: impl FnOnce() -> T) -> T {
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _env = ENV_LOCK.lock().expect("env");
        let saved: Vec<(String, Option<String>)> = ENV_KEYS
            .iter()
            .map(|key| ((*key).to_string(), std::env::var(key).ok()))
            .collect();
        for key in ENV_KEYS {
            std::env::remove_var(key);
        }
        for (key, value) in overrides {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        reset_capabilities_cache();
        let result = f();
        for (key, value) in saved {
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
        reset_capabilities_cache();
        result
    }

    #[test]
    fn is_image_line_matches_typescript_prefixes() {
        assert!(is_image_line(
            "\x1b]1337;File=size=100,100;inline=1:base64encodeddata==\x07"
        ));
        assert!(is_image_line(
            "Some text \x1b]1337;File=size=100,100;inline=1:base64data==\x07 more text"
        ));
        assert!(is_image_line("\x1b]1337;File=:\x07"));
        assert!(is_image_line(
            "\x1b_Ga=T,f=100,t=f,d=base64data...\x1b\\\x1b_Gm=i=1;\x1b\\"
        ));
        assert!(is_image_line(
            "  \x1b_Ga=T,f=100...\x1b\\\x1b_Gm=i=1;\x1b\\  "
        ));
        assert!(!is_image_line("plain text"));
        assert!(!is_image_line("\x1b[31mRed text\x1b[0m"));
        assert!(!is_image_line("Some text with ]1337;File but missing ESC"));
        assert!(!is_image_line("Some text with _G but missing ESC"));
        assert!(!is_image_line("/path/to/File_1337_backup/image.jpg"));
    }

    #[test]
    fn encode_and_delete_match_typescript() {
        assert_eq!(
            encode_iterm2("AAAA", Some("2"), Some("auto"), None, None, None),
            "\x1b]1337;File=inline=1;size=3;width=2;height=auto:AAAA\x07"
        );
        let sequence = encode_kitty("AAAA", Some(2), Some(2), None, Some(false));
        assert!(sequence.starts_with("\x1b_Ga=T,f=100,q=2,C=1,c=2,r=2;"));
        assert_eq!(delete_kitty_image(42), "\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
        assert_eq!(delete_all_kitty_images(), "\x1b_Ga=d,d=A,q=2\x1b\\");
        assert_eq!(delete_all_kitty_placements(), "\x1b_Ga=d,d=a,q=2\x1b\\");
    }

    #[test]
    fn detect_capabilities_matches_typescript_env_fixtures() {
        with_env(&[], || {
            let caps = detect_capabilities(None);
            assert!(!caps.hyperlinks);
            assert_eq!(caps.images, None);
        });
        with_env(
            &[
                ("PI_HYPERLINKS", Some("1")),
                ("PI_IMAGE_PROTOCOL", Some("kitty")),
                ("PI_TRUE_COLOR", Some("1")),
            ],
            || {
                assert_eq!(
                    detect_capabilities(None),
                    TerminalCapabilities {
                        images: Some(ImageProtocol::Kitty),
                        true_color: true,
                        hyperlinks: true,
                    }
                );
            },
        );
        with_env(
            &[
                ("TERM_PROGRAM", Some("iterm.app")),
                ("PI_HYPERLINKS", Some("0")),
                ("PI_IMAGE_PROTOCOL", Some("none")),
                ("PI_TRUE_COLOR", Some("0")),
            ],
            || {
                assert_eq!(
                    detect_capabilities(None),
                    TerminalCapabilities {
                        images: None,
                        true_color: false,
                        hyperlinks: false,
                    }
                );
            },
        );
        with_env(
            &[
                ("TERM_PROGRAM", Some("ghostty")),
                ("PI_HYPERLINKS", Some("auto")),
                ("PI_IMAGE_PROTOCOL", Some("auto")),
                ("PI_TRUE_COLOR", Some("auto")),
            ],
            || {
                assert_eq!(
                    detect_capabilities(None),
                    TerminalCapabilities {
                        images: Some(ImageProtocol::Kitty),
                        true_color: true,
                        hyperlinks: true,
                    }
                );
            },
        );
        with_env(
            &[
                ("TMUX", Some("/tmp/tmux-1000/default,1234,0")),
                ("TERM_PROGRAM", Some("ghostty")),
            ],
            || {
                let caps = detect_capabilities(Some(|| true));
                assert!(caps.hyperlinks);
                assert_eq!(caps.images, None);
                let caps = detect_capabilities(Some(|| false));
                assert!(!caps.hyperlinks);
            },
        );
        with_env(&[("TERM", Some("screen-256color"))], || {
            let caps = detect_capabilities(None);
            assert!(!caps.hyperlinks);
            assert_eq!(caps.images, None);
        });
        with_env(
            &[
                ("TERM_PROGRAM", Some("WarpTerminal")),
                ("TMUX", Some("/tmp/tmux-1000/default,1234,0")),
                ("TERM", Some("tmux-256color")),
            ],
            || {
                let caps = detect_capabilities(Some(|| true));
                assert_eq!(caps.images, None);
                assert!(caps.hyperlinks);
            },
        );
    }

    #[test]
    fn render_crop_and_placement_match_typescript() {
        set_capabilities(TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        });
        set_cell_dimensions(CellDimensions {
            width_px: 10,
            height_px: 10,
        });
        let rendered = render_image(
            "AAAA",
            ImageDimensions {
                width_px: 20,
                height_px: 20,
            },
            ImageRenderOptions {
                max_width_cells: Some(2),
                ..ImageRenderOptions::default()
            },
        )
        .expect("kitty");
        assert!(!rendered.sequence.contains(",C=1,"));
        assert_eq!(rendered.rows, 2);

        let no_move = render_image(
            "AAAA",
            ImageDimensions {
                width_px: 20,
                height_px: 20,
            },
            ImageRenderOptions {
                max_width_cells: Some(2),
                move_cursor: Some(false),
                ..ImageRenderOptions::default()
            },
        )
        .expect("kitty");
        assert!(no_move.sequence.contains(",C=1,"));

        let result = render_image(
            "AAAA",
            ImageDimensions {
                width_px: 100,
                height_px: 100,
            },
            ImageRenderOptions {
                max_width_cells: Some(3),
                image_id: Some(42),
                move_cursor: Some(false),
                ..ImageRenderOptions::default()
            },
        )
        .expect("kitty");
        assert_eq!(
            get_kitty_image_metadata(&result.sequence),
            Some(KittyImageMetadata {
                image_id: 42,
                columns: 3,
                rows: 3,
                width_px: 100,
                height_px: 100,
            })
        );
        assert!(crop_kitty_image_line(&result.sequence, 2, 1).contains("y=66,h=34,r=1"));

        register_kitty_image_metadata(KittyImageMetadata {
            image_id: 42,
            columns: 3,
            rows: 3,
            width_px: 100,
            height_px: 100,
        });
        let transmission = encode_kitty(
            "A".repeat(8192).as_str(),
            Some(3),
            Some(3),
            Some(42),
            Some(false),
        );
        let cropped = crop_kitty_image_line(&transmission, 2, 1);
        let line = format!("left {cropped} right");
        let placement = get_kitty_image_placement(&line).expect("placement");
        assert_eq!(
            placement.transmission_bytes,
            line.len() - "left ".len() - " right".len()
        );
        assert_eq!(placement.estimated_decoded_bytes, 100 * 100 * 4);
        assert_eq!(
            placement.sequence,
            "\x1b_Ga=p,q=2,C=1,c=3,i=42,y=66,h=34,r=1\x1b\\"
        );
        assert_eq!(
            placement.replacement_line,
            format!("left {} right", placement.sequence)
        );
        assert!(!placement.replacement_line.contains("AAAA"));

        set_cell_dimensions(CellDimensions {
            width_px: 9,
            height_px: 18,
        });
        reset_capabilities_cache();
    }
}
