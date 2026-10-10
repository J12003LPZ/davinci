//! Local credential redaction before provider context and portable projections.
use regex::Regex;
use std::sync::OnceLock;

pub fn text(input: &str) -> String {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| [
        r#"(?i)[a-z0-9_.-]*(?:api[_-]?key|client[_-]?secret|access[_-]?token|secret|password|passwd|authorization|private[_-]?key)[a-z0-9_.-]*[\s\"']*[:=][\s\"']*(?:(?:basic|bearer|digest|token)\s+)?[^\s\"',;}]+"#,
        r"(?i)\b(?:basic|bearer)\s+[a-z0-9+/=._~-]{8,}",
        r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b",
        r"\b(?:gh[pousr]_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]+",
        r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        r"https?://[^\s/:@]+:[^\s/@]+@",
    ].into_iter().map(|pattern| Regex::new(pattern).expect("fixed credential redaction pattern")).collect());
    super::super::credential_redaction::quoted_assignments(input)
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
    fn quoted_credentials_are_fully_redacted_before_other_masks() {
        for input in [
            r#"password: "fixture first second third"; safe=visible"#,
            r#"password="fixture first second third"; safe=visible"#,
            r#"{"client_secret": "fixture first \"second\" third", "safe": "visible"}"#,
            "password='fixture first \\'second third'; safe=visible",
            "password: \"fixture é 界 🦀\"; safe=visible",
            "password: \"fixture first second third",
            "password: \"fixture first second third\\",
            "password: \"fixture first\nsecond third\"; safe=visible",
        ] {
            let output = text(input);
            for secret_part in ["fixture", "first", "second", "third", "é", "界", "🦀"] {
                assert!(!output.contains(secret_part), "{output}");
            }
            if input.contains("visible") {
                assert!(output.contains("visible"), "{output}");
            }
        }
    }
    // Assembled at runtime so secret scanners do not flag the fixtures.
    const BASIC: &str = concat!("Ba", "sic");
    const BEARER: &str = concat!("Bea", "rer");
    const B64: &str = "Zml4dHVyZS1zZW5zaXRpdmU6eA==";
    #[test]
    fn security_redaction_covers_assignments_tokens_and_terminal_controls() {
        for secret in [
            "api_key = 'fixture-sensitive'",
            "{\"client_secret\": \"fixture-sensitive\"}",
            "https://user:fixture-sensitive@example.invalid",
            &format!("Authorization: {BASIC} {B64}"),
            &format!("{{\"Authorization\": \"{BASIC} {B64}\"}}"),
            &format!("curl -H 'Authorization: {BEARER} fixture-sensitive' x"),
            "AWS_SECRET_ACCESS_KEY=fixture-sensitive",
            "export aws_secret_access_key = \"fixture-sensitive\"",
            "db.password: fixture-sensitive",
        ] {
            assert!(!text(secret).contains("fixture-sensitive"));
        }
        assert!(!text("\u{1b}]52;clipboard\u{7}\u{202e}").contains(['\u{1b}', '\u{7}', '\u{202e}']));
        assert!(text("fn allowed() { validate_owner(); }").contains("validate_owner"));
    }
}
