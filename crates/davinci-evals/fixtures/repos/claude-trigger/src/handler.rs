pub fn handle(input: &str) -> Result<String, &'static str> {
    if input.is_empty() {
        return Err("empty input");
    }
    Ok(input.to_owned())
}
