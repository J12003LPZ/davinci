//! Opt-in real BasedPyright/Pyright compatibility verification.
use super::config::{PythonBackend, ServerOverride};
use super::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn provisioned(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| {
        panic!("{name} must name a provisioned Python language-server executable")
    }));
    assert!(path.is_absolute() && path.is_file(), "{path:?}");
    path
}

fn interpreter() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("DAVINCI_TEST_PYTHON")
            .expect("DAVINCI_TEST_PYTHON must name the analysis interpreter"),
    );
    assert!(path.is_absolute() && path.is_file(), "{path:?}");
    path
}

fn trusted(manager: &LanguageIntelligence) {
    let mut policy =
        davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::AlwaysApprove);
    policy.project_trusted = true;
    manager.set_permissions(Some(Arc::new(davinci_agent::PermissionState::new(policy))));
}

fn call(manager: &LanguageIntelligence, name: &str, args: Value) -> Value {
    let result = manager.execute(name, &args).unwrap();
    if result.is_error {
        let details = result.details.unwrap_or(Value::Null);
        assert_eq!(
            details.pointer("/error/code").and_then(Value::as_str),
            Some("unsupported_method"),
            "{name}: {details}"
        );
        return details;
    }
    result.details.unwrap()
}

fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

fn run_python_backend(backend: PythonBackend, server_env: &str) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src/example")).unwrap();
    std::fs::write(
        root.join("pyrightconfig.json"),
        r#"{"include":["src"],"typeCheckingMode":"strict"}"#,
    )
    .unwrap();
    std::fs::write(root.join("src/example/__init__.py"), "").unwrap();
    std::fs::write(
        root.join("src/example/models.py"),
        "class User:\n    name: str\n",
    )
    .unwrap();
    let good = "from .models import User\n\ndef greeting(user: User) -> str:\n    return user.name\n";
    std::fs::write(root.join("src/example/service.py"), good).unwrap();
    let baseline = root.join(".basedpyright");
    std::fs::write(&baseline, "fixture baseline must remain unchanged\n").unwrap();
    let baseline_before = bytes(&baseline);

    let mut config = LanguageIntelligenceConfig::default();
    config.python.backend = backend;
    config.python.interpreter = Some(interpreter());
    config.python.server = Some(ServerOverride {
        program: provisioned(server_env),
        args: vec!["--stdio".into()],
    });
    config.python.request_timeout_ms = 30_000;
    config.python.initialization_timeout_ms = 60_000;
    config.python.cold_request_timeout_ms = 120_000;

    let manager = LanguageIntelligence::new(root, config);
    trusted(&manager);

    let definition = call(
        &manager,
        "lsp_definition",
        json!({"path":"src/example/service.py","line":1,"column":22}),
    );
    if definition["available"] == true {
        assert!(definition["items"].as_array().unwrap().iter().any(|item| {
            item["path"] == "src/example/models.py"
        }), "{definition}");
    }
    let references = call(
        &manager,
        "lsp_references",
        json!({"path":"src/example/service.py","line":3,"column":21,"includeDeclaration":true}),
    );
    if references["available"] == true {
        assert!(references["total"].as_u64().unwrap_or(0) >= 1);
    }
    let hover = call(
        &manager,
        "lsp_hover",
        json!({"path":"src/example/service.py","line":3,"column":21}),
    );
    if hover["available"] == true {
        assert!(hover["text"].as_str().unwrap_or_default().contains("User"));
    }
    let symbols = call(
        &manager,
        "lsp_document_symbols",
        json!({"path":"src/example/service.py"}),
    );
    if symbols["available"] == true {
        assert!(symbols["items"].as_array().unwrap().iter().any(|item| item["name"] == "greeting"));
    }
    let workspace = call(
        &manager,
        "lsp_workspace_symbols",
        json!({"path":"src/example/service.py","query":"greeting"}),
    );
    if workspace["available"] == true {
        assert!(workspace["total"].as_u64().unwrap_or(0) >= 1);
    }
    let _ = call(
        &manager,
        "lsp_implementations",
        json!({"path":"src/example/service.py","line":3,"column":21}),
    );
    let _ = call(
        &manager,
        "lsp_type_definition",
        json!({"path":"src/example/service.py","line":3,"column":21}),
    );

    std::fs::write(
        root.join("src/example/service.py"),
        "from .models import User\n\ndef greeting(user: User) -> str:\n    return 42\n",
    )
    .unwrap();
    let diagnostics = call(
        &manager,
        "lsp_diagnostics",
        json!({"path":"src/example/service.py","severity":"error"}),
    );
    if diagnostics["available"] == true {
        assert_eq!(diagnostics["advisory"], true);
        assert!(diagnostics["total"].as_u64().unwrap_or(0) >= 1, "{diagnostics}");
    }
    std::fs::write(root.join("src/example/service.py"), good).unwrap();
    let fixed = call(
        &manager,
        "lsp_diagnostics",
        json!({"path":"src/example/service.py","severity":"error"}),
    );
    if fixed["available"] == true {
        assert_eq!(fixed["advisory"], true);
    }

    if backend == PythonBackend::Basedpyright {
        assert_eq!(bytes(&baseline), baseline_before);
    }
    manager.shutdown();
}

#[test]
#[ignore = "requires provisioned DAVINCI_TEST_BASEDPYRIGHT and DAVINCI_TEST_PYTHON"]
fn real_basedpyright_lsp_semantics() {
    run_python_backend(PythonBackend::Basedpyright, "DAVINCI_TEST_BASEDPYRIGHT");
}

#[test]
#[ignore = "requires provisioned DAVINCI_TEST_PYRIGHT and DAVINCI_TEST_PYTHON"]
fn real_pyright_lsp_semantics() {
    run_python_backend(PythonBackend::Pyright, "DAVINCI_TEST_PYRIGHT");
}
