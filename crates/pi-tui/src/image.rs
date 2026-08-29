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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_and_iterm_wrappers() {
        let chunk = kitty_image_chunk("QQ==", true);
        assert!(chunk.starts_with(KITTY_IMAGE_PREFIX));
        assert!(chunk.ends_with(KITTY_IMAGE_SUFFIX));
        assert!(iterm_image("QQ==").contains("1337"));
    }
}
