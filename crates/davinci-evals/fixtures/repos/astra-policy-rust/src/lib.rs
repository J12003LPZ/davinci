pub fn normalize_slug(input: &str) -> String {
    input.trim().replace(' ', "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_lowercase_and_hyphenated() {
        assert_eq!(normalize_slug(" Hello World "), "hello-world");
    }
}
