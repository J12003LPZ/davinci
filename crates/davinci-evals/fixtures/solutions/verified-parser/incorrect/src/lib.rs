pub fn parse_port(input: &str) -> u16 {
    input.trim_start().parse().unwrap_or(0)
}