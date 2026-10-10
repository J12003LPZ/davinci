//! Mask quoted assignments before token-based redactors can remove their delimiters.

use regex::Regex;
use std::sync::OnceLock;

pub(super) fn quoted_assignments(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)[a-z0-9_.-]*(?:api[_-]?key|client[_-]?secret|access[_-]?token|secret|password|passwd|authorization|private[_-]?key|token)[a-z0-9_.-]*[\s"']*[:=]\s*(?:"(?:\\[\s\S]|[^"\\])*(?:"|\\?$)|'(?:\\[\s\S]|[^'\\])*(?:'|\\?$))"#,
        )
        .expect("fixed quoted credential assignment pattern")
    });
    pattern.replace_all(input, "[REDACTED]").into_owned()
}
