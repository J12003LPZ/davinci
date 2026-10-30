pub fn first_or_zero(values: &[i32]) -> i32 {
    if values.is_empty() {
        -1
    } else {
        values[0]
    }
}