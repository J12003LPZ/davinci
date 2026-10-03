//! Local credential redaction before provider context and portable projections.
use regex::Regex;
use std::sync::OnceLock;

pub fn text(input: &str) -> String {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| [
        r#"(?i)(?:api[_-]?key|client[_-]?secret|access[_-]?token|password|secret|authorization)[\s\"']*[:=][\s\"']*[^\s\"',;}]+"#,
        r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b",
        r"\b(?:gh[pousr]_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]+",
        r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        r"https?://[^\s/:@]+:[^\s/@]+@",
    ].into_iter().map(|pattern| Regex::new(pattern).expect("fixed credential redaction pattern")).collect());
    input
        .lines()
        .map(|line| {
            let mut line = super::redact_evidence(line);
            for pattern in patterns {
                line = pattern.replace_all(&line, "[REDACTED]").into_owned();
            }
            line.chars()
                .filter(|c| !c.is_control() || *c == '\t')
                .filter(|c| !matches!(*c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'))
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn security_redaction_covers_assignments_tokens_and_terminal_controls() {
        for secret in [
            "api_key = 'fixture-sensitive'",
            "{\"client_secret\": \"fixture-sensitive\"}",
            "https://user:fixture-sensitive@example.invalid",
        ] {
            assert!(!text(secret).contains("fixture-sensitive"));
        }
        assert!(!text("\u{1b}]52;clipboard\u{7}\u{202e}").contains(['\u{1b}', '\u{7}', '\u{202e}']));
        assert!(text("fn allowed() { validate_owner(); }").contains("validate_owner"));
    }
}
