use super::{
    events::{event_from_session_entry, EventImage},
    ArtifactRef, ContextVmRuntime,
};
use crate::evidence::{session_token, SCRIPT_INVENTORY_URI, SCRIPT_LOG_URI, SCRIPT_OUTPUT_URI};
use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::Arc,
};

/// The most bytes of one Script log or output a `retrieve_context` call can
/// load. What lies past it is reported as cut, never dropped silently; the
/// stores themselves retain about 1 MiB.
pub(crate) const MAX_SCRIPT_EVIDENCE_BYTES: usize = 2 * 1024 * 1024;

/// Where the host keeps this session's Script logs and outputs, and whether
/// the permission policy would still let the model read a given file. The
/// host sets it; nothing the model sends reaches it.
#[derive(Clone)]
pub struct ScriptEvidenceBinding {
    pub dir: PathBuf,
    pub session_id: String,
    pub readable: Arc<dyn Fn(&Path) -> bool + Send + Sync>,
    /// The open background Scripts, one line each, from the run registry.
    pub inventory: Arc<dyn Fn() -> String + Send + Sync>,
}

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

/// The most image data (base64 bytes) a sessionless VM keeps in memory so
/// `retrieve_context` can hand images back. A bound session reads them from
/// its JSONL instead and needs none of this.
pub(crate) const MAX_SESSIONLESS_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Image bytes of sources a sessionless VM holds, within a byte bound. A
/// source whose images did not fit is remembered so retrieval can say the
/// images are gone rather than imply the text is all there was.
#[derive(Debug, Default, Clone)]
pub(crate) struct SourceImages {
    kept: HashMap<String, Vec<EventImage>>,
    dropped: HashSet<String>,
    bytes: usize,
}

impl SourceImages {
    pub(crate) fn insert(&mut self, source_ref: &str, images: &[EventImage]) {
        if images.is_empty() || self.kept.contains_key(source_ref) {
            return;
        }
        let size: usize = images.iter().map(|image| image.data.len()).sum();
        if self.bytes.saturating_add(size) > MAX_SESSIONLESS_IMAGE_BYTES {
            self.dropped.insert(source_ref.to_string());
            return;
        }
        self.bytes += size;
        self.kept.insert(source_ref.to_string(), images.to_vec());
    }

    /// The entries for `refs`, to survive a rewrite of the transcript.
    pub(crate) fn subset<'a>(&self, refs: impl Iterator<Item = &'a String>) -> Self {
        let mut out = Self::default();
        for source_ref in refs {
            if let Some(images) = self.kept.get(source_ref) {
                out.insert(source_ref, images);
            } else if self.dropped.contains(source_ref) {
                out.dropped.insert(source_ref.clone());
            }
        }
        out
    }

    /// Whether any held entry belongs to a source outside `live`: the
    /// transcript was rewritten, so the held set must be rebuilt.
    pub(crate) fn holds_source_outside(&self, live: &HashSet<&str>) -> bool {
        self.kept
            .keys()
            .chain(self.dropped.iter())
            .any(|source_ref| !live.contains(source_ref.as_str()))
    }

    /// Where the held bytes of `source_ref` live, so a test can tell reuse
    /// from a copy.
    #[cfg(test)]
    pub(crate) fn data_ptr(&self, source_ref: &str) -> Option<*const u8> {
        self.kept
            .get(source_ref)
            .and_then(|images| images.first())
            .map(|image| image.data.as_ptr())
    }

    pub(crate) fn absorb(&mut self, other: Self) {
        for (source_ref, images) in other.kept {
            self.insert(&source_ref, &images);
        }
        self.dropped.extend(other.dropped);
    }
}

/// What a source holds: its text, and the images it carried.
pub(crate) struct SourceMaterial {
    pub text: String,
    pub images: Vec<EventImage>,
    /// The source had images that this VM no longer holds (sessionless, past
    /// the in-memory bound).
    pub images_not_retained: bool,
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
        [&self.source_contents, &self.retained_sources]
            .iter()
            .map(|sources| {
                sources
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .values()
                    .map(String::len)
                    .sum::<usize>()
            })
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn source_content(&self, source_ref: &str) -> Result<String, String> {
        self.source_material(source_ref)
            .map(|material| material.text)
    }

