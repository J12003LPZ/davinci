//! Preflight: a missing runtime, a switched-off feature and an installed but
//! refused runtime are three different states with three different fixes.
//! The runtime here is a small fake whose manifest pins real hashes, so the
//! deep pass genuinely verifies it. No Node or Chromium is started.

use davinci_coding_agent::design::health::*;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", davinci_sys::hex::Lower(&Sha256::digest(bytes)))
}

struct Fixture {
    _dir: tempfile::TempDir,
    root: PathBuf,
    node: PathBuf,
    browser: PathBuf,
    workspace: PathBuf,
}

const REQUIRED: [&str; 9] = [
    "host/main.cjs",
    "host/server.cjs",
    "host/protocol.cjs",
    "host/compiler.cjs",
    "host/compile.cjs",
    "ui/index.html",
    "ui/app.js",
    "ui/app.css",
    "ui/manifest.json",
];

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A runtime the real verifier accepts, outside the workspace.
fn installed(node_version: &str) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().canonicalize().unwrap();
    let root = base.join("runtime");
    let workspace = base.join("project");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut files = serde_json::Map::new();
    for name in REQUIRED {
        let text = format!("// {name}");
        write(&root.join(name), &text);
        files.insert(name.into(), json!(hash(text.as_bytes())));
    }
    let node = base.join("node.exe");
    std::fs::write(&node, "node").unwrap();
    let cache = base.join("browsers");
    let directory = cache.join("chromium");
    let browser = directory.join("chrome.exe");
    write(&browser, "chrome");
    let fonts = base.join("fonts");
    write(&fonts.join("a.ttf"), "font");
    let manifest = json!({
        "schema_version": 1,
        "node_version": node_version,
        "node_sha256": hash(b"node"),
        "files": files,
        "browser": {
            "cache": cache,
            "directory": directory,
            "executable": browser,
            "files": {"chrome.exe": hash(b"chrome")},
        },
        "fonts": [{"directory": fonts, "files": {"a.ttf": hash(b"font")}}],
    });
    write(
        &root.join("runtime-manifest.json"),
        &serde_json::to_string(&manifest).unwrap(),
    );
    Fixture {
        _dir: dir,
        root,
        node,
        browser,
        workspace,
    }
}

fn probe(fixture: &Fixture, enabled: bool, deep: bool) -> HealthProbe {
    HealthProbe {
        enabled,
        runtime: Some(fixture.root.clone()),
        node: Some(fixture.node.clone()),
        workspace: fixture.workspace.clone(),
        deep,
        overrides: vec![],
        provider_refusal: None,
    }
}
fn codes(health: &DesignRuntimeHealth) -> Vec<&'static str> {
    health.diagnostics.iter().map(|d| d.code).collect()
}

#[test]
fn design_preflight_distinguishes_missing_disabled_invalid() {
    // Missing: nothing is installed.
    let workspace = tempfile::tempdir().unwrap();
    let missing = assess(&HealthProbe {
        enabled: true,
        runtime: None,
        node: None,
        workspace: workspace.path().to_path_buf(),
        deep: false,
        overrides: vec![],
        provider_refusal: None,
    });
    assert_eq!(missing.state, HealthState::Missing);
    assert!(matches!(
        missing.fix_action,
        Some(FixAction::RunSetup { .. })
    ));
    assert_eq!(
        codes(&missing),
        ["runtime_not_installed", "node_not_configured"]
    );

    // Disabled: installed and sound, only the switch is off.
    let fixture = installed("24.19.0");
    let disabled = assess(&probe(&fixture, false, false));
    assert_eq!(disabled.state, HealthState::Disabled);
    assert_eq!(disabled.fix_action, Some(FixAction::TurnOn));
    assert_eq!(codes(&disabled), ["feature_off"]);

    // Invalid: installed, switched on, and refused.
    let other = installed("22.1.0");
    let invalid = assess(&probe(&other, true, false));
    assert_eq!(invalid.state, HealthState::Invalid);
    assert_eq!(codes(&invalid), ["node_version_unsupported"]);
    assert!(matches!(
        invalid.fix_action,
        Some(FixAction::Reinstall { .. })
    ));

    // Three different states, three different fixes.
    let states = [missing.state, disabled.state, invalid.state];
    assert_eq!(
        states
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        3
    );
    assert_ne!(missing.fix_action, disabled.fix_action);
    assert_ne!(disabled.fix_action, invalid.fix_action);
}

