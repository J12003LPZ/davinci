pub fn timeout_ms(config: &str) -> Option<u64> {
    config
        .split(';')
        .find_map(|field| field.strip_prefix("timeout_seconds="))
        .and_then(|value| value.parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_seconds_are_converted_to_milliseconds() {
        assert_eq!(timeout_ms("timeout_seconds=3"), Some(3_000));
    }
}
