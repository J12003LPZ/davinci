//! Shared search/navigation for native selectors. Source collections never move.
pub fn matches(query: &str, values: &[&str]) -> bool {
    let text = values.join(" ").to_lowercase();
    query
        .split_whitespace()
        .all(|word| text.contains(&word.to_lowercase()))
}

pub fn step(indices: &[usize], selected: usize, delta: isize) -> usize {
    if indices.is_empty() {
        return selected;
    }
    let at = indices
        .iter()
        .position(|index| *index == selected)
        .unwrap_or(0);
    indices[crate::davinci::model::wrap_index(at, delta, indices.len())]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn search_handles_unicode_words_and_keeps_original_identity() {
        assert!(matches("MODEL b", &["Model", "provider-b"]));
        assert!(matches("界", &["Project 界"]));
        assert!(!matches("other", &["Model"]));
        assert_eq!(step(&[2, 7, 11], 7, 1), 11);
        assert_eq!(step(&[2, 7, 11], 2, -1), 11);
        assert_eq!(step(&[], 7, 1), 7);
    }
}
