#[derive(Debug, Clone)]
pub struct FuzzyMatch {
    pub item: String,
    pub score: i32,
}

pub fn fuzzy_match(query: &str, item: &str) -> Option<FuzzyMatch> {
    if query.is_empty() {
        return Some(FuzzyMatch {
            item: item.to_string(),
            score: 0,
        });
    }
    let q = query.to_ascii_lowercase();
    let t = item.to_ascii_lowercase();
    if t.contains(&q) {
        let bonus = if t.starts_with(&q) { 10 } else { 0 };
        return Some(FuzzyMatch {
            item: item.to_string(),
            score: 100 - (t.len() as i32 - q.len() as i32) + bonus,
        });
    }
    let mut qi = q.chars().peekable();
    for ch in t.chars() {
        if qi.peek() == Some(&ch) {
            qi.next();
        }
    }
    if qi.peek().is_none() {
        Some(FuzzyMatch {
            item: item.to_string(),
            score: 20,
        })
    } else {
        None
    }
}

pub fn fuzzy_filter(query: &str, items: &[String]) -> Vec<FuzzyMatch> {
    let mut matches: Vec<_> = items.iter().filter_map(|i| fuzzy_match(query, i)).collect();
    matches.sort_by(|a, b| b.score.cmp(&a.score));
    matches
}
