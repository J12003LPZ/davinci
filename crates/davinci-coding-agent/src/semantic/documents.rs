//! Document version tracking, exact content hashing, and UTF-16/UTF-8 position conversions.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use davinci_agent::semantic::{Diagnostic, Position};

/// Converts a UTF-16 offset within a line to a byte index, returning None if splitting a surrogate.
pub fn utf16_to_byte(line: &str, offset: usize) -> Option<usize> {
    let mut units = 0;
    for (byte, ch) in line.char_indices() {
        if units == offset {
            return Some(byte);
        }
        units += ch.len_utf16();
        if units > offset {
            return None;
        }
    }
    if units == offset {
        Some(line.len())
    } else {
        None
    }
}

/// Converts a byte index within a line to a UTF-16 offset, returning None if not on char boundary.
pub fn byte_to_utf16(line: &str, byte_offset: usize) -> Option<usize> {
    if byte_offset > line.len() {
        return None;
    }
    let mut units = 0;
    for (byte, ch) in line.char_indices() {
        if byte == byte_offset {
            return Some(units);
        }
        if byte > byte_offset {
            return None;
        }
        units += ch.len_utf16();
    }
    if byte_offset == line.len() {
        Some(units)
    } else {
        None
    }
}

/// Computes a deterministic SHA-256 hash of document text content.
pub fn sha256_digest(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Tracks line start byte indices with CRLF, LF, and bare CR normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineIndex {
    line_starts: Vec<usize>,
    total_len: usize,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\r' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    line_starts.push(i + 2);
                    i += 2;
                    continue;
                } else {
                    line_starts.push(i + 1);
                    i += 1;
                    continue;
                }
            } else if bytes[i] == b'\n' {
                line_starts.push(i + 1);
            }
            i += 1;
        }

        Self {
            line_starts,
            total_len: text.len(),
        }
    }

    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }

    /// Converts (line, utf16_char) to global byte offset in document.
    pub fn position_to_byte(&self, text: &str, pos: Position) -> Option<usize> {
        let line_idx = pos.line as usize;
        if line_idx >= self.line_starts.len() {
            return None;
        }
        let line_start = self.line_starts[line_idx];
        let line_end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.total_len
        };

        let raw_line = &text[line_start..line_end];
        let stripped = raw_line.trim_end_matches(&['\r', '\n'][..]);
        let offset = utf16_to_byte(stripped, pos.character as usize)?;
        Some(line_start + offset)
    }

    /// Converts global byte offset to (line, utf16_char).
    pub fn byte_to_position(&self, text: &str, byte_offset: usize) -> Option<Position> {
        if byte_offset > self.total_len {
            return None;
        }
        let line_idx = match self.line_starts.binary_search(&byte_offset) {
            Ok(idx) => idx,
            Err(idx) => idx.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        let line_end = if line_idx + 1 < self.line_starts.len() {
            self.line_starts[line_idx + 1]
        } else {
            self.total_len
        };

        let raw_line = &text[line_start..line_end];
        let rel_byte = byte_offset - line_start;
        let utf16_col = byte_to_utf16(raw_line, rel_byte)?;

        Some(Position {
            line: line_idx as u32,
            character: utf16_col as u32,
        })
    }
}

/// Versioned document state with monotonic version counter and exact content hash.
#[derive(Debug, Clone)]
pub struct TrackedDocument {
    pub uri: String,
    pub path: PathBuf,
    pub version: i32,
    pub content: String,
    pub content_hash: String,
    pub line_index: LineIndex,
    pub diagnostics: Vec<Diagnostic>,
    pub diagnostics_version: Option<i32>,
    pub is_open: bool,
}

impl TrackedDocument {
    pub fn new(uri: String, path: PathBuf, content: String) -> Self {
        let content_hash = sha256_digest(&content);
        let line_index = LineIndex::new(&content);
        Self {
            uri,
            path,
            version: 1,
            content,
            content_hash,
            line_index,
            diagnostics: Vec::new(),
            diagnostics_version: None,
            is_open: true,
        }
    }

    pub fn update(&mut self, new_content: String) {
        self.version += 1;
        self.content_hash = sha256_digest(&new_content);
        self.line_index = LineIndex::new(&new_content);
        self.content = new_content;
    }
}

/// Manages open documents, version increments, and document-bound diagnostics.
#[derive(Debug, Default)]
pub struct DocumentTracker {
    documents: HashMap<PathBuf, TrackedDocument>,
}

impl DocumentTracker {
    pub fn new() -> Self {
        Self {
            documents: HashMap::new(),
        }
    }

    /// Opens or tracks a document, setting initial version 1.
    pub fn did_open(&mut self, uri: String, path: PathBuf, content: String) -> i32 {
        let doc = TrackedDocument::new(uri, path.clone(), content);
        self.documents.insert(path, doc);
        1
    }

