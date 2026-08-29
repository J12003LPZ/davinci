//! Kitty / iTerm image protocol helpers matching `vendor/pi/packages/tui/src/terminal-image.ts`.

pub const KITTY_IMAGE_PREFIX: &str = "\x1b_G";
pub const KITTY_IMAGE_SUFFIX: &str = "\x1b\\";

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
