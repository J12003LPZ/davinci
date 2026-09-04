#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FuzzyMatch {
    pub matches: bool,
    pub score: f64,
}

pub fn fuzzy_match(query: &str, text: &str) -> FuzzyMatch {
    let query_lower = query.to_lowercase();
    let text_lower = text.to_lowercase();
    if query_lower.is_empty() {
        return FuzzyMatch {
            matches: true,
            score: 0.0,
        };
    }
    if query_lower.len() > text_lower.len() {
        return FuzzyMatch {
            matches: false,
            score: 0.0,
        };
    }
    let query_chars: Vec<char> = query_lower.chars().collect();
    let text_chars: Vec<char> = text_lower.chars().collect();
    let mut query_index = 0;
    let mut score = 0.0;
    let mut last_match_index: isize = -1;
    let mut consecutive = 0.0;
    for (i, ch) in text_chars.iter().enumerate() {
        if query_index >= query_chars.len() {
            break;
        }
        if *ch == query_chars[query_index] {
            let is_boundary =
                i == 0 || matches!(text_chars.get(i - 1), Some(c) if " -_./:".contains(*c));
            if last_match_index == i as isize - 1 {
                consecutive += 1.0;
                score -= consecutive * 5.0;
            } else {
                consecutive = 0.0;
                if last_match_index >= 0 {
                    score += (i as isize - last_match_index - 1) as f64 * 2.0;
                }
            }
            if is_boundary {
                score -= 10.0;
            }
            score += i as f64 * 0.1;
            last_match_index = i as isize;
            query_index += 1;
        }
    }
    if query_index < query_chars.len() {
        FuzzyMatch {
            matches: false,
            score: 0.0,
        }
    } else {
        FuzzyMatch {
            matches: true,
            score,
        }
    }
}

pub fn fuzzy_filter(query: &str, items: &[String]) -> Vec<String> {
    let mut scored: Vec<(f64, String)> = items
        .iter()
        .filter_map(|item| {
            let m = fuzzy_match(query, item);
            m.matches.then_some((m.score, item.clone()))
        })
        .collect();
    scored.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    scored.into_iter().map(|(_, item)| item).collect()
}
