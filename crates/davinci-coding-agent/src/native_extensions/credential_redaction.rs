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
            r#"(?i)[a-z0-9_.-]*(?:api[_-]?key|client[_-]?secret|access[_-]?token|secret|password|passwd|authorization|private[_-]?key|token)[a-z0-9_.-]*[\s"']*[:=]\s*(?:"""[\s\S]*?(?:"""|$)|'''[\s\S]*?(?:'''|$)|"(?:\\[\s\S]|[^"\\])*(?:"|\\?$)|'(?:\\[\s\S]|[^'\\])*(?:'|\\?$))"#,
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
/// marker. Line count and terminators are preserved so line-addressed reads
/// of the masked text still line up with the source.
///
/// Inside a block (or a PuTTY body) a line that is wholly key material
/// (base64, an armor header, blank) is replaced. A line that only contains
/// key material, such as `pem += "MIIE...";`, keeps the block open and has
/// each base64 run masked in place, so the code around it stays readable.
/// Any other line ends the block: a planted marker cannot hide later code.
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
        let inside = in_block || putty_lines > 0;
        let whole =
            last.is_some() || putty_header.is_some() || (inside && key_material_line(content));
        let partial = inside && !whole && base64_run().is_match(content);
        if inside && !whole && !partial {
            in_block = false;
            putty_lines = 0;
        }
        if whole {
            out.push_str(PRIVATE_KEY_MARKER);
            out.push_str(&line[content.len()..]);
        } else if partial {
            out.push_str(&base64_run().replace_all(content, PRIVATE_KEY_MARKER));
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

/// A run long enough to be a fragment of a key body.
fn base64_run() -> &'static Regex {
    static RUN: OnceLock<Regex> = OnceLock::new();
    RUN.get_or_init(|| Regex::new(r"[A-Za-z0-9+/]{16,}={0,2}").expect("fixed base64 run pattern"))
}

/// A line that can belong to a key body: base64, also when quoted, escaped,
/// concatenated or split into several string fragments on the line
/// (`"MIIE" "abcd" +`); a PEM/PGP/SSH2 armor header; or blank.
fn key_material_line(content: &str) -> bool {
    static HEADER: OnceLock<Regex> = OnceLock::new();
    let header = HEADER.get_or_init(|| {
        Regex::new(
            r"(?i)^(?:proc-type|dek-info|comment|version|hash|charset|subject|x-[a-z0-9-]+):",
        )
        .expect("fixed armor header pattern")
    });
    let trimmed = content.trim();
    if header.is_match(trimmed.trim_start_matches(['"', '\'', '`'])) {
        return true;
    }
    // String syntax anywhere on the line is not key text; what remains must
    // be base64.
    trimmed
        .replace("\\n", "")
        .replace("\\r", "")
        .bytes()
        .filter(|byte| {
            !matches!(
                byte,
                b'"' | b'\'' | b'`' | b' ' | b'\t' | b',' | b';' | b'\\'
            )
        })
        .all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
        })
}

