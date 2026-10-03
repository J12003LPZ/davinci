//! Kitty / iTerm image protocol helpers matching `vendor/pi/packages/tui/src/terminal-image.ts`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

pub const KITTY_IMAGE_PREFIX: &str = "\x1b_G";
pub const KITTY_IMAGE_SUFFIX: &str = "\x1b\\";
pub const ITERM2_IMAGE_PREFIX: &str = "\x1b]1337;File=";

/// TS `isImageLine`.
pub fn is_image_line(line: &str) -> bool {
    line.starts_with(KITTY_IMAGE_PREFIX)
        || line.starts_with(ITERM2_IMAGE_PREFIX)
        || line.contains(KITTY_IMAGE_PREFIX)
        || line.contains(ITERM2_IMAGE_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KittyImageMetadata {
    pub image_id: u32,
    pub columns: u32,
    pub rows: u32,
    pub width_px: u32,
    pub height_px: u32,
}

struct RegisteredKittyImage {
    metadata: KittyImageMetadata,
    transmission_generation: u64,
}

fn next_kitty_generation() -> u64 {
    static GENERATION: OnceLock<Mutex<u64>> = OnceLock::new();
    let cell = GENERATION.get_or_init(|| Mutex::new(0));
    let mut value = cell.lock().unwrap_or_else(|err| err.into_inner());
    *value += 1;
    *value
}

fn kitty_metadata() -> &'static Mutex<HashMap<u32, RegisteredKittyImage>> {
    static STORE: OnceLock<Mutex<HashMap<u32, RegisteredKittyImage>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// TS `registerKittyImageMetadata`.
pub fn register_kitty_image_metadata(metadata: KittyImageMetadata) {
    let mut store = kitty_metadata()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    store.remove(&metadata.image_id);
    store.insert(
        metadata.image_id,
        RegisteredKittyImage {
            metadata,
            transmission_generation: next_kitty_generation(),
        },
    );
    if store.len() > 1000 {
        if let Some(oldest) = store.keys().next().copied() {
            store.remove(&oldest);
        }
    }
}

fn kitty_controls(line: &str) -> Option<&str> {
    let start = line.find(KITTY_IMAGE_PREFIX)?;
    let after = &line[start + KITTY_IMAGE_PREFIX.len()..];
    after.split_once(';').map(|(controls, _)| controls)
}

fn image_id_from_controls(controls: &str) -> Option<u32> {
    for part in controls.split(',') {
        if let Some(value) = part.strip_prefix("i=") {
            return value.parse().ok();
        }
    }
    None
}

/// TS `getKittyImageMetadata`.
pub fn get_kitty_image_metadata(line: &str) -> Option<KittyImageMetadata> {
    let controls = kitty_controls(line)?;
    let image_id = image_id_from_controls(controls)?;
    let store = kitty_metadata()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    store.get(&image_id).map(|item| item.metadata)
}

/// TS `cropKittyImageLine`.
pub fn crop_kitty_image_line(line: &str, hidden_rows: u32, visible_rows: u32) -> String {
    let Some(metadata) = get_kitty_image_metadata(line) else {
        return line.to_string();
    };
    let Some(match_start) = line.find(KITTY_IMAGE_PREFIX) else {
        return line.to_string();
    };
    let Some(controls) = kitty_controls(line) else {
        return line.to_string();
    };
    if hidden_rows >= metadata.rows || visible_rows == 0 {
        return line.to_string();
    }
    let cropped_rows = visible_rows.min(metadata.rows - hidden_rows);
    if hidden_rows == 0 && cropped_rows == metadata.rows {
        return line.to_string();
    }
    let source_y = (metadata.height_px * hidden_rows) / metadata.rows;
    let source_end = {
        let num = metadata.height_px * (hidden_rows + cropped_rows);
        num.div_ceil(metadata.rows)
    };
    let source_height = 1.max(metadata.height_px.min(source_end).saturating_sub(source_y));
    let mut kept: Vec<String> = controls
        .split(',')
        .filter(|control| {
            !control.starts_with("y=") && !control.starts_with("h=") && !control.starts_with("r=")
        })
        .map(ToOwned::to_owned)
        .collect();
    kept.push(format!("y={source_y}"));
    kept.push(format!("h={source_height}"));
    kept.push(format!("r={cropped_rows}"));
    let rest_start = match_start + KITTY_IMAGE_PREFIX.len() + controls.len() + 1;
    format!(
        "{}{KITTY_IMAGE_PREFIX}{};{}",
        &line[..match_start],
        kept.join(","),
        &line[rest_start..]
    )
}

pub fn kitty_image_chunk(payload_b64: &str, last: bool) -> String {
    let more = if last { 0 } else { 1 };
    format!("{KITTY_IMAGE_PREFIX}a=T,f=100,m={more};{payload_b64}{KITTY_IMAGE_SUFFIX}")
}

/// TS `encodeKitty` from `terminal-image.ts`: `a=T,f=100,q=2` plus optional placement.
pub fn encode_kitty(
    base64_data: &str,
    columns: Option<u32>,
    rows: Option<u32>,
    image_id: Option<u32>,
    move_cursor: bool,
) -> String {
    const CHUNK_SIZE: usize = 4096;
    let mut params = vec!["a=T".into(), "f=100".into(), "q=2".into()];
    if !move_cursor {
        params.push("C=1".into());
    }
    if let Some(columns) = columns {
        params.push(format!("c={columns}"));
    }
    if let Some(rows) = rows {
        params.push(format!("r={rows}"));
    }
    if let Some(image_id) = image_id {
        params.push(format!("i={image_id}"));
    }
    if base64_data.len() <= CHUNK_SIZE {
        return format!(
            "{KITTY_IMAGE_PREFIX}{};{base64_data}{KITTY_IMAGE_SUFFIX}",
            params.join(",")
        );
    }
    let mut chunks = Vec::new();
    let mut offset = 0;
    let mut first = true;
    while offset < base64_data.len() {
        let end = (offset + CHUNK_SIZE).min(base64_data.len());
        let chunk = &base64_data[offset..end];
        let last = end == base64_data.len();
        if first {
            chunks.push(format!(
                "{KITTY_IMAGE_PREFIX}{},m=1;{chunk}{KITTY_IMAGE_SUFFIX}",
                params.join(",")
            ));
            first = false;
        } else if last {
            chunks.push(format!(
                "{KITTY_IMAGE_PREFIX}m=0;{chunk}{KITTY_IMAGE_SUFFIX}"
            ));
        } else {
            chunks.push(format!(
                "{KITTY_IMAGE_PREFIX}m=1;{chunk}{KITTY_IMAGE_SUFFIX}"
            ));
        }
        offset = end;
    }
    chunks.join("")
}

/// TS `deleteKittyImage`.
pub fn delete_kitty_image(image_id: u32) -> String {
    format!("{KITTY_IMAGE_PREFIX}a=d,d=I,i={image_id},q=2{KITTY_IMAGE_SUFFIX}")
}

/// TS `deleteAllKittyImages`.
pub fn delete_all_kitty_images() -> String {
    format!("{KITTY_IMAGE_PREFIX}a=d,d=A,q=2{KITTY_IMAGE_SUFFIX}")
}

/// TS `deleteAllKittyPlacements`.
pub fn delete_all_kitty_placements() -> String {
    format!("{KITTY_IMAGE_PREFIX}a=d,d=a,q=2{KITTY_IMAGE_SUFFIX}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellDimensions {
    pub width_px: u32,
    pub height_px: u32,
}

fn cell_dimensions() -> &'static Mutex<CellDimensions> {
    static STORE: OnceLock<Mutex<CellDimensions>> = OnceLock::new();
    STORE.get_or_init(|| {
        Mutex::new(CellDimensions {
            width_px: 9,
            height_px: 18,
        })
    })
}

/// TS `getCellDimensions`.
pub fn get_cell_dimensions() -> CellDimensions {
    *cell_dimensions()
        .lock()
        .unwrap_or_else(|err| err.into_inner())
}

/// TS `setCellDimensions`.
pub fn set_cell_dimensions(width_px: u32, height_px: u32) {
    *cell_dimensions()
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = CellDimensions {
        width_px,
        height_px,
    };
}

pub fn iterm_image(payload_b64: &str) -> String {
    format!("\x1b]1337;File=inline=1:{payload_b64}\u{7}")
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImageHeader {
    pub ids: Vec<String>,
    pub rows: u32,
}

/// Parse a Kitty graphics protocol header line matching
/// `parseKittyImageHeader` in `vendor/pi/packages/tui/src/tui-main-screen.ts`.
pub fn parse_kitty_image_header(line: &str) -> Option<KittyImageHeader> {
    let rest = line.strip_prefix(KITTY_IMAGE_PREFIX)?;
    let (params, _) = rest.split_once(';').unwrap_or((rest, ""));
    let mut ids = Vec::new();
    let mut rows = 1_u32;
    for part in params.split(',') {
        if let Some(value) = part.strip_prefix("i=") {
            ids.push(value.to_string());
        } else if let Some(value) = part.strip_prefix("I=") {
            ids.push(value.to_string());
        } else if let Some(value) = part.strip_prefix("r=") {
            rows = value.parse().unwrap_or(1);
        }
    }
    Some(KittyImageHeader { ids, rows })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KittyImagePlacement {
    pub image_id: u32,
    pub transmission_generation: u64,
    pub transmission_bytes: usize,
    pub estimated_decoded_bytes: usize,
    pub sequence: String,
    pub replacement_line: String,
}

const KITTY_PLACEMENT_CONTROL_KEYS: &[&str] = &[
    "i", "p", "x", "y", "w", "h", "X", "Y", "c", "r", "C", "U", "z", "P", "Q", "H", "V",
];

/// TS `getKittyImagePlacement`.
pub fn get_kitty_image_placement(line: &str) -> Option<KittyImagePlacement> {
    let start = line.find(KITTY_IMAGE_PREFIX)?;
    let after = &line[start + KITTY_IMAGE_PREFIX.len()..];
    let (controls, _) = after.split_once(';')?;
    let image_id = image_id_from_controls(controls)?;
    let store = kitty_metadata()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    let registered = store.get(&image_id)?;
    let mut command_start = start;
    let mut command_controls = controls.to_string();
    let mut transmission_end;
    loop {
        let rel = line[command_start + KITTY_IMAGE_PREFIX.len()..].find(KITTY_IMAGE_SUFFIX)?;
        let terminator = command_start + KITTY_IMAGE_PREFIX.len() + rel;
        transmission_end = terminator + KITTY_IMAGE_SUFFIX.len();
        let is_more = command_controls.split(',').any(|part| part == "m=1");
        if !is_more {
            break;
        }
        command_start = transmission_end;
        if !line[command_start..].starts_with(KITTY_IMAGE_PREFIX) {
            return None;
        }
        let after_prefix = &line[command_start + KITTY_IMAGE_PREFIX.len()..];
        let (next_controls, _) = after_prefix.split_once(';')?;
        command_controls = next_controls.to_string();
    }
    let kept: Vec<&str> = controls
        .split(',')
        .filter(|control| {
            let key = control.split('=').next().unwrap_or("");
            KITTY_PLACEMENT_CONTROL_KEYS.contains(&key)
        })
        .collect();
    let sequence = format!(
        "{KITTY_IMAGE_PREFIX}a=p,q=2,{}{KITTY_IMAGE_SUFFIX}",
        kept.join(",")
    );
    Some(KittyImagePlacement {
        image_id,
        transmission_generation: registered.transmission_generation,
        transmission_bytes: transmission_end - start,
        estimated_decoded_bytes: (registered.metadata.width_px as usize)
            .saturating_mul(registered.metadata.height_px as usize)
            .saturating_mul(4),
        sequence: sequence.clone(),
        replacement_line: format!("{}{sequence}{}", &line[..start], &line[transmission_end..]),
    })
}

pub fn kitty_image_ids(line: &str) -> Vec<String> {
    parse_kitty_image_header(line)
        .map(|header| header.ids)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_and_iterm_wrappers() {
        let chunk = kitty_image_chunk("QQ==", true);
        assert!(chunk.starts_with(KITTY_IMAGE_PREFIX));
        assert!(chunk.ends_with(KITTY_IMAGE_SUFFIX));
        assert!(iterm_image("QQ==").contains("1337"));
        let header = parse_kitty_image_header("\x1b_Gi=7,r=2;QQ==\x1b\\").unwrap();
        assert_eq!(header.ids, ["7"]);
        assert_eq!(header.rows, 2);
        let encoded = encode_kitty("QQ==", Some(2), Some(3), Some(42), false);
        assert!(encoded.contains("a=T,f=100,q=2,C=1,c=2,r=3,i=42"));
        assert_eq!(delete_kitty_image(42), "\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
        let big = encode_kitty(&"A".repeat(5000), None, None, None, true);
        assert!(big.contains(",m=1;"));
        assert!(big.contains("m=0;"));
    }
}
