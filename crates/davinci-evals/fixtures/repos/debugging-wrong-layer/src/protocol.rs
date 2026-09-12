pub fn decode_version(value: &str) -> Result<u32, String> {
    value.parse().map_err(|_| "invalid version".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_decoder_accepts_the_wire_version_prefix() {
        assert_eq!(decode_version("v2").unwrap(), 2);
    }
}
