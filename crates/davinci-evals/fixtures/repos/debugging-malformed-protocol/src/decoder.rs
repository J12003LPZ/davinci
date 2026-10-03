pub fn decode_count(value: &str) -> Result<u64, String> {
    Ok(value
        .split('=')
        .nth(1)
        .unwrap_or_default()
        .parse()
        .unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_count_is_rejected() {
        assert!(decode_count("count=not-a-number").is_err());
    }
}
