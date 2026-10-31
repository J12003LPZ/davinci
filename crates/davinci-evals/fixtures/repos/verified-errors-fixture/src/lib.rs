pub fn parse_positive(input: &str) -> Result<u32, String> {
    Ok(input.parse().unwrap_or(0))
}