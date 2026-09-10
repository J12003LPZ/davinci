pub mod documents;
pub mod manager;
pub mod rename;
pub mod tools;
pub mod transport;

use davinci_agent::semantic::{
    RenamePreview, SemanticCapabilities, SemanticResult, SemanticService,
};
use std::path::Path;

/// Native implementation of SemanticService connecting to LSP server manager.
#[derive(Debug, Default)]
pub struct NativeSemanticService {
    // In subsequent tasks, this coordinates with Transport, Manager, and Documents.
}

impl NativeSemanticService {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SemanticService for NativeSemanticService {
    fn is_server_available(&self, _language: &str, _path: &Path) -> bool {
        false
    }

    fn capabilities(&self, _language: &str, _path: &Path) -> SemanticCapabilities {
        SemanticCapabilities::default()
    }

    fn definition(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
    ) -> Result<SemanticResult, String> {
        Err("No language server available".into())
    }

    fn references(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _include_declaration: bool,
    ) -> Result<SemanticResult, String> {
        Err("No language server available".into())
    }

    fn outline(&self, _cwd: &Path, _file_path: &str) -> Result<SemanticResult, String> {
        Err("No language server available".into())
    }

    fn diagnostics(&self, _cwd: &Path, _file_path: &str) -> Result<SemanticResult, String> {
        Err("Diagnostics are unavailable: no active language server for this file type".into())
    }

    fn call_hierarchy(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _incoming: bool,
    ) -> Result<SemanticResult, String> {
        Err("Call hierarchy is unsupported without an active language server".into())
    }

    fn rename_preview(
        &self,
        _cwd: &Path,
        _file_path: &str,
        _line: u32,
        _character: u32,
        _new_name: &str,
    ) -> Result<RenamePreview, String> {
        Err("Rename preview is unavailable without an active language server; plain text search cannot safely guarantee semantic rename".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_native_semantic_service_defaults_to_unavailable() {
        let svc = NativeSemanticService::new();
        assert!(!svc.is_server_available("rust", Path::new("src/main.rs")));
        assert_eq!(
            svc.capabilities("rust", Path::new("src/main.rs")),
            SemanticCapabilities::default()
        );
        assert!(svc.definition(Path::new("."), "src/main.rs", 1, 1).is_err());
        assert!(svc.diagnostics(Path::new("."), "src/main.rs").is_err());
        assert!(svc
            .call_hierarchy(Path::new("."), "src/main.rs", 1, 1, true)
            .is_err());
        assert!(svc
            .rename_preview(Path::new("."), "src/main.rs", 1, 1, "foo")
            .is_err());
    }
}
