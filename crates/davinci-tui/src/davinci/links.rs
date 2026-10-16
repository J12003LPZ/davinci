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
    let drive = cfg!(windows) && has_drive(token);
    let rooted = drive
        || token.starts_with('/')
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with("~/");
    // A drive path may use backslashes; anything else with one is a share or
    // not a path at all.
    if !rooted || token.starts_with("//") || (token.contains('\\') && !drive) {
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
        let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
        PathBuf::from(home?).join(rest)
    } else if path.starts_with('/') || drive {
        PathBuf::from(path)
    } else {
        cwd.join(path)
    };
    let full = normalize(&full);
    // `/work/app` has a root but no drive on Windows; both count as anchored.
    if !(full.is_absolute() || full.has_root()) || !exists(&full) {
        return None;
    }
    file_url(&full)
}

/// `X:\` or `X:/` at the start of a token.
fn has_drive(token: &str) -> bool {
    let bytes = token.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/')
}

/// Build the address from path components so separators and drive letters
/// come out as `file:///C:/dir/file` on every platform.
fn file_url(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    let mut out = String::from("file://");
    let mut any = false;
    for part in path.components() {
        match part {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(letter) => {
                    out.push('/');
                    out.push(letter as char);
                    out.push(':');
                    any = true;
                }
                _ => return None,
            },
            Component::Normal(name) => {
                out.push('/');
                out.push_str(&encode(&name.to_string_lossy()));
                any = true;
            }
            _ => {}
        }
    }
    if !any {
        out.push('/');
    }
    Some(out)
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
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// What a click on `url` may hand to the operating system's opener. Web
/// addresses go through unchanged. A `file://` address never does: the opener
/// would run an executable, script or shortcut that model output merely
/// names, so only a folder is opened (the file's own folder for a file).
pub fn opener_target(url: &str, is_dir: impl Fn(&Path) -> bool) -> Option<String> {
    let Some(rest) = url.strip_prefix("file://") else {
        return Some(url.to_owned());
    };
    let path = decode(rest)?;
    let path = if cfg!(windows) {
        // `/C:/dir` -> `C:/dir`
        let bytes = path.as_bytes();
        if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b':' {
            path[1..].to_owned()
        } else {
            path
        }
    } else {
        path
    };
    let path = PathBuf::from(path);
    let folder = if is_dir(&path) {
        path
    } else {
        path.parent()?.to_path_buf()
    };
    // A bundle is a folder the opener launches (`open Foo.app`), so neither
    // it nor anything inside it is handed over.
    let launches = folder.components().any(|part| {
        part.as_os_str()
            .to_str()
            .and_then(|name| name.rsplit_once('.'))
            .is_some_and(|(_, ext)| {
                BUNDLE_EXTENSIONS
                    .iter()
                    .any(|bundle| ext.eq_ignore_ascii_case(bundle))
            })
    });
    if launches {
        return None;
    }
    Some(folder.to_string_lossy().into_owned())
}

const BUNDLE_EXTENSIONS: [&str; 12] = [
    "app",
    "bundle",
    "framework",
    "plugin",
    "prefpane",
    "workflow",
    "action",
    "scpt",
    "appex",
    "kext",
    "pkg",
    "mpkg",
];

fn decode(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = encoded.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
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

    #[test]
    fn a_file_link_opens_its_folder_never_the_file() {
        let is_dir = |p: &Path| p == Path::new("/work/app/src");
        assert_eq!(
            opener_target("https://x.dev/y", is_dir).as_deref(),
            Some("https://x.dev/y")
        );
        let folder = Path::new("/work/app/src").to_string_lossy().into_owned();
        assert_eq!(
            opener_target("file:///work/app/src", is_dir).as_deref(),
            Some(folder.as_str())
        );
        assert_eq!(
            opener_target("file:///work/app/src/evil%20run.exe", is_dir).as_deref(),
            Some(folder.as_str())
        );
        assert_eq!(opener_target("file:///work/%ZZ", is_dir), None);
        // Bundles launch when opened, and so does anything inside one.
        assert_eq!(opener_target("file:///Apps/Evil.APP", |_| true), None);
        assert_eq!(opener_target("file:///Apps/Evil.app/x/y", |_| false), None);
    }

    #[cfg(windows)]
    #[test]
    fn a_drive_path_links_with_forward_slashes_and_shares_stay_unlinked() {
        let cwd = Path::new(r"C:\work\app");
        let found = find(
            r"see C:\work\app\src\a.rs:4 and C:/work/app/src/é.rs",
            cwd,
            &mut |_| true,
        );
        assert_eq!(found[0].url, "file:///C:/work/app/src/a.rs");
        assert_eq!(found[1].url, "file:///C:/work/app/src/%C3%A9.rs");
        assert!(find(r"\\server\share\x", cwd, &mut |_| true).is_empty());
    }
}
