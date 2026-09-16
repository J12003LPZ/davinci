//! Source-bound semantic query execution, URI conversions, and scope verification.

use std::fs;
use std::path::{Path, PathBuf};

use super::documents::sha256_digest;

/// Converts a local filesystem path to a canonical `file://` URI.
pub fn path_to_uri(path: &Path) -> String {
    let path_str = path.to_string_lossy().replace('\\', "/");
    let mut encoded = String::with_capacity(path_str.len() + 8);

    for b in path_str.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' | b':' => {
                encoded.push(b as char);
            }
            b' ' => encoded.push_str("%20"),
            other => {
                encoded.push_str(&format!("%{:02X}", other));
            }
        }
    }

    if encoded.starts_with("//") {
        // UNC path: //server/share -> file://server/share
        format!("file:{}", encoded)
    } else if encoded.starts_with('/') {
        format!("file://{}", encoded)
    } else {
        format!("file:///{}", encoded)
    }
}

/// Parses a `file://` URI into a normalized local PathBuf.
pub fn uri_to_path(uri: &str) -> Result<PathBuf, String> {
    let stripped = uri
        .strip_prefix("file://")
        .ok_or_else(|| format!("Invalid URI scheme (expected file://): {uri}"))?;

    // Decode percent-encoded octets
    let mut decoded_bytes = Vec::new();
    let bytes = stripped.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(val) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                decoded_bytes.push(val);
                i += 3;
                continue;
            }
        }
        decoded_bytes.push(bytes[i]);
        i += 1;
    }

    let decoded_str =
        String::from_utf8(decoded_bytes).map_err(|e| format!("Invalid UTF-8 in URI: {e}"))?;

    if decoded_str.starts_with('/') && decoded_str.len() >= 3 && decoded_str.as_bytes()[2] == b':' {
        // Windows drive: /C:/path -> C:/path
        let win_path = &decoded_str[1..];
        Ok(PathBuf::from(win_path.replace('/', "\\")))
    } else {
        Ok(PathBuf::from(decoded_str))
    }
}

/// Validates whether a target path is contained within an authorized workspace root.
pub fn is_path_in_root(target: &Path, root: &Path) -> bool {
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target)
    };
    let (outside_lexical, symlink_escape) = davinci_agent::check_path_boundary(root, &target);
    !outside_lexical && !symlink_escape
}

/// Verifies whether the on-disk file content matches the expected hash.
pub fn verify_source_freshness(path: &Path, expected_hash: &str) -> bool {
    if let Ok(content) = fs::read_to_string(path) {
        sha256_digest(&content) == expected_hash
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_uri_percent_encoding() {
        let p = Path::new("/workspace/my project/file#1.rs");
        let uri = path_to_uri(p);
        assert!(uri.contains("%20"));
        assert!(uri.contains("%23"));

        let roundtrip = uri_to_path(&uri).unwrap();
        assert_eq!(roundtrip, p);
    }

    #[test]
    fn test_windows_drive_and_unc() {
        // Windows drive path
        let win_p = Path::new("C:\\Users\\admin\\repo\\main.rs");
        let uri = path_to_uri(win_p);
        assert!(uri.starts_with("file:///C:/") || uri.starts_with("file:///c:/"));
        let decoded = uri_to_path(&uri).unwrap();
        assert_eq!(decoded, PathBuf::from("C:\\Users\\admin\\repo\\main.rs"));

        // UNC path
        let unc_p = Path::new("\\\\fileserver\\share\\code.rs");
        let unc_uri = path_to_uri(unc_p);
        assert!(unc_uri.starts_with("file://fileserver/share"));
    }

    #[test]
    fn test_generated_file_changed_midquery() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("generated.rs");
        fs::write(&file_path, "original content").unwrap();

        let orig_hash = sha256_digest("original content");
        assert!(verify_source_freshness(&file_path, &orig_hash));

        // File modified concurrently / mid-query
        fs::write(&file_path, "modified by build tool").unwrap();
        assert!(!verify_source_freshness(&file_path, &orig_hash));
    }

    #[test]
    fn test_path_in_root_boundary() {
        let dir = tempdir().unwrap();
        let inside = dir.path().join("src/lib.rs");
        let outside = PathBuf::from("/etc/passwd");

        assert!(is_path_in_root(&inside, dir.path()));
        assert!(!is_path_in_root(&outside, dir.path()));
        assert!(!is_path_in_root(
            &dir.path()
                .join("missing")
                .join("..")
                .join("..")
                .join("escape.rs"),
            dir.path()
        ));
    }
}
