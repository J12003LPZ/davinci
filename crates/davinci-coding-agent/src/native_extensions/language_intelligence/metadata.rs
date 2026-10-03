//! Bounded metadata access for project and installed-tool discovery.
use super::protocol::{IntelligenceError, RequestBudget, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
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
    #[allow(dead_code)]
    fn list_directory(
        &self,
        path: &Path,
        class: MetadataClass,
        max_entries: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<PathBuf>>;
}

#[derive(Debug)]
pub(super) struct ResolutionContext {
    pub workspace: PathBuf,
    pub reader: std::sync::Arc<dyn MetadataReader>,
    pub budget: RequestBudget,
}

#[derive(Debug, Clone)]
pub(super) struct FsMetadataReader {
    workspace: PathBuf,
    installation_roots: Vec<PathBuf>,
}

impl FsMetadataReader {
    pub fn new(workspace: PathBuf, installation_roots: Vec<PathBuf>) -> Self {
        Self {
            workspace,
            installation_roots,
        }
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
}

impl MetadataReader for FsMetadataReader {
    fn resolve_path(
        &self,
        path: &Path,
        class: MetadataClass,
        budget: &RequestBudget,
    ) -> Result<PathBuf> {
        budget.check()?;
        let canonical = path.canonicalize().map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Metadata path is unavailable")
        })?;
        if !self.allowed(&canonical, class) {
            return Err(IntelligenceError::new(
                "permission_denied",
                "Metadata path is outside the authorized discovery scope",
            ));
        }
        Ok(canonical)
    }

    fn read(
        &self,
        path: &Path,
        class: MetadataClass,
        max_bytes: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<u8>> {
        let path = self.resolve_path(path, class, budget)?;
        let metadata = std::fs::metadata(&path).map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Metadata file is unavailable")
        })?;
        if !metadata.is_file() || metadata.len() > max_bytes as u64 {
            return Err(IntelligenceError::new(
                "project_resolution_incomplete",
                "Metadata file exceeds the bounded discovery policy",
            ));
        }
        let bytes = std::fs::read(path).map_err(|_| {
            IntelligenceError::new("metadata_unavailable", "Could not read project metadata")
        })?;
        budget.check()?;
        Ok(bytes)
    }

    fn list_directory(
        &self,
        path: &Path,
        class: MetadataClass,
        max_entries: usize,
        budget: &RequestBudget,
    ) -> Result<Vec<PathBuf>> {
        let path = self.resolve_path(path, class, budget)?;
        let mut out = Vec::new();
        let entries = std::fs::read_dir(path).map_err(|_| {
            IntelligenceError::new(
                "metadata_unavailable",
                "Could not list project metadata directory",
            )
        })?;
        for entry in entries {
            budget.check()?;
            if out.len() >= max_entries {
                return Err(IntelligenceError::new(
                    "project_resolution_incomplete",
                    "Directory expansion exceeded its bounded discovery limit",
                ));
            }
            let entry = entry.map_err(|_| {
                IntelligenceError::new(
                    "metadata_unavailable",
                    "Could not inspect project metadata entry",
                )
            })?;
            out.push(entry.path());
        }
        Ok(out)
    }
}
