use std::fs;

use davinci_ai::AuthStorage;
use davinci_coding_agent::decision_providers::typesafe::{resolve_api_key, TypeSafeAuthError};
use davinci_coding_agent::settings::to_interactive_config;
use davinci_coding_agent::settings::{load_merged_settings, Settings};

#[test]
fn setting_defaults_off_and_malformed_values_fail_safe() {
    assert!(!Settings::default().decision_intelligence_enabled());
    let malformed: Settings = serde_json::from_value(serde_json::json!({
        "decisionIntelligence": {"enabled": "yes"}
    }))
    .unwrap();
    assert!(!malformed.decision_intelligence_enabled());

    let config = to_interactive_config(&Settings::default(), "dark");
    let row = davinci_tui::interactive_settings_list(&config)
        .items
        .into_iter()
        .find(|item| item.id == "decision-intelligence")
        .expect("decision intelligence row");
    assert_eq!(row.label, "TypeSafe / Jev decision intelligence");
    assert_eq!(row.current_value, "off");
    assert_eq!(row.values, ["off", "on"]);

    let key_action = davinci_tui::interactive_settings_list(&config)
        .items
        .into_iter()
        .find(|item| item.id == "typesafe-api-key")
        .expect("TypeSafe key replacement action");
    assert_eq!(key_action.label, "TypeSafe / Jev API key");
    assert_eq!(key_action.current_value, "replace");
    assert_eq!(key_action.values, ["replace"]);
}

#[test]
fn project_settings_cannot_enable_a_user_disabled_feature() {
    let root = tempfile::tempdir().unwrap();
    let agent_dir = root.path().join("agent");
    let project = root.path().join("project");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(project.join(".pi")).unwrap();
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"defaultProjectTrust":"always","decisionIntelligence":{"enabled":false}}"#,
    )
    .unwrap();
    fs::write(
        project.join(".pi/settings.json"),
        r#"{"decisionIntelligence":{"enabled":true}}"#,
    )
    .unwrap();

    let merged = load_merged_settings(&agent_dir, &project);
    assert!(!merged.decision_intelligence_enabled());
}

#[test]
fn project_settings_can_disable_a_user_enabled_feature() {
    let root = tempfile::tempdir().unwrap();
    let agent_dir = root.path().join("agent");
    let project = root.path().join("project");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(project.join(".pi")).unwrap();
    fs::write(
        agent_dir.join("settings.json"),
        r#"{"defaultProjectTrust":"always","decisionIntelligence":{"enabled":true}}"#,
    )
    .unwrap();
    fs::write(
        project.join(".pi/settings.json"),
        r#"{"decisionIntelligence":{"enabled":false}}"#,
    )
    .unwrap();

    let merged = load_merged_settings(&agent_dir, &project);
    assert!(!merged.decision_intelligence_enabled());
}

#[test]
fn environment_key_precedes_stored_key_and_empty_override_is_explicit() {
    let previous = std::env::var("TYPESAFE_API_KEY").ok();
    let mut auth = AuthStorage::in_memory();
    auth.login_api_key("typesafe", " Bearer stored-key ")
        .unwrap();

    std::env::remove_var("TYPESAFE_API_KEY");
    assert_eq!(
        resolve_api_key(&auth).unwrap().as_deref(),
        Some("stored-key")
    );
    std::env::set_var("TYPESAFE_API_KEY", " Bearer environment-key\r\n");
    assert_eq!(
        resolve_api_key(&auth).unwrap().as_deref(),
        Some("environment-key")
    );
    std::env::set_var("TYPESAFE_API_KEY", " ");
    assert_eq!(
        resolve_api_key(&auth),
        Err(TypeSafeAuthError::InvalidEnvironment)
    );

    match previous {
        Some(value) => std::env::set_var("TYPESAFE_API_KEY", value),
        None => std::env::remove_var("TYPESAFE_API_KEY"),
    }
}
