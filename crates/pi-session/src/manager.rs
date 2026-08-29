use crate::types::*;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct SessionManager {
    pub session_dir: PathBuf,
}

impl SessionManager {
    pub fn new(session_dir: impl AsRef<Path>) -> Self {
        let dir = session_dir.as_ref().to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        Self { session_dir: dir }
    }

    pub fn session_path(&self, session_id: &str) -> PathBuf {
        self.session_dir.join(format!("{}.jsonl", session_id))
    }

    pub fn create_session(
        &self,
        cwd: &str,
        parent_session: Option<&str>,
    ) -> std::io::Result<(String, PathBuf)> {
        let id = uuid::Uuid::new_v4().to_string();
        let path = self.session_path(&id);
        let header = SessionHeader {
            entry_type: "session".to_string(),
            version: CURRENT_SESSION_VERSION,
            id: id.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            cwd: cwd.to_string(),
            parent_session: parent_session.map(String::from),
        };
        let mut file = File::create(&path)?;
        let line = serde_json::to_string(&header)?;
        writeln!(file, "{}", line)?;
        Ok((id, path))
    }

    pub fn append_entry(&self, session_id: &str, entry: &SessionEntry) -> std::io::Result<()> {
        let path = self.session_path(session_id);
        let mut file = OpenOptions::new().append(true).open(&path)?;
        let line = serde_json::to_string(entry)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    pub fn read_entries(&self, session_id: &str) -> std::io::Result<Vec<SessionEntry>> {
        let path = self.session_path(session_id);
        let file = File::open(&path)?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for (i, line_res) in reader.lines().enumerate() {
            let line = line_res?;
            if i == 0 {
                // Header line
                continue;
            }
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<SessionEntry>(&line) {
                entries.push(entry);
            }
        }
        Ok(entries)
    }

    pub fn list_sessions(&self) -> std::io::Result<Vec<SessionInfo>> {
        let mut sessions = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.session_dir) {
            for entry_res in entries {
                let entry = entry_res?;
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                    if let Ok(file) = File::open(&path) {
                        let mut reader = BufReader::new(file);
                        let mut first_line = String::new();
                        if reader.read_line(&mut first_line).is_ok() {
                            if let Ok(header) = serde_json::from_str::<SessionHeader>(&first_line) {
                                let message_count = reader.lines().count();
                                sessions.push(SessionInfo {
                                    id: header.id,
                                    path: path.to_string_lossy().to_string(),
                                    timestamp: header.timestamp,
                                    cwd: header.cwd,
                                    message_count,
                                });
                            }
                        }
                    }
                }
            }
        }
        Ok(sessions)
    }
}