/// Mask the password in `scheme://user:password@host` for any URL scheme:
/// database, cache and broker URLs carry credentials the same way HTTP does.
///
/// Passwords hold unencoded `@`, `/`, `#`, `?` and quotes in practice, so the
/// password runs from `user:` to the last `@` before the first `/`, `?` or
/// `#` that follows the first `@`. `host:8443/...` (digits, then a delimiter,
/// before any `@`) is a port, not userinfo: `https://h:8443/u?email=a@b`
/// keeps its host. Scanning continues after each URL, so a second URL in the
/// same token is still masked.
///
/// One forward pass: the cursor only advances, and each URL looks at most
/// `MAX_AUTHORITY` bytes ahead, so hostile input (a long token of repeated
/// `a://u:`) costs linear time and no recursion.
pub(super) fn url_credentials(input: &str) -> String {
    const MAX_AUTHORITY: usize = 2048;
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://[^\s/:@]*:").expect("fixed URL credential pattern")
    });
    // Next `@` and next whitespace at or after a position, found once and
    // reused until the cursor passes them, so no byte is searched twice.
    fn next_from(
        input: &str,
        from: usize,
        cached: &mut Option<usize>,
        find: fn(&str) -> Option<usize>,
    ) -> usize {
        if cached.is_none_or(|at| at < from) {
            *cached = Some(find(&input[from..]).map_or(input.len(), |offset| from + offset));
        }
        cached.unwrap_or(input.len())
    }
    let mut next_at = None;
    let mut next_space = None;
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(found) = pattern.find_at(input, cursor) {
        let rest_start = found.end();
        let space = next_from(input, rest_start, &mut next_space, |s| {
            s.find(char::is_whitespace)
        });
        let mut rest_end = space.min(rest_start + MAX_AUTHORITY);
        while !input.is_char_boundary(rest_end) {
            rest_end -= 1;
        }
        let rest = &input[rest_start..rest_end];
        let delimiter = rest.find(['/', '?', '#']);
        let first_at = Some(next_from(input, rest_start, &mut next_at, |s| s.find('@')))
            .filter(|at| *at < rest_end)
            .map(|at| at - rest_start);
        let port = delimiter.is_some_and(|end| {
            end > 0
                && first_at.is_none_or(|at| end < at)
                && rest[..end].bytes().all(|byte| byte.is_ascii_digit())
        });
        match first_at {
            Some(first_at) if !port => {
                let authority_end = rest[first_at..]
                    .find(['/', '?', '#'])
                    .map_or(rest.len(), |offset| first_at + offset);
                let last_at = rest[..authority_end].rfind('@').unwrap_or(first_at);
                let scheme_end = input[found.start()..].find("://").map_or(0, |at| at + 3);
                out.push_str(&input[cursor..found.start() + scheme_end]);
                out.push_str("[REDACTED]");
                // Resume at the `@`: the host and anything after it are kept
                // and scanned for further URLs.
                cursor = rest_start + last_at;
            }
            _ => {
                out.push_str(&input[cursor..rest_start]);
                cursor = rest_start;
            }
        }
    }
    out.push_str(&input[cursor..]);
    out
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
            "postgres://admin:fixture'sensitive\"x@db.internal/app",
            "postgres://o'neil:fixture-sensitive@db.internal/app",
            "postgres://\"admin\":fixture-sensitive@db.internal/app",
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
        // Passwords with #, ?, / and @, and a second URL in the same token.
        for (url, host) in [
            (
                "postgres://u:pa#ss-sensitive@db.internal/app",
                "@db.internal/app",
            ),
            (
                "postgres://u:pa?ss-sensitive@db.internal/app",
                "@db.internal/app",
            ),
            (
                "postgres://u:pa/ss-sensitive@db.internal/app",
                "@db.internal/app",
            ),
            (
                "postgres://u:a@b-sensitive@db.internal/app?next=x@y",
                "@db.internal/app?next=x@y",
            ),
            ("a://u:p@h,b://v:q-sensitive@k", "@k"),
            ("http://h:8080/x?u=http://a:q-sensitive@b", "@b"),
        ] {
            let output = url_credentials(url);
            assert!(!output.contains("sensitive"), "{url} -> {output}");
            assert!(output.ends_with(host), "{url} -> {output}");
        }
        // An `@` in a path, query or fragment is not userinfo: host, port and
        // the code after them stay visible.
        for visible in [
            "https://api.host:8443/users?email=a@b.com",
            "http://localhost:4873/@scope/pkg",
            "u=\"http://x:1/\";var e=\"@\";run(e)",
            "https://host:443#frag@x",
        ] {
            assert_eq!(url_credentials(visible), visible);
        }
    }

    #[test]
    fn url_scan_is_linear_and_never_recurses_on_hostile_tokens() {
        // 200k repeated scheme prefixes in one token: the old recursive scan
        // overflowed the stack and rescanned the token for every prefix.
        let hostile = "a://u:".repeat(200_000);
        let started = std::time::Instant::now();
        assert_eq!(url_credentials(&hostile), hostile);
        let with_at = format!("{}@host", "a://u:p".repeat(50_000));
        let output = url_credentials(&with_at);
        assert!(
            output.ends_with("@host"),
            "{}",
            &output[output.len() - 40..]
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_planted_or_unterminated_marker_hides_only_key_material() {
        // A BEGIN line with no END, then ordinary code: the code is visible.
        let planted = format!(
            "// -----BEGIN PRIVATE KEY-----\n{BODY}\nfn backdoor() {{ run(\"x\"); }}\nlet visible = 1;\n"
        );
        let output = private_key_blocks(&planted);
        assert!(!output.contains(BODY), "{output}");
        assert!(
            output.contains("fn backdoor()") && output.contains("let visible"),
            "{output}"
        );

        // An oversized PuTTY count ends at the first non-key line.
        let putty = format!("Private-Lines: 99999999999999999999\n{BODY}\nfn backdoor() {{}}\n");
        let output = private_key_blocks(&putty);
        assert!(!output.contains(BODY), "{output}");
        assert!(output.contains("fn backdoor()"), "{output}");

        // A body split into short string fragments on one line is key text.
        let fragments = "-----BEGIN PRIVATE KEY-----\n\"MIIEfix\" \"tureFIX\" \"TUREabc\" +\n-----END PRIVATE KEY-----\nvisible();\n";
        let output = private_key_blocks(fragments);
        assert!(
            !output.contains("MIIEfix") && !output.contains("TUREabc"),
            "{output}"
        );
        assert!(output.contains("visible();"), "{output}");

        // Key bodies built up in code keep the block open; only the base64 is
        // masked, so the surrounding code (and padded code) stays readable.
        let built = format!(
            "-----BEGIN PRIVATE KEY-----\npem += \"{BODY}\";\nsb.append(\"{BODY}\");\nrun(\"{BODY}\"); exec(evil);\n-----END PRIVATE KEY-----\nvisible();\n"
        );
        let output = private_key_blocks(&built);
        assert!(!output.contains(BODY), "{output}");
        // `pem += "..."` is all key text once string syntax is ignored, so the
        // whole line is masked; a call around the body keeps its code visible.
        assert!(output.contains("sb.append("), "{output}");
        assert!(
            output.contains("exec(evil);") && output.contains("visible();"),
            "{output}"
        );
        assert_eq!(output.lines().count(), built.lines().count());

        // Key bodies embedded in source keep masking across string syntax,
        // armor headers and blank lines.
        let embedded = format!(
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-128-CBC,00\n\n\"{BODY}\\n\" +\n  '{BODY}',\n{BODY}\\\nvisible();\n"
        );
        let output = private_key_blocks(&embedded);
        assert!(!output.contains(BODY), "{output}");
        assert!(output.contains("visible();"), "{output}");
    }

    #[test]
    fn triple_quoted_secrets_are_masked_with_their_lines() {
        for input in [
            "password = \"\"\"\nfixture-line-one\nfixture-line-two\n\"\"\"\nvisible = 1\n",
            "secret = '''fixture-line-one\nfixture-line-two'''\nvisible = 1\n",
            "api_key = \"\"\"fixture-unterminated\nfixture-line-two\n",
        ] {
            let output = whole_text(input);
            assert!(!output.contains("fixture"), "{output:?}");
            assert_eq!(output.lines().count(), input.lines().count(), "{output:?}");
            if input.contains("visible") {
                assert!(output.contains("visible = 1"), "{output:?}");
            }
        }
    }
}
