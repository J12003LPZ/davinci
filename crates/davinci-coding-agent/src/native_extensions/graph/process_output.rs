//! Bounded line assembly; oversized lines retain their original bytes on disk.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
pub(super) const FRAME_BYTES: usize = 16 * 1024;
pub(super) const QUEUED_FRAMES: usize = 64;
const MAX_LINE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputSpool {
    pub path: PathBuf,
    pub stderr: bool,
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn spool_preserves_exact_bytes_and_bounded_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let input = "界".repeat(MAX_LINE_BYTES);
        let mut capture = LineCapture::default();
        let mut saved = Vec::new();
        for chunk in input.as_bytes().chunks(1001) {
            capture
                .feed(chunk, Some(dir.path()), false, &mut |_| Ok(()), &mut saved)
                .unwrap();
            assert!(capture.pending.len() <= MAX_LINE_BYTES);
        }
        capture.finish(false, &mut |_| Ok(()), &mut saved).unwrap();
        assert_eq!(std::fs::read(&saved[0].path).unwrap(), input.as_bytes());
        assert!(capture.spooled);
    }
    #[test]
    fn small_utf8_split_across_frames_is_preserved() {
        let mut capture = LineCapture::default();
        let mut saved = Vec::new();
        let mut lines = Vec::new();
        for byte in "界\n".as_bytes() {
            capture
                .feed(
                    &[*byte],
                    None,
                    false,
                    &mut |line| {
                        lines.push(line.to_owned());
                        Ok(())
                    },
                    &mut saved,
                )
                .unwrap();
        }
        assert_eq!(lines, ["界"]);
        assert!(saved.is_empty());
    }
    #[test]
    fn failed_spooling_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let mut capture = LineCapture::default();
        let mut saved = Vec::new();
        assert!(capture
            .feed(
                &vec![b'x'; MAX_LINE_BYTES + 1],
                Some(&missing),
                false,
                &mut |_| Ok(()),
                &mut saved
            )
            .is_err());
    }
}
#[derive(Default)]
pub(super) struct LineCapture {
    pending: Vec<u8>,
    spool: Option<tempfile::NamedTempFile>,
    bytes: u64,
    pub spooled: bool,
}
impl LineCapture {
    pub fn feed(
        &mut self,
        bytes: &[u8],
        dir: Option<&Path>,
        stderr: bool,
        emit: &mut impl FnMut(&str) -> io::Result<()>,
        saved: &mut Vec<OutputSpool>,
    ) -> io::Result<()> {
        for chunk in bytes.split_inclusive(|byte| *byte == b'\n') {
            self.bytes += chunk.len() as u64;
            if self.spool.is_none()
                && self.pending.len().saturating_add(chunk.len()) > MAX_LINE_BYTES
            {
                let mut file = match dir {
                    Some(dir) => tempfile::Builder::new()
                        .prefix("worker-output-")
                        .tempfile_in(dir)?,
                    None => tempfile::Builder::new()
                        .prefix("davinci-worker-output-")
                        .tempfile()?,
                };
                file.write_all(&self.pending)?;
                self.pending.clear();
                self.spool = Some(file);
            }
            if let Some(file) = self.spool.as_mut() {
                file.write_all(chunk)?;
            } else {
                self.pending.extend_from_slice(chunk);
            }
            if chunk.last() == Some(&b'\n') {
                self.finish(stderr, emit, saved)?;
            }
        }
        Ok(())
    }
    pub fn finish(
        &mut self,
        stderr: bool,
        emit: &mut impl FnMut(&str) -> io::Result<()>,
        saved: &mut Vec<OutputSpool>,
    ) -> io::Result<()> {
        if let Some(mut file) = self.spool.take() {
            file.flush()?;
            let (_, path) = file.keep().map_err(|err| err.error)?;
            emit(&format!(
                "[output saved: {} ({} bytes)]",
                path.display(),
                self.bytes
            ))?;
            saved.push(OutputSpool {
                path,
                stderr,
                bytes: self.bytes,
            });
            self.spooled = true;
            // Every path is delivered to the caller; keep diagnostic metadata
            // bounded too, rather than retaining a second unbounded transcript.
            if saved.len() > 64 {
                saved.remove(0);
            }
        } else if !self.pending.is_empty() {
            emit(String::from_utf8_lossy(&self.pending).trim_end())?;
        }
        self.pending.clear();
        self.bytes = 0;
        Ok(())
    }
}
