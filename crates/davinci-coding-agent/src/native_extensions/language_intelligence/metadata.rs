//! Bounded metadata access used by project and executable discovery.
//!
//! This seam deliberately exposes no subprocess API. Production callers can
//! narrow installation roots separately from workspace configuration reads.

use super::identity::RequestBudget;
use super::protocol::{IntelligenceError, Result};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MetadataClass {
    WorkspaceConfiguration,
    InstalledExecutable,
    AnalysisEnvironment,
}

pub(super) trait MetadataReader: Send + Sync + std::fmt::Debug {
    fn resolve_path(
        &self,
        path: &Path,
        class: MetadataClass,
        budget: &RequestBudget,
    ) -> Result<PathBuf>;
    fn read(
        &self,
        path: &Path,
        class: MetadataClass,
        max_bytes: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<u8>>;
    fn list_directory(
        &self,
        path: &Path,
        class: MetadataClass,
        max_entries: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<PathBuf>>;
}

#[derive(Debug, Clone)]
pub(super) struct ResolutionContext {
    pub workspace: PathBuf,
    pub reader: Arc<dyn MetadataReader>,
    pub budget: RequestBudget,
}

#[derive(Debug)]
pub(super) struct WorkspaceMetadataReader {
    workspace: PathBuf,
    installation_roots: Vec<PathBuf>,
}

impl WorkspaceMetadataReader {
    pub(super) fn new(workspace: PathBuf, installation_roots: Vec<PathBuf>) -> Self {
        Self {
            workspace,
            installation_roots,
        }
    }

    fn check_budget(budget: &RequestBudget) -> Result<()> {
        if budget.remaining().is_none() {
            return Err(IntelligenceError::new(
                "request_timeout",
                "Language metadata resolution exceeded its request budget",
            ));
        }
        Ok(())
    }

    fn allowed(&self, path: &Path, class: MetadataClass) -> bool {
        match class {
            MetadataClass::WorkspaceConfiguration => path.starts_with(&self.workspace),
            MetadataClass::InstalledExecutable | MetadataClass::AnalysisEnvironment => {
                path.starts_with(&self.workspace)
                    || self
                        .installation_roots
                        .iter()
                        .any(|root| path.starts_with(root))
            }
        }
    }

    fn canonical_allowed(&self, path: &Path, class: MetadataClass) -> Result<PathBuf> {
        let canonical = path.canonicalize().map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Metadata path is unavailable")
        })?;
        if !self.allowed(&canonical, class) {
            return Err(IntelligenceError::new(
                "outside_workspace",
                "Metadata path escapes its authorized roots",
            ));
        }
        Ok(canonical)
    }
}

impl MetadataReader for WorkspaceMetadataReader {
    fn resolve_path(
        &self,
        path: &Path,
        class: MetadataClass,
        budget: &RequestBudget,
    ) -> Result<PathBuf> {
        Self::check_budget(budget)?;
        self.canonical_allowed(path, class)
    }

    fn read(
        &self,
        path: &Path,
        class: MetadataClass,
        max_bytes: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<u8>> {
        Self::check_budget(budget)?;
        let canonical = self.canonical_allowed(path, class)?;
        let metadata = std::fs::metadata(&canonical).map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Metadata path is unavailable")
        })?;
        if !metadata.is_file() || metadata.len() > max_bytes as u64 {
            return Err(IntelligenceError::new(
                "metadata_too_large",
                "Metadata input exceeds its bounded size",
            ));
        }
        let bytes = std::fs::read(canonical).map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Metadata could not be read")
        })?;
        if bytes.len() > max_bytes {
            return Err(IntelligenceError::new(
                "metadata_too_large",
                "Metadata input exceeds its bounded size",
            ));
        }
        Ok(bytes)
    }

    fn list_directory(
        &self,
        path: &Path,
        class: MetadataClass,
        max_entries: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<PathBuf>> {
        Self::check_budget(budget)?;
        let canonical = self.canonical_allowed(path, class)?;
        let mut result = Vec::new();
        let entries = std::fs::read_dir(canonical).map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Metadata directory is unavailable")
        })?;
        for entry in entries {
            Self::check_budget(budget)?;
            if result.len() >= max_entries {
                return Err(IntelligenceError::new(
                    "project_resolution_incomplete",
                    "Metadata directory exceeds the bounded entry limit",
                ));
            }
            let path = entry
                .map_err(|_| {
                    IntelligenceError::new(
                        "metadata_unavailable",
                        "Metadata directory entry is unavailable",
                    )
                })?
                .path();
            result.push(path);
        }
        Ok(result)
    }
}
