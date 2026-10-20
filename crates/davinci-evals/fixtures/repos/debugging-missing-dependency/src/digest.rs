pub fn digest(input: &[u8]) -> u32 {
    checksum::sum(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_matches_the_checksum_dependency() {
        assert_eq!(digest(b"abc"), 294);
    }
}
