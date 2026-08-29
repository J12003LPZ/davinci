use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextFile {
    pub path: PathBuf,
    pub name: String,
    pub body: String,
}

pub fn load_context_files(cwd: &Path, enabled: bool) -> Vec<ContextFile> {
    if !enabled {
        return Vec::new();
    }
    let mut files = Vec::new();
    for name in ["AGENTS.md", "CLAUDE.md"] {
        let path = cwd.join(name);
        if let Ok(body) = fs::read_to_string(&path) {
            files.push(ContextFile {
                path,
                name: name.to_string(),
                body,
            });
        }
    }
    files
}