    pub(crate) fn source_material(&self, source_ref: &str) -> Result<SourceMaterial, String> {
        if source_ref == SCRIPT_INVENTORY_URI {
            return self.script_inventory_material();
        }
        if source_ref.starts_with(SCRIPT_LOG_URI) || source_ref.starts_with(SCRIPT_OUTPUT_URI) {
            return self.script_evidence_material(source_ref);
        }
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
            let (text, images, _) = self.session_source_text(&binding, id, &expected)?;
            return Ok(SourceMaterial {
                text,
                images,
                images_not_retained: false,
            });
        }
        let text = match self
            .source_contents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(source_ref)
        {
            Some(text) => text.clone(),
            None => self
                .retained_sources
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .get(source_ref)
                .cloned()
                .ok_or("unknown context source_ref")?,
        };
        let held = self.source_images.read().unwrap_or_else(|e| e.into_inner());
        Ok(SourceMaterial {
            text,
            images: held.kept.get(source_ref).cloned().unwrap_or_default(),
            images_not_retained: held.dropped.contains(source_ref),
        })
    }

    /// Bind where Script evidence is kept. Re-binding replaces the earlier one.
    pub fn bind_script_evidence(&self, binding: ScriptEvidenceBinding) {
        *self
            .script_evidence
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(binding);
    }

    /// The full list of open background Scripts. It is built by the host from
    /// the run registry for this session, workspace and branch, so there is no
    /// session entry to anchor it to; the ledger cites it when it does not fit.
    fn script_inventory_material(&self) -> Result<SourceMaterial, String> {
        let binding = self
            .script_evidence
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("script evidence unavailable: no evidence store is bound")?;
        let mut text = (binding.inventory)();
        if text.is_empty() {
            text = "No open background Scripts.".into();
        }
        if text.len() > MAX_SCRIPT_EVIDENCE_BYTES {
            text =
                crate::evidence::cut_at_char_boundary(&text, MAX_SCRIPT_EVIDENCE_BYTES).to_string();
            text.push_str(
                "
[truncated]",
            );
        }
        Ok(SourceMaterial {
            text,
            images: Vec::new(),
            images_not_retained: false,
        })
    }

    /// The text behind a `script-log://` or `script-output://` reference.
    /// The reference must be one a tool result in this conversation carries
    /// (a foreign branch's is not in `events`), and, with a bound session, the
    /// authoritative session entry must still name it. The file is then found
    /// by the host's own naming, inside the evidence directory, for this
    /// session only, and only while the permission policy still allows it.
    fn script_evidence_material(&self, uri: &str) -> Result<SourceMaterial, String> {
        let unavailable = |why: &str| format!("script evidence unavailable: {why}");
        let (entry_ref, expected) = self
            .events
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .find(|event| {
                event
                    .artifact_refs
                    .iter()
                    .any(|artifact| artifact.uri == uri)
            })
            .map(|event| (event.source_ref.clone(), event.content_hash.clone()))
            .ok_or("unknown context source_ref")?;
        let session = self
            .session_source
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let (Some(id), Some(session)) = (entry_ref.strip_prefix("session:"), session) {
            let (_, _, refs) = self.session_source_text(&session, id, &expected)?;
            if !refs.iter().any(|artifact| artifact.uri == uri) {
                return Err(unavailable("the session entry does not name it"));
            }
        }
        let binding = self
            .script_evidence
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or_else(|| unavailable("no evidence store is bound"))?;
        let (name, is_log) = match uri.strip_prefix(SCRIPT_LOG_URI) {
            Some(id) => (id, true),
            None => (
                uri.strip_prefix(SCRIPT_OUTPUT_URI).unwrap_or_default(),
                false,
            ),
        };
        if !script_evidence_name_ok(name, is_log, &binding.session_id) {
            return Err(unavailable("malformed or foreign reference"));
        }
        let file_name = format!("{name}.{}", if is_log { "jsonl" } else { "txt" });
        let real = contained_file(&binding.dir, &file_name).map_err(unavailable)?;
        if !(binding.readable)(&real) {
            return Err(unavailable(
                "the current permissions no longer allow reading it",
            ));
        }
        let (mut text, cut) = if is_log {
            let size = std::fs::metadata(&real)
                .map_err(|_| unavailable("file is unreadable"))?
                .len();
            if size > MAX_SCRIPT_EVIDENCE_BYTES as u64 {
                return Err(unavailable("log exceeds the retrieval bound"));
            }
            let slice = crate::codemode::ScriptLogStore::new(&binding.dir, &binding.session_id)
                .read_script_log(name, 0, MAX_SCRIPT_EVIDENCE_BYTES)
                .map_err(|error| unavailable(&error.message))?;
            (slice.text, !slice.complete)
        } else {
            read_utf8_prefix(&real, MAX_SCRIPT_EVIDENCE_BYTES)
                .map_err(|_| unavailable("file is unreadable"))?
        };
        if cut {
            text.push_str(&format!(
                "\n[truncated: only the first {MAX_SCRIPT_EVIDENCE_BYTES} bytes of this script evidence are retrievable]"
            ));
        }
        Ok(SourceMaterial {
            text,
            images: Vec::new(),
            images_not_retained: false,
        })
    }

    fn session_source_text(
        &self,
        binding: &SessionSource,
        id: &str,
        expected_hash: &str,
    ) -> Result<(String, Vec<EventImage>, Vec<ArtifactRef>), String> {
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
                        return Ok((event.visible_text, event.images, event.artifact_refs));
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

/// A name the host would have given: `script-log-<12 hex>`, or this session's
/// `s<token>-script-output-<12 hex>`. Nothing else is ever joined to a path.
fn script_evidence_name_ok(name: &str, is_log: bool, session_id: &str) -> bool {
    let tail_ok = |tail: &str| tail.len() == 12 && tail.chars().all(|c| c.is_ascii_hexdigit());
    if is_log {
        name.strip_prefix("script-log-").is_some_and(tail_ok)
    } else {
        name.strip_prefix(&format!("s{}-script-output-", session_token(session_id)))
            .is_some_and(tail_ok)
    }
}

/// `dir/file_name` when it is the evidence directory's own child after
/// resolving links. A symlink or junction resolves elsewhere and fails.
fn contained_file(dir: &Path, file_name: &str) -> Result<PathBuf, &'static str> {
    let real = std::fs::canonicalize(dir.join(file_name)).map_err(|_| "file is missing")?;
    let root = std::fs::canonicalize(dir).map_err(|_| "evidence store is missing")?;
    if real != root.join(file_name) {
        return Err("file is outside the evidence store");
    }
    Ok(real)
}

