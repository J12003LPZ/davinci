pub fn parse_port(input: &str) -> u16 {
    input.trim().parse().unwrap_or(0)
}