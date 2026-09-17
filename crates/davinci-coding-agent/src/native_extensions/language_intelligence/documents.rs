//! Canonical document identity, bounded disk reads and UTF-16 positions.

use super::protocol::{IntelligenceError, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(super) const MAX_SOURCE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone)]
pub(super) struct Document {
    pub path: PathBuf,
    pub uri: String,
    pub version: i64,
    pub text: String,
}

impl Document {
    pub fn open(path: PathBuf, language: &str, text: String) -> Result<(Self, Vec<Value>)> {
        let uri = file_uri(&path)?;
        let event = json!({"method":"textDocument/didOpen", "params":{"textDocument":{
            "uri":uri, "languageId":language, "version":1, "text":text
        }}});
        Ok((
            Self {
                path,
                uri,
                version: 1,
                text,
            },
            vec![event],
        ))
    }

    pub fn refreshed(
        &self,
        text: String,
        sync_kind: u64,
        save: Option<bool>,
    ) -> Result<(Self, Vec<Value>)> {
        if text == self.text {
            return Ok((self.clone(), Vec::new()));
        }
        if !matches!(sync_kind, 1 | 2) {
            return Err(IntelligenceError::new(
                "unsupported_document_sync",
                "Server does not support document changes",
            ));
        }
        let version = self
            .version
            .checked_add(1)
            .ok_or_else(|| IntelligenceError::new("protocol_error", "Document version overflow"))?;
        let mut change = json!({"text":text});
        if sync_kind == 2 {
            let mut lines = self.text.split('\n');
            let first = lines.next().unwrap_or("");
            let (line, last) = lines
                .enumerate()
                .last()
                .map(|(i, s)| (i + 1, s))
                .unwrap_or((0, first));
            change["range"] = json!({"start":{"line":0,"character":0}, "end":{"line":line,"character":last.encode_utf16().count()}});
        }
        let mut events = vec![json!({"method":"textDocument/didChange", "params":{
            "textDocument":{"uri":self.uri,"version":version}, "contentChanges":[change]
        }})];
        if let Some(include_text) = save {
            let mut params = json!({"textDocument":{"uri":self.uri}});
            if include_text {
                params["text"] = json!(text);
            }
            events.push(json!({"method":"textDocument/didSave","params":params}));
        }
        Ok((
            Self {
                text,
                version,
                ..self.clone()
            },
            events,
        ))
    }
}

pub(super) fn file_uri(path: &Path) -> Result<String> {
    url::Url::from_file_path(path)
        .map(String::from)
        .map_err(|_| {
            IntelligenceError::new(
                "invalid_source_path",
                "Source path cannot be represented by a file URI",
            )
        })
}

pub(super) fn source_path(workspace: &Path, raw: &str) -> Result<PathBuf> {
    let invalid = || {
        IntelligenceError::new(
            "invalid_source_path",
            "Expected an existing UTF-8 source file inside the workspace",
        )
    };
    if raw.is_empty() || raw.contains('\0') {
        return Err(invalid());
    }
    let root = workspace.canonicalize().map_err(|_| invalid())?;
    let path = root.join(raw).canonicalize().map_err(|_| invalid())?;
    if !path.starts_with(&root) {
        return Err(IntelligenceError::new(
            "outside_workspace",
            "Source path escapes the workspace",
        ));
    }
    if !path.is_file() {
        return Err(invalid());
    }
    Ok(path)
}

