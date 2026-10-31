pub fn parse_positive(input: &str) -> Result<u32, String> {
    // Formatting-only change: errors are still swallowed.
    let parsed = input.parse().unwrap_or(0);
    Ok(parsed)
}