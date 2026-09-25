//! Stable language, project, process and session identities.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LanguageFamily {
    #[serde(rename = "typescript")]
    TypeScript,
    #[serde(rename = "rust")]
    Rust,
    #[serde(rename = "python")]
    Python,
}

impl LanguageFamily {
    pub fn from_path(path: &Path) -> Option<Self> {
        match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
            "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs" => Some(Self::TypeScript),
            "rs" => Some(Self::Rust),
            "py" | "pyi" => Some(Self::Python),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProject {
    pub workspace: PathBuf,
    pub root: PathBuf,
    pub family: LanguageFamily,
    pub analysis_environment: Option<PathBuf>,
    pub config_files: Vec<PathBuf>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionKey {
    pub workspace: PathBuf,
    pub project_root: PathBuf,
    pub family: LanguageFamily,
    pub profile_fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub canonical_program: PathBuf,
    pub executable_fingerprint: String,
}

impl ServerInvocation {
    pub fn new(program: PathBuf, args: Vec<String>) -> std::io::Result<Self> {
        let canonical_program = program.canonicalize()?;
        let metadata = std::fs::metadata(&canonical_program)?;
        let mut hasher = Sha256::new();
        hasher.update(canonical_program.to_string_lossy().as_bytes());
        hasher.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified().and_then(|v| {
            v.duration_since(std::time::UNIX_EPOCH)
                .map_err(std::io::Error::other)
        }) {
            hasher.update(modified.as_nanos().to_le_bytes());
        }
        for arg in &args {
            hasher.update([0]);
            hasher.update(arg.as_bytes());
        }
        Ok(Self {
            program,
            args,
            canonical_program,
            executable_fingerprint: format!("{:x}", hasher.finalize()),
        })
    }
}
