//! Bounded line assembly and owned disk retention for oversized output.
use std::io::{self, Write};
use std::path::{Path, PathBuf};
pub(super) const FRAME_BYTES: usize = 16 * 1024;
pub(super) const QUEUED_FRAMES: usize = 64;
const MAX_LINE_BYTES: usize = 256 * 1024;
const MAX_SPOOL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RETAINED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_RETAINED_FILES: usize = 64;

/// Until the supervisor returns its outcome, every persisted spool is owned.
/// Any error drops this guard and removes already-kept files too.
#[derive(Default)]
pub(super) struct SpoolFiles {
    pub files: Vec<OutputSpool>,
}
impl SpoolFiles {
    pub fn commit(mut self) -> Vec<OutputSpool> {
        std::mem::take(&mut self.files)
    }
}
impl Drop for SpoolFiles {
    fn drop(&mut self) {
        for spool in &self.files {
            let _ = std::fs::remove_file(&spool.path);
        }
    }
}

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
    fn pr_review_spool_byte_budget_is_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let mut capture = LineCapture::default();
        let mut saved = SpoolFiles::default();
        let mut line = vec![b'x'; MAX_SPOOL_BYTES as usize - 1];
        line.push(b'\n');
        for _ in 0..3 {
            capture
                .feed(
                    &line,
                    Some(dir.path()),
                    false,
                    &mut |_| Ok(()),
                    &mut saved.files,
                )
                .unwrap();
        }
        assert!(saved.files.iter().map(|spool| spool.bytes).sum::<u64>() <= MAX_RETAINED_BYTES);
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            saved.files.len()
        );
        drop(saved); // A supervisor error before commit must clean kept files.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn pr_review_one_hundred_large_lines_have_no_orphan_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut capture = LineCapture::default();
        let mut saved = Vec::new();
        let mut line = vec![b'x'; MAX_LINE_BYTES + 1];
        line.push(b'\n');
        for _ in 0..100 {
            capture
                .feed(&line, Some(dir.path()), false, &mut |_| Ok(()), &mut saved)
                .unwrap();
        }
        let paths: std::collections::BTreeSet<_> =
            saved.iter().map(|spool| spool.path.clone()).collect();
        let files: std::collections::BTreeSet<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(files.len(), paths.len(), "orphaned spool files");
        assert!(files.is_subset(&paths));
        assert!(files.len() <= 64);
    }

    #[test]
    fn pr_review_failed_delivery_does_not_keep_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let mut capture = LineCapture::default();
        let mut saved = Vec::new();
        capture
            .feed(
                &vec![b'x'; MAX_LINE_BYTES + 1],
                Some(dir.path()),
                false,
                &mut |_| Ok(()),
                &mut saved,
            )
            .unwrap();
        assert!(capture
            .finish(
                false,
                &mut |_| Err(io::Error::other("fixture callback failure")),
                &mut saved
            )
            .is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert!(saved.is_empty());
    }

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
            self.bytes = self.bytes.saturating_add(chunk.len() as u64);
            if self.bytes > MAX_SPOOL_BYTES {
                // Drop the in-flight tempfile even when the caller keeps capture.
                self.spool.take();
                self.pending.clear();
                self.bytes = 0;
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("worker output line exceeds spool limit ({MAX_SPOOL_BYTES} bytes)"),
                ));
            }
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
            // Callback failures must not orphan a kept file. Keep ownership
            // through delivery; keep() preserves this exact tempfile path.
            emit(&format!(
                "[output saved: {} ({} bytes)]",
                file.path().display(),
                self.bytes
            ))?;
            let (_, path) = file.keep().map_err(|err| err.error)?;
            saved.push(OutputSpool {
                path,
                stderr,
                bytes: self.bytes,
            });
            self.spooled = true;
            let mut bytes: u64 = saved.iter().map(|spool| spool.bytes).sum();
            while saved.len() > MAX_RETAINED_FILES || bytes > MAX_RETAINED_BYTES {
                let oldest = &saved[0];
                match std::fs::remove_file(&oldest.path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
                bytes -= saved.remove(0).bytes;
            }
        } else if !self.pending.is_empty() {
            emit(String::from_utf8_lossy(&self.pending).trim_end())?;
        }
        self.pending.clear();
        self.bytes = 0;
        Ok(())
    }
}