    /// Updates document with new content, incrementing monotonic version.
    pub fn did_change(&mut self, path: &Path, new_content: String) -> Result<i32, String> {
        let doc = self
            .documents
            .get_mut(path)
            .ok_or_else(|| format!("Document {} is not open", path.display()))?;
        doc.update(new_content);
        Ok(doc.version)
    }

    /// Marks document as closed. Does not erase diagnostics history immediately.
    pub fn did_close(&mut self, path: &Path) -> Result<(), String> {
        let doc = self
            .documents
            .get_mut(path)
            .ok_or_else(|| format!("Document {} is not open", path.display()))?;
        doc.is_open = false;
        Ok(())
    }

    /// Records diagnostics attached to a specific document version.
    pub fn record_diagnostics(
        &mut self,
        path: &Path,
        version: Option<i32>,
        diagnostics: Vec<Diagnostic>,
    ) {
        if let Some(doc) = self.documents.get_mut(path) {
            doc.diagnostics = diagnostics;
            doc.diagnostics_version = version;
        }
    }

    pub fn get_document(&self, path: &Path) -> Option<&TrackedDocument> {
        self.documents.get(path)
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.documents.get(path).is_some_and(|d| d.is_open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_agent::semantic::Range;

    #[test]
    fn f10_utf16_boundaries() {
        assert_eq!(utf16_to_byte("a😀b", 1), Some(1));
        assert_eq!(utf16_to_byte("a😀b", 2), None);
        assert_eq!(utf16_to_byte("a😀b", 3), Some(5));
        assert_eq!(utf16_to_byte("a😀b", 4), Some(6));
    }

    #[test]
    fn test_crlf_and_bare_cr() {
        let text = "line1\r\nline2\rline3\nline4";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 4);

        let p1 = Position {
            line: 0,
            character: 0,
        };
        assert_eq!(index.position_to_byte(text, p1), Some(0));

        let p2 = Position {
            line: 1,
            character: 0,
        };
        assert_eq!(index.position_to_byte(text, p2), Some(7));

        let p3 = Position {
            line: 2,
            character: 0,
        };
        assert_eq!(index.position_to_byte(text, p3), Some(13));

        let p4 = Position {
            line: 3,
            character: 0,
        };
        assert_eq!(index.position_to_byte(text, p4), Some(19));
    }

    #[test]
    fn test_astral_unicode() {
        let text = "🚀 rocket 🌟 star";
        assert_eq!(utf16_to_byte(text, 0), Some(0));
        assert_eq!(utf16_to_byte(text, 1), None); // Half of rocket emoji surrogate
        assert_eq!(utf16_to_byte(text, 2), Some(4)); // Space after rocket
    }

    #[test]
    fn test_combining_marks() {
        let text = "e\u{0301} cafe"; // e + combining acute accent
        assert_eq!(utf16_to_byte(text, 0), Some(0));
        assert_eq!(utf16_to_byte(text, 1), Some(1));
        assert_eq!(utf16_to_byte(text, 2), Some(3)); // After accent
    }

    #[test]
    fn test_empty_final_line() {
        let text = "first line\n";
        let index = LineIndex::new(text);
        assert_eq!(index.line_count(), 2);

        let p_end = Position {
            line: 1,
            character: 0,
        };
        assert_eq!(index.position_to_byte(text, p_end), Some(11));
    }

    #[test]
    fn test_same_size_edit() {
        let mut tracker = DocumentTracker::new();
        let path = PathBuf::from("edit.txt");
        let v1 = tracker.did_open("file:///edit.txt".into(), path.clone(), "abcd".into());
        assert_eq!(v1, 1);

        let doc1 = tracker.get_document(&path).unwrap();
        let hash1 = doc1.content_hash.clone();

        // Same size replacement
        let v2 = tracker.did_change(&path, "wxyz".into()).unwrap();
        assert_eq!(v2, 2);

        let doc2 = tracker.get_document(&path).unwrap();
        assert_ne!(doc2.content_hash, hash1);
    }

    #[test]
    fn test_diagnostic_arrives_after_document_close() {
        let mut tracker = DocumentTracker::new();
        let path = PathBuf::from("foo.rs");
        tracker.did_open("file:///foo.rs".into(), path.clone(), "fn main() {}".into());
        tracker.did_close(&path).unwrap();
        assert!(!tracker.is_open(&path));

        // Diagnostics arriving for closed document are stored with originating version without crash
        let diag = Diagnostic {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end: Position {
                    line: 0,
                    character: 7,
                },
            },
            severity: davinci_agent::semantic::DiagnosticSeverity::Warning,
            code: Some("unused".into()),
            source: Some("rustc".into()),
            message: "function is never used".into(),
        };
        tracker.record_diagnostics(&path, Some(1), vec![diag]);

        let doc = tracker.get_document(&path).unwrap();
        assert_eq!(doc.diagnostics.len(), 1);
        assert_eq!(doc.diagnostics_version, Some(1));
        assert!(!doc.is_open);
    }
}
