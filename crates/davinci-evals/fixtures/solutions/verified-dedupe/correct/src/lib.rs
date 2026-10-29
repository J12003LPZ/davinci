pub fn unique(values: &[i32]) -> Vec<i32> {
    let mut seen = std::collections::BTreeSet::new();
    values
        .iter()
        .copied()
        .filter(|value| seen.insert(*value))
        .collect()
}