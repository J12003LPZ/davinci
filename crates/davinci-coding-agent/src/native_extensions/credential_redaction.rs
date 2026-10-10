//! Credential masks shared by Security Scan and memory redaction. These run on
//! whole text: a quoted value or a private key can span several lines, and a
//! per-line pass would leave the lines between its delimiters visible.

use regex::Regex;
use std::sync::OnceLock;

pub(super) const PRIVATE_KEY_MARKER: &str = "[REDACTED PRIVATE KEY MATERIAL]";

/// Every whole-text mask, in order. Each keeps the line count, so a
/// line-addressed read can mask the whole source and then take its window:
/// a window that starts inside a key or a multi-line quoted value would
/// otherwise miss the line that identifies it as a credential.
pub(super) fn whole_text(input: &str) -> String {
    url_credentials(&quoted_assignments(&private_key_blocks(input)))
}

/// Mask quoted assignments before token-based redactors can remove their
/// delimiters. A value spanning lines keeps its line breaks after the mask.
pub(super) fn quoted_assignments(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(
            r#"(?i)[a-z0-9_.-]*(?:api[_-]?key|client[_-]?secret|access[_-]?token|secret|password|passwd|authorization|private[_-]?key|token)[a-z0-9_.-]*[\s"']*[:=]\s*(?:"(?:\\[\s\S]|[^"\\])*(?:"|\\?$)|'(?:\\[\s\S]|[^'\\])*(?:'|\\?$))"#,
        )
        .expect("fixed quoted credential assignment pattern")
    });
    pattern
        .replace_all(input, |captures: &regex::Captures<'_>| {
            let mut mask = String::from("[REDACTED]");
            for (index, _) in captures[0].match_indices('\n') {
                let crlf = captures[0][..index].ends_with('\r');
                mask.push_str(if crlf { "\r\n" } else { "\n" });
            }
            mask
        })
        .into_owned()
}

/// Replace every line from a `BEGIN ... PRIVATE KEY` marker through its `END`
/// marker, or through the end of the text when the block is unterminated.
/// Line count and terminators are preserved so line-addressed reads of the
/// masked text still line up with the source.
pub(super) fn private_key_blocks(input: &str) -> String {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    let marker = MARKER.get_or_init(|| {
        // PEM and OpenSSH use five dashes, SSH2 four and spaces. The key type
        // between the words is free text (`X-ED25519`, tabs), but bounded so
        // prose that merely says "begin ... private key" is not armor.
        Regex::new(r"(?i)-{3,}\s*(BEGIN|END)\b[^\r\n]{0,40}?PRIVATE\s+KEY")
            .expect("fixed private key marker pattern")
    });
    static PUTTY: OnceLock<Regex> = OnceLock::new();
    let putty = PUTTY.get_or_init(|| {
        Regex::new(r"(?i)\bPrivate-Lines:\s*(\d+)").expect("fixed PuTTY key pattern")
    });
    let mut out = String::with_capacity(input.len());
    let mut in_block = false;
    // PuTTY keys have no armor: `Private-Lines: N` precedes N body lines.
    let mut putty_lines = 0usize;
    for line in input.split_inclusive('\n') {
        let content = line.trim_end_matches(['\n', '\r']);
        // Every line updates both trackers, masked or not, so one format's
        // body can never hide the other's opening marker.
        let last = marker
            .captures_iter(content)
            .last()
            .map(|captures| captures[1].eq_ignore_ascii_case("BEGIN"));
        let putty_header = putty
            .captures(content)
            .map(|count| count[1].parse().unwrap_or(usize::MAX));
        if in_block || last.is_some() || putty_lines > 0 || putty_header.is_some() {
            out.push_str(PRIVATE_KEY_MARKER);
            out.push_str(&line[content.len()..]);
        } else {
            out.push_str(line);
        }
        putty_lines = putty_lines.saturating_sub(1);
        if let Some(count) = putty_header {
            putty_lines = putty_lines.max(count);
        }
        // The last armor marker on a line decides, so a one-line key (escaped
        // newlines in a string) opens and closes here.
        if let Some(opens) = last {
            in_block = opens;
        }
    }
    out
}

