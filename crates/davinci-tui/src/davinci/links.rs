//! Links in rendered rows: web addresses and file paths that exist.
//!
//! The runtime wraps each in an OSC 8 hyperlink after a frame is drawn, and
//! a Ctrl-click (Cmd where the terminal reports it) opens the one under the
//! pointer. Paths that start with two slashes or two backslashes name a
//! network share and are never linked.

use std::path::{Path, PathBuf};

use unicode_width::UnicodeWidthStr;

/// A link on one rendered row: the cells it covers and where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    pub start: u16,
    pub end: u16,
    pub url: String,
}

/// Every link in `row`. `exists` says whether a path names a file or folder;
/// `cwd` anchors relative paths.
pub fn find(row: &str, cwd: &Path, exists: &mut impl FnMut(&Path) -> bool) -> Vec<Link> {
    let mut out = Vec::new();
    let mut offset = 0;
    for token in row.split(' ') {
        let start = offset;
        offset += token.len() + 1;
        if token.is_empty() {
            continue;
        }
        let lead = token
            .find(|c: char| !matches!(c, '(' | '[' | '<' | '"' | '\'' | '`'))
            .unwrap_or(token.len());
        let body = trim_tail(&token[lead..]);
        if body.is_empty() {
            continue;
        }
        let url = if body.starts_with("http://") || body.starts_with("https://") {
            (body.len() > body.find("//").unwrap_or(0) + 2).then(|| body.to_owned())
        } else {
            path_url(body, cwd, exists)
        };
        if let Some(url) = url {
            let from = start + lead;
            out.push(Link {
                start: row[..from].width() as u16,
                end: row[..from + body.len()].width() as u16,
                url,
            });
        }
    }
    out
}

/// Drop sentence punctuation after a link, and a closing bracket nothing
/// opened.
fn trim_tail(token: &str) -> &str {
    let mut end = token.len();
    while let Some(last) = token[..end].chars().next_back() {
        let open = match last {
            ')' => Some('('),
            ']' => Some('['),
            '>' => Some('<'),
            _ => None,
        };
        let drop = match last {
            '.' | ',' | ';' | ':' | '!' | '?' | '"' | '\'' | '`' => true,
            _ => open.is_some_and(|open| {
                token[..end].matches(last).count() > token[..end].matches(open).count()
            }),
        };
        if !drop {
            break;
        }
        end -= last.len_utf8();
    }
    &token[..end]
}

/// `file://` address of an existing path, with an optional `:line[:col]`
/// suffix left out of the address.
fn path_url(token: &str, cwd: &Path, exists: &mut impl FnMut(&Path) -> bool) -> Option<String> {
    let rooted = token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/");
    if !rooted || token.starts_with("//") || token.contains('\\') {
        return None;
    }
    let mut path = token;
    // `src/lib.rs:12:3` names the file.
    for _ in 0..2 {
        match path.rsplit_once(':') {
            Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => {
                path = head;
            }
            _ => break,
        }
    }
    let full: PathBuf = if let Some(rest) = path.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME")?).join(rest)
    } else if path.starts_with('/') {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let full = normalize(&full);
    if !full.is_absolute() || !exists(&full) {
        return None;
    }
    Some(format!("file://{}", encode(&full.to_string_lossy())))
}

/// Resolve `.` and `..` without touching the disk.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn encode(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for byte in path.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// The link covering column `column` of `row`.
pub fn at(links: &[Link], column: u16) -> Option<&Link> {
    links
        .iter()
        .find(|link| (link.start..link.end).contains(&column))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn links(row: &str) -> Vec<Link> {
        let cwd = Path::new("/work/app");
        find(row, cwd, &mut |path| {
            path.starts_with("/work/app/src") || path == Path::new("/etc/hosts")
        })
    }

    #[test]
    fn web_addresses_drop_trailing_punctuation() {
        let found = links("see https://example.com/a?b=1, or (https://x.dev/y).");
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].url, "https://example.com/a?b=1");
        assert_eq!((found[0].start, found[0].end), (4, 29));
        assert_eq!(found[1].url, "https://x.dev/y");
    }

    #[test]
    fn an_existing_path_links_without_its_line_number() {
        let found = links("  error in ./src/lib.rs:12:3 and /etc/hosts");
        assert_eq!(found[0].url, "file:///work/app/src/lib.rs");
        assert_eq!(found[1].url, "file:///etc/hosts");
    }

    #[test]
    fn missing_files_and_network_shares_are_not_linked() {
        assert!(links("open ./nope.txt").is_empty());
        assert!(links("//server/share/file").is_empty());
        assert!(links(r"\\server\share\src").is_empty());
        assert!(links("http://").is_empty());
    }

    #[test]
    fn columns_count_cells_not_bytes() {
        let found = links("日本 https://a.io");
        assert_eq!(found[0].start, 5);
        assert_eq!(found[0].end, 5 + "https://a.io".len() as u16);
        assert!(at(&found, 6).is_some());
        assert!(at(&found, 4).is_none());
    }

    #[test]
    fn parent_directories_resolve_and_odd_bytes_are_escaped() {
        let cwd = Path::new("/work/app");
        let found = find("../app/src/a b.rs ../app/src/é.rs", cwd, &mut |_| true);
        assert_eq!(found[0].url, "file:///work/app/src/a");
        assert_eq!(found[1].url, "file:///work/app/src/%C3%A9.rs");
    }
}
