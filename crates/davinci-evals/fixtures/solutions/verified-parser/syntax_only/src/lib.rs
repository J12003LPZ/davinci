pub fn parse_port(input: &str) -> u16 {
    // Formatting-only change: whitespace is still not trimmed.
    let parsed = input.parse().unwrap_or(0);
    parsed
}