/// Mask the password in `scheme://user:password@host` for any URL scheme:
/// database, cache and broker URLs carry credentials the same way HTTP does.
/// Passwords often hold an unencoded `@`, `/` or quote, so the mask runs to
/// the last `@` of the whitespace-delimited token rather than the first.
pub(super) fn url_credentials(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r#"(?i)\b([a-z][a-z0-9+.-]*://)[^\s/:@"'<>`]*:\S*@"#)
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
    fn ssh2_and_putty_private_keys_are_masked() {
        let ssh2 = format!(
            "---- BEGIN SSH2 ENCRYPTED PRIVATE KEY ----\nComment: \"fixture\"\n{BODY}\n---- END SSH2 ENCRYPTED PRIVATE KEY ----\nvisible\n"
        );
        let output = private_key_blocks(&ssh2);
        assert!(!output.contains(BODY), "{output}");
        assert!(output.ends_with("visible\n"), "{output}");

        let putty = format!(
            "PuTTY-User-Key-File-3: ssh-ed25519\nPublic-Lines: 1\nAAAApublic\nPrivate-Lines: 2\n{BODY}\n{BODY}\nPrivate-MAC: fixture\n"
        );
        let output = private_key_blocks(&putty);
        assert!(!output.contains(BODY), "{output}");
        assert!(
            output.contains("AAAApublic") && output.contains("Private-MAC"),
            "{output}"
        );
        assert_eq!(output.lines().count(), putty.lines().count());
    }

    #[test]
    fn one_key_format_never_hides_another_ones_marker() {
        // A PuTTY body count that covers a PEM BEGIN line must still open the
        // PEM block, and a PEM block must still see a PuTTY header.
        let mixed = format!(
            "Private-Lines: 1\n-----BEGIN PRIVATE KEY-----\n{BODY}\n{BODY}\n-----END PRIVATE KEY-----\nvisible\n"
        );
        let output = private_key_blocks(&mixed);
        assert!(!output.contains(BODY), "{output}");
        assert!(output.ends_with("visible\n"), "{output}");

        let mixed = format!(
            "-----BEGIN PRIVATE KEY-----\nPrivate-Lines: 3\n-----END PRIVATE KEY-----\n{BODY}\n{BODY}\nvisible\n"
        );
        let output = private_key_blocks(&mixed);
        assert!(!output.contains(BODY), "{output}");
        assert!(output.ends_with("visible\n"), "{output}");
    }

    #[test]
    fn free_text_key_types_are_armor() {
        for begin in [
            "-----BEGIN X-ED25519 PRIVATE KEY-----",
            "-----BEGIN\tRSA PRIVATE KEY-----",
            "-----BEGIN PGP PRIVATE KEY BLOCK-----",
        ] {
            let input = format!("{begin}\n{BODY}\n-----END PRIVATE KEY-----\nvisible\n");
            let output = private_key_blocks(&input);
            assert!(!output.contains(BODY), "{begin}: {output}");
            assert!(output.ends_with("visible\n"), "{output}");
        }
    }

    #[test]
    fn multi_line_quoted_values_keep_their_line_count() {
        for input in [
            "password = \"fixture first\nfixture second\nfixture third\"\nvisible\n",
            "password = \"fixture first\r\nfixture second\"\r\nvisible\r\n",
        ] {
            let output = whole_text(input);
            assert!(!output.contains("fixture"), "{output:?}");
            assert_eq!(output.lines().count(), input.lines().count(), "{output:?}");
            assert!(output.contains("visible"), "{output:?}");
        }
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
            // Unencoded `@` and `/` in the password.
            "postgres://admin:fixture@sensitive@db.internal/app",
            "postgres://admin:fixture/sensitive@db.internal/app",
            "postgres://admin:fixture'sensitive\"x@db.internal/app",
            "DATABASE_URL=\"mysql://root:fixture-sensitive@db/app\"",
        ] {
            let output = url_credentials(url);
            assert!(!output.contains("sensitive"), "{output}");
            assert!(output.contains("[REDACTED]@"), "{output}");
            assert!(!output.contains("[REDACTED]@sensitive"), "{output}");
        }
        assert_eq!(
            url_credentials("postgres://db.internal:5432/app"),
            "postgres://db.internal:5432/app"
        );
    }
}
