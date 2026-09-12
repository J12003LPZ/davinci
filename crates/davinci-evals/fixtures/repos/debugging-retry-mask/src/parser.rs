pub fn parse_port(value: &str) -> Result<u16, String> {
    value.parse().map_err(|_| "invalid port".into())
}

pub fn load_port(value: &str) -> Result<u16, String> {
    for _attempt in 0..3 {
        if let Ok(port) = parse_port(value) {
            return Ok(port);
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_invalid_port_remains_an_error() {
        assert!(load_port("not-a-port").is_err());
    }
}