pub(super) fn read_source(path: &Path) -> Result<String> {
    let invalid = || {
        IntelligenceError::new(
            "invalid_source_path",
            "Source file could not be read as UTF-8",
        )
    };
    let file = std::fs::File::open(path).map_err(|_| invalid())?;
    let mut bytes = Vec::new();
    file.take((MAX_SOURCE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| invalid())?;
    if bytes.len() > MAX_SOURCE_BYTES {
        return Err(IntelligenceError::new(
            "source_too_large",
            "Source exceeds the 1 MiB synchronization limit",
        ));
    }
    String::from_utf8(bytes).map_err(|_| invalid())
}

pub(super) fn position(text: &str, line: u64, column: u64) -> Result<Value> {
    let invalid = || {
        IntelligenceError::new(
            "invalid_position",
            "Expected a valid 1-based line and UTF-16 column",
        )
    };
    let row = usize::try_from(line.checked_sub(1).ok_or_else(invalid)?).map_err(|_| invalid())?;
    let offset = column.checked_sub(1).ok_or_else(invalid)?;
    let content = text
        .split('\n')
        .nth(row)
        .ok_or_else(invalid)?
        .trim_end_matches('\r');
    let mut units = 0u64;
    for character in content.chars() {
        if units >= offset {
            break;
        }
        units += character.len_utf16() as u64;
    }
    if units != offset {
        return Err(invalid());
    }
    Ok(json!({"line": row, "character": offset}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_paths_reject_escape_missing_and_nonfiles() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("project")).unwrap();
        std::fs::write(root.join("outside.ts"), "x").unwrap();
        std::fs::write(root.join("project/a.ts"), "text").unwrap();
        let project = root.join("project");
        assert_eq!(source_path(&project, "a.ts").unwrap(), project.join("a.ts"));
        assert_eq!(
            source_path(&project, "../outside.ts").unwrap_err().code,
            "outside_workspace"
        );
        assert_eq!(
            source_path(&project, "missing.ts").unwrap_err().code,
            "invalid_source_path"
        );
        assert_eq!(
            source_path(&project, ".").unwrap_err().code,
            "invalid_source_path"
        );
    }

    #[test]
    fn source_reads_bound_size_and_require_utf8() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.ts");
        std::fs::write(&path, "hello λ").unwrap();
        assert_eq!(read_source(&path).unwrap(), "hello λ");
        std::fs::write(&path, [0xff]).unwrap();
        assert!(read_source(&path).is_err());
        std::fs::write(&path, vec![b'a'; MAX_SOURCE_BYTES + 1]).unwrap();
        assert_eq!(read_source(&path).unwrap_err().code, "source_too_large");
    }

    #[cfg(unix)]
    #[test]
    fn symlink_sources_cannot_escape_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let target = outside.path().join("outside.ts");
        std::fs::write(&target, "private").unwrap();
        std::os::unix::fs::symlink(target, root.join("alias.ts")).unwrap();
        assert_eq!(
            source_path(&root, "alias.ts").unwrap_err().code,
            "outside_workspace"
        );
    }

    #[test]
    fn positions_are_one_based_utf16_and_check_surrogate_boundaries() {
        let text = "a🦀b\r\nx\n";
        assert_eq!(
            position(text, 1, 4).unwrap(),
            json!({"line":0,"character":3})
        );
        assert_eq!(
            position(text, 1, 5).unwrap(),
            json!({"line":0,"character":4})
        );
        assert_eq!(
            position(text, 3, 1).unwrap(),
            json!({"line":2,"character":0})
        );
        for (line, column) in [(0, 1), (1, 0), (1, 3), (1, 6), (4, 1)] {
            assert_eq!(
                position(text, line, column).unwrap_err().code,
                "invalid_position"
            );
        }
    }

    #[test]
    fn document_opens_once_changes_versions_and_skips_unchanged_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().canonicalize().unwrap().join("space λ.ts");
        std::fs::write(&path, "let x = 1;").unwrap();
        let (document, events) =
            Document::open(path.clone(), "typescript", "let x = 1;".into()).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0]["method"], "textDocument/didOpen");
        assert_eq!(
            events[0]["params"]["textDocument"]["languageId"],
            "typescript"
        );
        // Windows canonical paths use a verbatim prefix; file URIs do not.
        assert_eq!(
            url::Url::parse(&document.uri)
                .unwrap()
                .to_file_path()
                .unwrap()
                .canonicalize()
                .unwrap(),
            path
        );
        let (same, events) = document
            .refreshed(document.text.clone(), 1, Some(true))
            .unwrap();
        assert!(events.is_empty());
        assert_eq!(same.version, 1);
        let (updated, events) = document
            .refreshed("let x = 2;".into(), 1, Some(true))
            .unwrap();
        assert_eq!(updated.version, 2);
        assert_eq!(
            document.version, 1,
            "failed sends must not advance the previous state"
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["method"], "textDocument/didChange");
        assert_eq!(
            events[0]["params"]["contentChanges"][0],
            json!({"text":"let x = 2;"})
        );
        assert_eq!(events[1]["method"], "textDocument/didSave");
        assert_eq!(events[1]["params"]["text"], "let x = 2;");
    }

    #[test]
    fn incremental_servers_receive_full_old_range_in_utf16() {
        let dir = tempfile::tempdir().unwrap();
        let (document, _) =
            Document::open(dir.path().join("a.ts"), "typescript", "a\n🦀".into()).unwrap();
        let (_, events) = document.refreshed("b".into(), 2, None).unwrap();
        assert_eq!(
            events[0]["params"]["contentChanges"][0]["range"],
            json!({"start":{"line":0,"character":0},"end":{"line":1,"character":2}})
        );
        assert_eq!(
            document.refreshed("b".into(), 0, None).unwrap_err().code,
            "unsupported_document_sync"
        );
    }
}
