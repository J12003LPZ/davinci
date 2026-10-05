//! Lowercase hex for digest bytes.
//!
//! `sha2` 0.11 digests no longer implement `LowerHex`. Wrapping the bytes keeps
//! existing `{:x}` format strings, including prefixes like `"sha256:{:x}"`.

use std::fmt;

/// Formats borrowed bytes as lowercase hex through `{:x}`.
pub struct Lower<'a>(pub &'a [u8]);

impl fmt::LowerHex for Lower<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Lower;

    #[test]
    fn formats_bytes_as_two_lowercase_digits_each() {
        assert_eq!(format!("{:x}", Lower(&[0x00, 0x0f, 0xab, 0xff])), "000fabff");
        assert_eq!(format!("{:x}", Lower(&[])), "");
    }
}