/// Up to `cap` bytes of a file as text, cut on a character boundary. The
/// flag says whether more followed.
fn read_utf8_prefix(path: &Path, cap: usize) -> std::io::Result<(String, bool)> {
    let mut bytes = Vec::new();
    File::open(path)?
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)?;
    let cut = bytes.len() > cap;
    bytes.truncate(cap);
    Ok((
        match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(error) => {
                let valid = error.utf8_error().valid_up_to();
                let split_tail = cut && error.utf8_error().error_len().is_none();
                let bytes = error.into_bytes();
                let end = if split_tail { valid } else { bytes.len() };
                String::from_utf8_lossy(&bytes[..end]).into_owned()
            }
        },
        cut,
    ))
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

    #[test]
    fn script_evidence_names_are_only_the_hosts_own() {
        let session = "session-a";
        let token = session_token(session);
        assert!(script_evidence_name_ok(
            "script-log-0123456789ab",
            true,
            session
        ));
        assert!(!script_evidence_name_ok(
            "script-log-0123456789a",
            true,
            session
        ));
        assert!(!script_evidence_name_ok(
            "script-log-../../x12345",
            true,
            session
        ));
        assert!(!script_evidence_name_ok(
            "script-log-0123456789ab.jsonl",
            true,
            session
        ));
        let own = format!("s{token}-script-output-0123456789ab");
        assert!(script_evidence_name_ok(&own, false, session));
        // Another session's output carries another token.
        assert!(!script_evidence_name_ok(&own, false, "session-b"));
        assert!(!script_evidence_name_ok(
            "../s-script-output-0123456789ab",
            false,
            session
        ));
    }

    #[test]
    fn a_link_out_of_the_evidence_store_is_not_contained() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("evidence");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("script-log-0123456789ab.jsonl"), "own").unwrap();
        assert!(contained_file(&dir, "script-log-0123456789ab.jsonl").is_ok());
        assert!(contained_file(&dir, "script-log-ffffffffffff.jsonl").is_err());

        let outside = root.path().join("outside.jsonl");
        std::fs::write(&outside, "secret").unwrap();
        let link = dir.join("script-log-aaaaaaaaaaaa.jsonl");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&outside, &link);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&outside, &link);
        // Creating a symlink can need a privilege the runner lacks; the
        // containment rule is exercised wherever one can be made.
        if linked.is_ok() {
            assert_eq!(
                contained_file(&dir, "script-log-aaaaaaaaaaaa.jsonl"),
                Err("file is outside the evidence store")
            );
        }
    }

    #[test]
    fn a_prefix_read_stops_on_a_character_boundary_and_says_it_was_cut() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        // Two-byte characters: a cap of 5 splits the third one.
        std::fs::write(&path, "éééé").unwrap();
        assert_eq!(
            read_utf8_prefix(&path, 5).unwrap(),
            ("éé".to_string(), true)
        );
        assert_eq!(
            read_utf8_prefix(&path, 8).unwrap(),
            ("éééé".to_string(), false)
        );
        assert_eq!(
            read_utf8_prefix(&path, 100).unwrap(),
            ("éééé".to_string(), false)
        );
    }
}
