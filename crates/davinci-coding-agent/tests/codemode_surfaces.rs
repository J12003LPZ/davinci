use davinci_coding_agent::{
    args::parse_args,
    codemode_host::config::{managed_paths, resolve_config, ReadOnlyConfig},
};
use serde_json::json;
use std::path::Path;

#[test]
fn read_only_cli_is_explicit_and_default_is_off() {
    assert_eq!(parse_args(&[]).codemode, None);
    for mode in ["off", "read-only", "controlled"] {
        let args = parse_args(&["--codemode".into(), mode.into()]);
        assert_eq!(args.codemode.as_deref(), Some(mode));
        assert!(args.diagnostics.is_empty());
    }
    assert!(!parse_args(&["--codemode".into(), "invalid".into()])
        .diagnostics
        .is_empty());
}

#[test]
fn read_only_off_ignores_unused_invalid_paths() {
    for config in [
        json!({"mode":"off","nodePath":23,"hostPath":"relative"}),
        json!(false),
    ] {
        assert_eq!(
            resolve_config(None, Some(&config), agent_dir()).unwrap(),
            None
        );
        assert_eq!(
            resolve_config(Some("off"), Some(&config), agent_dir()).unwrap(),
            None
        );
    }
    assert_eq!(resolve_config(None, None, agent_dir()).unwrap(), None);
}

fn agent_dir() -> &'static Path {
    Path::new(if cfg!(windows) { r"C:\agent" } else { "/agent" })
}

#[test]
fn read_only_defaults_to_the_managed_runtime_and_controlled_is_unavailable() {
    // `/config` only stores the mode, so missing paths mean the managed
    // install under <agent dir>/codemode.
    let managed = managed_paths(agent_dir());
    assert!(managed
        .node_path
        .starts_with(agent_dir().join("codemode").join("node")));
    assert_eq!(managed.host_path, agent_dir().join("codemode").join("host"));
    for user in [None, Some(json!({"mode": "read-only"}))] {
        assert_eq!(
            resolve_config(Some("read-only"), user.as_ref(), agent_dir()).unwrap(),
            Some(managed_paths(agent_dir()))
        );
    }
    // Explicit absolute paths still win; relative or non-string ones fail.
    let custom = if cfg!(windows) {
        r"C:\custom\node.exe"
    } else {
        "/custom/node"
    };
    assert_eq!(
        resolve_config(
            None,
            Some(&json!({"mode":"read-only","nodePath":custom})),
            agent_dir()
        )
        .unwrap(),
        Some(ReadOnlyConfig {
            node_path: custom.into(),
            host_path: managed_paths(agent_dir()).host_path,
        })
    );
    for bad in [json!({"nodePath":"relative"}), json!({"hostPath": 7})] {
        assert!(resolve_config(Some("read-only"), Some(&bad), agent_dir()).is_err());
    }
    assert!(resolve_config(Some("controlled"), None, agent_dir())
        .unwrap_err()
        .contains("acceptance gates"));
}

#[test]
fn read_only_project_configuration_cannot_enable_or_replace_host() {
    use davinci_coding_agent::settings::{load_merged_settings_with_override, settings_path};
    let user = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let config_dir = project.path().join(".davinci");
    std::fs::create_dir(&config_dir).unwrap();
    std::fs::write(
        config_dir.join("settings.json"),
        serde_json::to_vec(&json!({
            "codemode":{"mode":"controlled","nodePath":"project","hostPath":"project"}
        }))
        .unwrap(),
    )
    .unwrap();
    let settings = load_merged_settings_with_override(user.path(), project.path(), Some(true));
    assert!(settings.codemode.is_none());
    let global = json!({"mode":"off","nodePath":"owner","hostPath":"owner"});
    std::fs::write(
        settings_path(user.path()),
        serde_json::to_vec(&json!({"codemode":global})).unwrap(),
    )
    .unwrap();
    let settings = load_merged_settings_with_override(user.path(), project.path(), Some(true));
    assert_eq!(settings.codemode, Some(global));
}

#[test]
fn a_home_workspace_admits_the_managed_node_but_not_a_project_node() {
    // `davinci` started from the home directory: the managed runtime under
    // `~/.davinci/agent/codemode` lies inside the workspace yet is DaVinci's.
    let home = tempfile::tempdir().unwrap();
    let agent = home.path().join(".davinci").join("agent");
    let managed = managed_paths(&agent);
    std::fs::create_dir_all(managed.node_path.parent().unwrap()).unwrap();
    std::fs::write(&managed.node_path, b"not node").unwrap();
    let error = managed.admit(home.path(), &agent).err().unwrap();
    assert!(!error.contains("Workspace-provided"), "{error}");

    let project_node = home.path().join("repo").join("node.exe");
    std::fs::create_dir_all(project_node.parent().unwrap()).unwrap();
    std::fs::write(&project_node, b"not node").unwrap();
    let project = ReadOnlyConfig {
        node_path: project_node,
        host_path: managed.host_path.clone(),
    };
    let error = project.admit(home.path(), &agent).err().unwrap();
    assert!(error.contains("Workspace-provided Node runtime is not admitted"), "{error}");
}
