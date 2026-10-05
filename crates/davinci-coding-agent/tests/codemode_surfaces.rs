use davinci_coding_agent::{args::parse_args, codemode_host::config::resolve_config};
use serde_json::json;

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
        assert_eq!(resolve_config(None, Some(&config)).unwrap(), None);
        assert_eq!(resolve_config(Some("off"), Some(&config)).unwrap(), None);
    }
    assert_eq!(resolve_config(None, None).unwrap(), None);
}

#[test]
fn read_only_requires_host_paths_and_controlled_is_explicitly_unavailable() {
    assert!(resolve_config(Some("read-only"), None)
        .unwrap_err()
        .contains("nodePath"));
    assert!(resolve_config(Some("controlled"), None)
        .unwrap_err()
        .contains("acceptance gates"));
    assert!(resolve_config(
        Some("read-only"),
        Some(&json!({"nodePath":"relative","hostPath":"relative"}))
    )
    .is_err());
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
