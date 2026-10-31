pub fn first_or_zero(values: &[i32]) -> i32 {
    values.first().copied().unwrap_or(0)
}