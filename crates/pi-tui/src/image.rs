//! Kitty / iTerm image protocol helpers matching `vendor/pi/packages/tui/src/terminal-image.ts`.

pub const KITTY_IMAGE_PREFIX: &str = "\x1b_G";
pub const KITTY_IMAGE_SUFFIX: &str = "\x1b\\";

pub fn kitty_image_chunk(payload_b64: &str, last: bool) -> String {
    let more = if last { 0 } else { 1 };
    format!("{KITTY_IMAGE_PREFIX}a=T,f=100,m={more};{payload_b64}{KITTY_IMAGE_SUFFIX}")
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
    }
}
