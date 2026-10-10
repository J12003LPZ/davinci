//! Credential masks shared by Security Scan and memory redaction. These run on
//! whole text: a quoted value or a private key can span several lines, and a
//! per-line pass would leave the lines between its delimiters visible.

use regex::Regex;
use std::sync::OnceLock;

pub(super) const PRIVATE_KEY_MARKER: &str = "[REDACTED PRIVATE KEY MATERIAL]";

/// Mask quoted assignments before token-based redactors can remove their delimiters.
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

/// Replace every line from a `BEGIN ... PRIVATE KEY` marker through its `END`
/// marker, or through the end of the text when the block is unterminated.
/// Line count and terminators are preserved so line-addressed reads of the
/// masked text still line up with the source.
pub(super) fn private_key_blocks(input: &str) -> String {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    let marker = MARKER.get_or_init(|| {
        Regex::new(r"(?i)-----\s*(BEGIN|END)\b[A-Z0-9 ]*PRIVATE KEY")
            .expect("fixed private key marker pattern")
    });
    let mut out = String::with_capacity(input.len());
    let mut in_block = false;
    for line in input.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        // Only a real armor marker opens or closes a block; the last one on
        // the line decides, so a one-line key (escaped newlines in a string)
        // opens and closes here and other text on the line changes nothing.
        let last = marker
            .captures_iter(content)
            .last()
            .map(|captures| captures[1].eq_ignore_ascii_case("BEGIN"));
        if in_block || last.is_some() {
            out.push_str(PRIVATE_KEY_MARKER);
            out.push_str(&line[content.len()..]);
        } else {
            out.push_str(line);
        }
        if let Some(opens) = last {
            in_block = opens;
        }
    }
    out
}

/// Mask the password in `scheme://user:password@host` for any URL scheme:
/// database, cache and broker URLs carry credentials the same way HTTP does.
pub(super) fn url_credentials(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/:@]*:[^\s/@]+@")
            .expect("fixed URL credential pattern")
    });
    pattern.replace_all(input, "$1[REDACTED]@").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BODY: &str = "MIIEfixtureFIXTUREfixtureFIXTUREfixtureFIXTUREabcd";

    #[test]
    fn private_key_blocks_mask_every_line_and_keep_line_numbers() {
        for kind in [
            "RSA PRIVATE KEY",
            "EC PRIVATE KEY",
            "OPENSSH PRIVATE KEY",
            "PRIVATE KEY",
            "ENCRYPTED PRIVATE KEY",
        ] {
            let input = format!(
                "before\r\nconst KEY: &str = \"\\\n-----BEGIN {kind}-----\n{BODY}\n{BODY}\n-----END {kind}-----\";\nafter\n"
            );
            let output = private_key_blocks(&input);
            assert!(!output.contains(BODY), "{kind}: {output}");
            assert_eq!(output.lines().count(), input.lines().count(), "{kind}");
            assert!(output.starts_with("before\r\n"), "{output}");
            assert!(output.ends_with("after\n"), "{output}");
        }
    }

    #[test]
    fn private_key_blocks_fail_closed_and_handle_one_line_keys() {
        let unterminated = format!("-----BEGIN PRIVATE KEY-----\n{BODY}\n{BODY}");
        assert!(!private_key_blocks(&unterminated).contains(BODY));

        let one_line = format!(
            "let key = \"-----BEGIN PRIVATE KEY-----\\n{BODY}\\n-----END PRIVATE KEY-----\";\nvisible\n"
        );
        let output = private_key_blocks(&one_line);
        assert!(!output.contains(BODY));
        assert!(output.ends_with("visible\n"), "{output}");

        let prose = "// rotate the private key yearly\nvisible\n";
        assert_eq!(private_key_blocks(prose), prose);
        let prose = "// BEGIN by loading the private key, END by dropping it\nvisible\n";
        assert_eq!(private_key_blocks(prose), prose);
    }

    #[test]
    fn text_around_a_begin_marker_never_closes_the_block() {
        // "END" in other text on the BEGIN line (APPENDED, ENDPOINT, pem_end)
        // must not end the block before the body.
        for begin in [
            "-----BEGIN RSA PRIVATE KEY----- // appended by vendor",
            "let pem_end = \"-----BEGIN PRIVATE KEY-----\\",
            "-----BEGIN EC PRIVATE KEY----- endpoint key",
            "-----END PRIVATE KEY----- -----BEGIN PRIVATE KEY-----",
            "-----begin openssh private key-----",
        ] {
            let input = format!("{begin}\n{BODY}\n{BODY}\n-----END PRIVATE KEY-----\nvisible\n");
            let output = private_key_blocks(&input);
            assert!(!output.contains(BODY), "{begin}: {output}");
            assert!(output.ends_with("visible\n"), "{begin}: {output}");
        }
    }

    #[test]
    fn url_credentials_cover_every_scheme_and_keep_the_host() {
        for url in [
            "postgres://admin:fixture-sensitive@db.internal/app",
            "mysql://root:fixture-sensitive@127.0.0.1:3306/db",
            "mongodb+srv://u:fixture-sensitive@cluster.example/db",
            "redis://:fixture-sensitive@cache:6379/0",
            "amqp://guest:fixture-sensitive@broker//",
            "https://user:fixture-sensitive@example.invalid",
        ] {
            let output = url_credentials(url);
            assert!(!output.contains("fixture-sensitive"), "{output}");
            assert!(output.contains("[REDACTED]@"), "{output}");
        }
        assert_eq!(
            url_credentials("postgres://db.internal:5432/app"),
            "postgres://db.internal:5432/app"
        );
    }
}
