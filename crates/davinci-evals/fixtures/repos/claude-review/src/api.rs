pub fn parse_limit(value: &str) -> usize {
    value.parse().unwrap_or(usize::MAX)
}