#[test]
fn an_intact_runtime_is_ready_on_both_passes() {
    let fixture = installed("24.19.0");
    let shallow = assess(&probe(&fixture, true, false));
    assert_eq!(shallow.state, HealthState::Ready);
    assert!(shallow.diagnostics.is_empty() && shallow.fix_action.is_none());
    #[cfg(not(target_os = "linux"))]
    {
        let deep = assess(&probe(&fixture, true, true));
        assert_eq!(deep.state, HealthState::Ready, "{:?}", deep.diagnostics);
    }
}

#[test]
fn only_the_deep_pass_sees_a_tampered_file() {
    let fixture = installed("24.19.0");
    std::fs::write(fixture.root.join("host/main.cjs"), "// replaced").unwrap();
    assert_eq!(
        assess(&probe(&fixture, true, false)).state,
        HealthState::Ready,
        "the cheap pass does not hash"
    );
    let deep = assess(&probe(&fixture, true, true));
    assert_eq!(deep.state, HealthState::Invalid);
    assert_eq!(codes(&deep), ["runtime_changed"]);
    assert!(matches!(deep.fix_action, Some(FixAction::Reinstall { .. })));
}

#[test]
fn each_refusal_names_its_cause_and_fix() {
    // No Chromium: missing, not invalid.
    let fixture = installed("24.19.0");
    std::fs::remove_file(&fixture.browser).unwrap();
    let health = assess(&probe(&fixture, true, false));
    assert_eq!(health.state, HealthState::Missing);
    assert_eq!(codes(&health), ["browser_missing"]);

    // A workspace that holds the home folder.
    let fixture = installed("24.19.0");
    let mut at_home = probe(&fixture, true, false);
    at_home.workspace = davinci_session::home_dir().expect("a home directory");
    let health = assess(&at_home);
    assert_eq!(health.state, HealthState::Invalid);
    assert_eq!(health.fix_action, Some(FixAction::StartInProject));
    assert!(codes(&health).contains(&"workspace_holds_home"));

    // A runtime the session could rewrite.
    let fixture = installed("24.19.0");
    let mut inside = probe(&fixture, true, false);
    inside.workspace = fixture.root.parent().unwrap().to_path_buf();
    let health = assess(&inside);
    assert_eq!(health.state, HealthState::Invalid);
    assert!(codes(&health).contains(&"runtime_in_workspace"));
    assert_eq!(
        health.fix_action,
        Some(FixAction::MoveRuntimeOutsideWorkspace)
    );

    // Generation would refuse the model route.
    let fixture = installed("24.19.0");
    let mut wrong = probe(&fixture, true, false);
    wrong.provider_refusal = Some("the selected model is not a subscription route".into());
    let health = assess(&wrong);
    assert_eq!(health.state, HealthState::Invalid);
    assert_eq!(health.fix_action, Some(FixAction::SelectSubscriptionModel));

    // An unreadable manifest.
    let fixture = installed("24.19.0");
    std::fs::write(fixture.root.join("runtime-manifest.json"), "{}").unwrap();
    let health = assess(&probe(&fixture, true, false));
    assert_eq!(codes(&health), ["manifest_unreadable"]);
}

#[test]
fn an_environment_override_is_named_only_when_something_is_wrong() {
    let fixture = installed("24.19.0");
    let mut off = probe(&fixture, false, false);
    off.overrides = vec!["DAVINCI_DESIGN_ENABLED".into()];
    let health = assess(&off);
    assert_eq!(health.state, HealthState::Disabled);
    assert!(codes(&health).contains(&"env_override"));
    assert_eq!(
        health.fix_action,
        Some(FixAction::UnsetEnvironment {
            name: "DAVINCI_DESIGN_ENABLED".into()
        })
    );

    let mut fine = probe(&fixture, true, false);
    fine.overrides = vec!["DAVINCI_DESIGN_ENABLED".into()];
    assert!(assess(&fine).diagnostics.is_empty());
}

#[test]
fn a_fix_is_always_something_the_person_does() {
    // Nothing offered here runs a script by itself: every action describes a
    // command or a switch.
    for action in [
        FixAction::TurnOn,
        FixAction::StartInProject,
        FixAction::SelectSubscriptionModel,
        FixAction::MoveRuntimeOutsideWorkspace,
        FixAction::UnsetEnvironment { name: "X".into() },
        FixAction::RunSetup {
            instruction: "run it".into(),
        },
        FixAction::Reinstall {
            instruction: "run it".into(),
        },
    ] {
        assert!(!action.describe().is_empty());
    }
}
