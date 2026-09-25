//! Stable language, project, invocation and session identities.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageFamily {
    TypeScript,
    Rust,
    Python,
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

#[derive(Debug, Clone)]
pub struct ServerInvocation {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub canonical_program: PathBuf,
    pub executable_fingerprint: String,
}

#[derive(Debug, Clone)]
pub struct RequestBudget {
    pub deadline: Instant,
    pub cancelled: Option<Arc<AtomicBool>>,
}

impl RequestBudget {
    pub fn remaining(&self) -> Option<std::time::Duration> {
        if self
            .cancelled
            .as_ref()
            .is_some_and(|cancelled| cancelled.load(std::sync::atomic::Ordering::Acquire))
        {
            return None;
        }
        self.deadline.checked_duration_since(Instant::now())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_root_different_languages_have_distinct_session_keys() {
        let rust = SessionKey {
            workspace: "/work".into(),
            project_root: "/work".into(),
            family: LanguageFamily::Rust,
            profile_fingerprint: "same".into(),
        };
        let python = SessionKey {
            family: LanguageFamily::Python,
            ..rust.clone()
        };
        assert_ne!(rust, python);
    }
}
