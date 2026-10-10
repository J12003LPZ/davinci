use super::{events::event_from_session_entry, ContextVmRuntime};
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::PathBuf,
};

/// The largest session record a source lookup buffers. `read_until` holds a
/// newline-free record whole, so a corrupt or enormous line could allocate
/// its full size even when the caller wants one small excerpt.
pub(crate) const MAX_SOURCE_RECORD_BYTES: usize = 16 * 1024 * 1024;
/// The largest header line accepted. Real headers are a few hundred bytes.
const MAX_HEADER_BYTES: usize = 256 * 1024;

/// One record read with a ceiling.
enum Record {
    /// A complete line in the buffer; the value is its length on disk.
    Line(u64),
    /// A complete line past the ceiling, skipped without being kept. The
    /// buffer holds its first `max` bytes; the value is its length on disk.
    Oversized(u64),
    /// End of file, or a final line with no newline yet.
    End,
}

fn read_record(
    reader: &mut impl BufRead,
    max: usize,
    line: &mut Vec<u8>,
) -> std::io::Result<Record> {
    line.clear();
    let read = reader
        .by_ref()
        .take(max as u64 + 1)
        .read_until(b'\n', line)? as u64;
    if line.last() == Some(&b'\n') {
        return Ok(Record::Line(read));
    }
    if line.len() <= max {
        return Ok(Record::End);
    }
    let mut consumed = read;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(Record::End);
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                reader.consume(index + 1);
                return Ok(Record::Oversized(consumed + index as u64 + 1));
            }
            None => {
                let len = available.len();
                consumed += len as u64;
                reader.consume(len);
            }
        }
    }
}

fn oversized(id: &str) -> String {
    format!("context source {id} exceeds {MAX_SOURCE_RECORD_BYTES} bytes")
}

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
        let mut header = Vec::new();
        let header_len = match read_record(&mut reader, MAX_HEADER_BYTES, &mut header)
            .map_err(|_| "context session read failed")?
        {
            Record::Line(len) => len,
            Record::Oversized(_) => return Err("invalid context session header".into()),
            Record::End => return Err("context session header missing".into()),
        };
        let header = davinci_session::parse_header(String::from_utf8_lossy(&header).trim_end())
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
                match read_entry_at(&mut reader, offset, id)? {
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
        let wanted = format!("\"id\":\"{id}\"");
        loop {
            let read = match read_record(reader, MAX_SOURCE_RECORD_BYTES, &mut line)
                .map_err(|_| "context session read failed")?
            {
                Record::End => break,
                Record::Line(read) => read,
                Record::Oversized(read) => {
                    scanned += 1;
                    position += read;
                    index.scanned_to = position;
                    // Its id sits near the start; if this is the record asked
                    // for, say so rather than "unavailable".
                    if line
                        .windows(wanted.len())
                        .any(|window| window == wanted.as_bytes())
                    {
                        return Err(oversized(id));
                    }
                    continue;
                }
            };
            scanned += 1;
            let start = position;
            position += read;
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

/// The entry at `offset` when it is `id`. `Ok(None)` means the offset is
/// stale; a record past the ceiling is an error, never buffered.
fn read_entry_at(
    reader: &mut BufReader<&mut File>,
    offset: u64,
    id: &str,
) -> Result<Option<davinci_session::SessionEntry>, String> {
    if reader.seek(SeekFrom::Start(offset)).is_err() {
        return Ok(None);
    }
    let mut line = Vec::new();
    match read_record(reader, MAX_SOURCE_RECORD_BYTES, &mut line) {
        Ok(Record::Line(_)) => {}
        Ok(Record::Oversized(_)) => return Err(oversized(id)),
        Ok(Record::End) | Err(_) => return Ok(None),
    }
    Ok(
        match davinci_session::parse_mutation(String::from_utf8_lossy(&line).trim_end()) {
            Ok(davinci_session::SessionMutation::Entry { entry, .. }) if entry.id == id => {
                Some(entry)
            }
            _ => None,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_past_the_ceiling_are_skipped_whole_and_reading_resumes() {
        let data = format!("ok\n{}\nnext\npartial", "x".repeat(100));
        let mut reader = BufReader::with_capacity(8, data.as_bytes());
        let mut line = Vec::new();
        assert!(matches!(
            read_record(&mut reader, 10, &mut line).unwrap(),
            Record::Line(3)
        ));
        assert!(matches!(
            read_record(&mut reader, 10, &mut line).unwrap(),
            Record::Oversized(101)
        ));
        assert!(line.len() <= 11, "{} bytes kept", line.len());
        assert!(matches!(
            read_record(&mut reader, 10, &mut line).unwrap(),
            Record::Line(5)
        ));
        assert_eq!(line, b"next\n");
        assert!(matches!(
            read_record(&mut reader, 10, &mut line).unwrap(),
            Record::End
        ));
    }
}
