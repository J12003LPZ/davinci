pub fn parse_positive(input: &str) -> Result<u32, String> {
    match input.parse::<u32>() {
        Ok(value) => Ok(value),
        Err(_) => Ok(0),
    }
}