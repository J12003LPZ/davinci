pub fn sum(input: &[u8]) -> u32 {
    input.iter().map(|byte| u32::from(*byte)).sum()
}
