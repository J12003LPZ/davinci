use super::{events::event_from_session_entry, ContextVmRuntime};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::PathBuf,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionSource {
    pub path: PathBuf,
    pub id: String,
}

/// Byte offsets of session entries already seen in the bound JSONL, so an older
/// `sourceRef` is one seek instead of a rescan from the header. The index is advisory:
/// every hit is re-parsed and its entry id and content hash are checked, and any
/// disagreement drops the index and rebuilds it from the file.
#[derive(Debug, Default)]
pub(crate) struct SourceIndex {
    path: Option<PathBuf>,
    session_id: String,
    /// Offset just past the last complete line scanned (0 until the header is skipped).
    scanned_to: u64,
    offsets: HashMap<String, u64>,
}

impl SourceIndex {
    fn reset_for(&mut self, binding: &SessionSource) {
        *self = Self {
            path: Some(binding.path.clone()),
            session_id: binding.id.clone(),
            ..Self::default()
        };
    }
}

impl ContextVmRuntime {
    pub fn bind_session_source(&self, path: PathBuf, id: String) {
        *self
            .session_source
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(SessionSource { path, id });
        *self.source_index.lock().unwrap_or_else(|e| e.into_inner()) = SourceIndex::default();
    }

    /// The session this VM reads authoritative sources from, if any.
    pub fn bound_session_id(&self) -> Option<String> {
        self.session_source
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(|source| source.id.clone())
    }

    /// Retained event text only; excludes small source metadata and page cache.
    pub fn resident_source_bytes(&self) -> usize {
        self.source_contents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .map(String::len)
            .sum()
    }

    pub(crate) fn source_content(&self, source_ref: &str) -> Result<String, String> {
        let binding = self
            .session_source
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let (Some(id), Some(binding)) = (source_ref.strip_prefix("session:"), binding) {
            let expected = self
                .events
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .find(|event| event.source_ref == source_ref)
                .map(|e| e.content_hash.clone())
                .ok_or("unknown context source_ref")?;
            return self.session_source_text(&binding, id, &expected);
        }
        self.source_contents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(source_ref)
            .cloned()
            .ok_or_else(|| "unknown context source_ref".into())
    }

    fn session_source_text(
        &self,
        binding: &SessionSource,
        id: &str,
        expected_hash: &str,
    ) -> Result<String, String> {
        let mut index = self.source_index.lock().unwrap_or_else(|e| e.into_inner());
        if index.path.as_ref() != Some(&binding.path) || index.session_id != binding.id {
            index.reset_for(binding);
        }
        let mut file = File::open(&binding.path).map_err(|_| "context session unavailable")?;
        let mut reader = BufReader::new(&mut file);
        let mut header = String::new();
        let header_len = reader
            .read_line(&mut header)
            .map_err(|_| "context session read failed")? as u64;
        if header_len == 0 {
            return Err("context session header missing".into());
        }
        let header = davinci_session::parse_header(header.trim_end())
            .map_err(|_| "invalid context session header")?;
        if header.id != binding.id {
            return Err("context session identity changed".into());
        }
        let file_len = reader
            .get_ref()
            .metadata()
            .map_err(|_| "context session read failed")?
            .len();
        if index.scanned_to > file_len {
            // The file shrank underneath the index: it was rewritten.
            index.reset_for(binding);
        }
        let mut rebuilt = false;
        loop {
            if let Some(&offset) = index.offsets.get(id) {
                match read_entry_at(&mut reader, offset, id) {
                    Some(entry) => {
                        let event = event_from_session_entry(&entry)
                            .ok_or("context source has no visible content")?;
                        if event.content_hash != expected_hash {
                            return Err("context source integrity check failed".into());
                        }
                        return Ok(event.visible_text);
                    }
                    None if !rebuilt => {
                        // Stale offset (file rewritten in place): rebuild from the header.
                        index.reset_for(binding);
                        rebuilt = true;
                        continue;
                    }
                    None => return Err("context source integrity check failed".into()),
                }
            }
            if !self.extend_index(&mut index, &mut reader, header_len, id)? {
                return Err("context source unavailable; replay required".into());
            }
        }
    }

    /// Scan forward from the last indexed position until `id` is indexed. Returns whether
    /// it was found. Only complete lines are indexed, so a half-written tail is retried later.
    fn extend_index(
        &self,
        index: &mut SourceIndex,
        reader: &mut BufReader<&mut File>,
        header_len: u64,
        id: &str,
    ) -> Result<bool, String> {
        let mut position = index.scanned_to.max(header_len);
        reader
            .seek(SeekFrom::Start(position))
            .map_err(|_| "context session read failed")?;
        let mut line = Vec::new();
        let mut scanned = 0u64;
        let mut found = false;
        loop {
            line.clear();
            let read = reader
                .read_until(b'\n', &mut line)
                .map_err(|_| "context session read failed")?;
            if read == 0 || line.last() != Some(&b'\n') {
                break;
            }
            scanned += 1;
            let start = position;
            position += read as u64;
            index.scanned_to = position;
            let text = String::from_utf8_lossy(&line);
            let Ok(davinci_session::SessionMutation::Entry { entry, .. }) =
                davinci_session::parse_mutation(text.trim_end())
            else {
                continue;
            };
            let matched = entry.id == id;
            index.offsets.entry(entry.id).or_insert(start);
            if matched {
                found = true;
                break;
            }
        }
        if scanned > 0 {
            self.metrics
                .write()
                .unwrap_or_else(|e| e.into_inner())
                .source_lines_scanned += scanned;
        }
        Ok(found)
    }
}

fn read_entry_at(
    reader: &mut BufReader<&mut File>,
    offset: u64,
    id: &str,
) -> Option<davinci_session::SessionEntry> {
    reader.seek(SeekFrom::Start(offset)).ok()?;
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    match davinci_session::parse_mutation(line.trim_end()) {
        Ok(davinci_session::SessionMutation::Entry { entry, .. }) if entry.id == id => Some(entry),
        _ => None,
    }
}
