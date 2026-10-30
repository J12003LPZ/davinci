pub fn parse_positive(input: &str) -> Result<u32, String> {
    let value: u32 = input.parse().map_err(|_| "invalid number".to_string())?;
    if value == 0 {
        Err("must be positive".to_string())
    } else {
        Ok(value)
    }
}