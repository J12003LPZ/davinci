//! Deterministic language-server fixture helpers.
#![allow(dead_code)]

use super::config::{LanguageIntelligenceConfig, ServerOverride};
use super::manager::LanguageIntelligence;
use super::metadata::{FsMetadataReader, ResolutionContext};
use super::protocol::RequestBudget;
use davinci_agent::{PermissionMode, PermissionPolicy, PermissionState};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub(super) struct TestWorkspace {
    dir: tempfile::TempDir,
    pub manager: LanguageIntelligence,
    events_path: PathBuf,
}

impl TestWorkspace {
    pub fn new(language: &str, mode: &str) -> Self {
        let dir = tempfile::tempdir().expect("fixture tempdir");
        let root = dir.path().canonicalize().expect("fixture canonical root");
        let events_path = root.join("lsp-events.jsonl");
        let node = std::env::var_os("DAVINCI_TEST_NODE")
            .map(PathBuf::from)
            .unwrap_or_else(|| davinci_sys::process::resolve_program("node"));
        let node = node.canonicalize().unwrap_or(node);
        assert!(
            node.is_absolute() && node.is_file(),
            "DAVINCI_TEST_NODE/node must resolve to an existing absolute executable"
        );
        let script = PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/language-server.cjs"
        ))
        .canonicalize()
        .expect("fixture server");
        let server = ServerOverride {
            program: node,
            args: vec![
                script.to_string_lossy().into_owned(),
                mode.into(),
                events_path.to_string_lossy().into_owned(),
            ],
        };
        let mut config = LanguageIntelligenceConfig::default();
        match language {
            "typescript" | "javascript" => {
                config.rust.enabled = false;
                config.python.enabled = false;
                config.typescript.server = Some(server);
                std::fs::write(root.join("package.json"), r#"{"private":true}"#).unwrap();
                std::fs::write(root.join("a.ts"), "hello").unwrap();
            }
            "rust" => {
                config.typescript.enabled = false;
                config.python.enabled = false;
                config.rust.server = Some(server);
                std::fs::create_dir_all(root.join("src")).unwrap();
                std::fs::write(
                    root.join("Cargo.toml"),
                    "[package]\nname=\"fixture\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
                )
                .unwrap();
                std::fs::write(root.join("src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
            }
            "python" => {
                config.typescript.enabled = false;
                config.rust.enabled = false;
                config.python.server = Some(server);
                std::fs::write(root.join("pyproject.toml"), "[tool.pyright]\n").unwrap();
                std::fs::write(root.join("app.py"), "value: int = 1\n").unwrap();
            }
            other => panic!("unsupported fixture language: {other}"),
        }
        let manager = LanguageIntelligence::new(&root, config);
        let mut policy = PermissionPolicy::new(PermissionMode::AlwaysApprove);
        policy.project_trusted = true;
        manager.set_permissions(Some(Arc::new(PermissionState::new(policy))));
        Self {
            dir,
            manager,
            events_path,
        }
    }

    pub fn root(&self) -> &Path {
        self.dir.path()
    }

    pub fn write(&self, relative: &str, text: &str) {
        let relative = Path::new(relative);
        assert!(!relative.is_absolute());
        assert!(relative.components().all(|component| matches!(
            component,
            std::path::Component::Normal(_) | std::path::Component::CurDir
        )));
        let path = self.root().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    pub fn call(&self, name: &str, args: Value) -> Value {
        self.manager
            .execute(name, &args)
            .expect("fixture dispatch")
            .details
            .expect("structured native result")
    }

    pub fn events(&self) -> Vec<Value> {
        let raw = std::fs::read_to_string(&self.events_path).unwrap_or_default();
        raw.lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    pub fn resolution_context(&self) -> ResolutionContext {
        let root = self.root().canonicalize().unwrap();
        ResolutionContext {
            workspace: root.clone(),
            reader: Arc::new(FsMetadataReader::new(root, Vec::new())),
            budget: RequestBudget::from_timeout(Duration::from_secs(3)),
        }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        self.manager.shutdown();
    }
}
