pub fn unique(values: &[i32]) -> Vec<i32> {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    sorted
}