use super::{events::event_from_message, ContextVmRuntime};
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};

#[derive(Debug, Clone)]
pub(crate) struct SessionSource {
    pub path: PathBuf,
    pub id: String,
}

impl ContextVmRuntime {
    pub fn bind_session_source(&self, path: PathBuf, id: String) {
        *self
            .session_source
            .write()
            .unwrap_or_else(|e| e.into_inner()) = Some(SessionSource { path, id });
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
            let file = File::open(&binding.path).map_err(|_| "context session unavailable")?;
            let mut lines = BufReader::new(file).lines();
            let header = lines
                .next()
                .ok_or("context session header missing")?
                .map_err(|_| "context session read failed")?;
            let header = davinci_session::parse_header(&header)
                .map_err(|_| "invalid context session header")?;
            if header.id != binding.id {
                return Err("context session identity changed".into());
            }
            for line in lines {
                let line = line.map_err(|_| "context session read failed")?;
                let Ok(davinci_session::SessionMutation::Entry { entry, .. }) =
                    davinci_session::parse_mutation(&line)
                else {
                    continue;
                };
                if entry.id != id {
                    continue;
                }
                let message =
                    serde_json::from_value(entry.message.ok_or("context source has no message")?)
                        .map_err(|_| "invalid context source message")?;
                let event = event_from_message(&message, source_ref.into(), entry.seq)
                    .ok_or("context source has no visible content")?;
                if event.content_hash != expected {
                    return Err("context source integrity check failed".into());
                }
                return Ok(event.visible_text);
            }
            return Err("context source unavailable; replay required".into());
        }
        self.source_contents
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(source_ref)
            .cloned()
            .ok_or_else(|| "unknown context source_ref".into())
    }
}
