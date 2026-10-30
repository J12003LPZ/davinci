pub fn sum_range(start: i32, end: i32) -> i32 {
    // Formatting-only change: the exclusive range is still wrong.
    let total = (start..end).sum();
    total
}