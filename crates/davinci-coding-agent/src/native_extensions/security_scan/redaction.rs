//! Local credential redaction before provider context and portable projections.
use regex::Regex;
use std::sync::OnceLock;

pub fn text(input: &str) -> String {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"(?i)\b(?:basic|bearer)\s+[a-z0-9+/=._~-]{8,}",
            r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b",
            r"\b(?:gh[pousr]_|github_pat_|xox[baprs]-)[A-Za-z0-9_-]+",
            r"\bnpm_[A-Za-z0-9]{20,}",
            r"\bglpat-[A-Za-z0-9_-]{20,}",
            r"\beyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("fixed credential redaction pattern"))
        .collect()
    });
    super::super::credential_redaction::whole_text(input)
        .lines()
        .map(|line| {
            let mut line = key_assignments(&super::redact_evidence(line));
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

/// `key = value`, `key: value`, `key := value` for credential-named keys.
/// The value may not start with `=` or `>`: `token == expected` and
/// `token => ...` are comparisons and arrows a security review needs to see,
/// not assignments. A purely numeric value of a token key is a count
/// (`max_tokens=100`), not a credential.
fn key_assignments(line: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)([a-z0-9_.-]*(?:api[_-]?key|client[_-]?secret|access[_-]?token|secret|password|passwd|authorization|private[_-]?key|token)[a-z0-9_.-]*)[\s"']*(?::=|[:=])[\s"']*(?:(?:basic|bearer|digest|token)\s+)?([^\s"',;}=>][^\s"',;}]*)"#,
        )
        .expect("fixed credential assignment pattern")
    });
    pattern
        .replace_all(line, |captures: &regex::Captures<'_>| {
            let count = captures[1].to_ascii_lowercase().contains("token")
                && captures[2].bytes().all(|byte| byte.is_ascii_digit());
            if count {
                captures[0].to_string()
            } else {
                "[REDACTED]".to_string()
            }
        })
        .into_owned()
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

    #[test]
    fn private_key_bodies_url_passwords_and_named_tokens_are_redacted() {
        let body = "MIIEfixtureFIXTUREfixtureFIXTUREfixtureFIXTUREabcd";
        let pem = format!(
            "const KEY: &str = \"\\\n-----BEGIN RSA PRIVATE KEY-----\n{body}\n{body}\n-----END RSA PRIVATE KEY-----\";\nfn after() {{}}"
        );
        let output = text(&pem);
        assert!(!output.contains(body), "{output}");
        assert_eq!(output.lines().count(), pem.lines().count());
        assert!(output.contains("fn after()"), "{output}");

        let npm = concat!("npm", "_fixtureTOKENfixtureTOKENfixture");
        let gitlab = concat!("gl", "pat-fixtureTOKENfixtureTOKEN");
        for secret in [
            "DATABASE_URL=postgres://admin:fixture-sensitive@db.internal/app".to_string(),
            "redis://:fixture-sensitive@cache:6379".to_string(),
            "export NPM_TOKEN=fixture-sensitive".to_string(),
            "GITLAB_TOKEN: fixture-sensitive".to_string(),
            "auth_token = fixture-sensitive".to_string(),
            format!("//registry.npmjs.org/:_authToken={npm}"),
            format!("see {npm}"),
            format!("see {gitlab}"),
        ] {
            let output = text(&secret);
            assert!(
                !output.contains("fixture-sensitive") && !output.contains("fixtureTOKENfixture"),
                "{secret} -> {output}"
            );
        }
        assert!(text("postgres://db.internal/app").contains("db.internal"));
        for secret in [
            "TOKEN=fixture-sensitive",
            "authtoken: fixture-sensitive",
            "NPMTOKEN=fixture-sensitive",
            "bot_token = \"fixture.sensitive.value\"",
            "DISCORD_TOKEN=fixture.sensitive.value",
            // Credential-named keys are masked whatever the value contains.
            "API_TOKEN=fixture(sensitive",
            "SLACK_TOKEN=fixture<sensitive>",
            "auth_token: [fixture-sensitive]",
            "TOKEN=fixture(sensitive",
            "token=fixture(sensitive",
        ] {
            let output = text(secret);
            assert!(!output.contains("sensitive"), "{secret} -> {output}");
        }
        // No value-shape allowlist: any token-named assignment is masked,
        // including tokenizer code. Mentions without an assignment stay.
        assert_eq!(text("let token = lexer.next();"), "let [REDACTED];");
        // Comparisons, arrows and counts are code a review must see.
        for code in [
            "if token == expected_token {",
            "if password == input {",
            "assert!(token != stored);",
            "Some(token) => token,",
            "max_tokens=100",
            "let max_tokens: 4096,",
        ] {
            assert_eq!(text(code), code);
        }
        for secret in [
            "password := fixture-sensitive",
            "token := fixture-sensitive",
            "api_token=12345abc",
        ] {
            let output = text(secret);
            assert!(
                !output.contains("fixture-sensitive") && !output.contains("12345abc"),
                "{secret} -> {output}"
            );
        }
        for prose in [
            "the token expires after an hour",
            "Repeated grep call blocked by token governor; change the query",
        ] {
            assert_eq!(text(prose), prose);
        }
    }
}
