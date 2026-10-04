use std::fs;

use davinci_coding_agent::settings::{
    load_merged_settings, load_settings_file, save_settings, to_interactive_config, update_settings,
    Settings,
};
use serde_json::{json, Value};

#[test]
fn legacy_decision_settings_are_ignored_without_losing_other_settings() {
    for legacy in [
        Value::Null,
        json!(true),
        json!("invalid"),
        json!(["invalid"]),
        json!({"enabled": "yes"}),
        json!({
            "enabled": true,
            "effortAdvice": true,
            "completionAdvice": true,
            "toolFamilyAdvice": true
        }),
    ] {
        let settings: Settings = serde_json::from_value(json!({
            "decisionIntelligence": legacy,
            "defaultProvider": "openai-codex",
            "defaultModel": "gpt-5.4",
            "subagents": {"maxConcurrent": 3}
        }))
        .unwrap();

        assert_eq!(settings.default_provider.as_deref(), Some("openai-codex"));
        assert_eq!(settings.default_model.as_deref(), Some("gpt-5.4"));
        assert!(!settings.extra.contains_key("decisionIntelligence"));
        let serialized = serde_json::to_value(&settings).unwrap();
        assert!(serialized.get("decisionIntelligence").is_none());
        assert_eq!(serialized["subagents"], json!({"maxConcurrent": 3}));
    }
}

#[test]
fn removed_settings_cannot_be_serialized_through_extension_config() {
    let mut settings = Settings::default();
    settings
        .extra
        .insert("decisionIntelligence".into(), json!({"enabled": true}));
    settings.extra.insert("customExtension".into(), json!(42));

    let serialized = serde_json::to_value(settings).unwrap();

    assert!(serialized.get("decisionIntelligence").is_none());
    assert_eq!(serialized["customExtension"], json!(42));
}

#[test]
fn trusted_project_merge_discards_legacy_settings_and_preserves_unrelated_values() {
    let root = tempfile::tempdir().unwrap();
    let agent_dir = root.path().join("agent");
    let project = root.path().join("project");
    fs::create_dir_all(&agent_dir).unwrap();
    fs::create_dir_all(project.join(".pi")).unwrap();
    fs::write(
        agent_dir.join("settings.json"),
        json!({
            "defaultProjectTrust": "always",
            "defaultProvider": "openai-codex",
            "decisionIntelligence": {"enabled": true, "effortAdvice": true},
            "subagents": {"maxConcurrent": 3}
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        project.join(".pi/settings.json"),
        json!({
            "decisionIntelligence": {"completionAdvice": true, "toolFamilyAdvice": true},
            "transport": "sse",
            "subagents": {"projectOption": true}
        })
        .to_string(),
    )
    .unwrap();

    let merged = load_merged_settings(&agent_dir, &project);
    assert_eq!(merged.default_provider.as_deref(), Some("openai-codex"));
    assert_eq!(merged.transport.as_deref(), Some("sse"));
    assert!(!merged.extra.contains_key("decisionIntelligence"));
    let serialized = serde_json::to_value(merged).unwrap();
    assert!(serialized.get("decisionIntelligence").is_none());
    assert_eq!(
        serialized["subagents"],
        json!({"maxConcurrent": 3, "projectOption": true})
    );
}

#[test]
fn settings_rewrites_drop_legacy_config_and_preserve_provider_and_extension_settings() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("settings.json");
    let original = json!({
        "decisionIntelligence": {"enabled": true},
        "defaultProvider": "openai-codex",
        "customExtension": {"enabled": true}
    });
    fs::write(&path, original.to_string()).unwrap();

    let settings = load_settings_file(&path);
    save_settings(root.path(), &settings).unwrap();
    let saved: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(saved.get("decisionIntelligence").is_none());
    assert_eq!(saved["defaultProvider"], original["defaultProvider"]);
    assert_eq!(saved["customExtension"], original["customExtension"]);

    fs::write(&path, original.to_string()).unwrap();
    update_settings(root.path(), |settings| {
        settings.show_tool_output = Some(true);
    })
    .unwrap();
    let updated: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert!(updated.get("decisionIntelligence").is_none());
    assert_eq!(updated["defaultProvider"], original["defaultProvider"]);
    assert_eq!(updated["customExtension"], original["customExtension"]);
    assert_eq!(updated["showToolOutput"], json!(true));
}

#[test]
fn interactive_settings_expose_no_removed_feature_or_key_prompt() {
    let settings: Settings = serde_json::from_value(json!({
        "decisionIntelligence": {"enabled": true},
        "transport": "sse"
    }))
    .unwrap();
    for settings in [Settings::default(), settings] {
        let config = to_interactive_config(&settings, "dark");
        let list = davinci_tui::interactive_settings_list(&config);
        assert!(!list.items.iter().any(|item| {
            matches!(item.id.as_str(), "decision-intelligence" | "typesafe-api-key")
        }));
        assert!(list.items.iter().any(|item| item.id == "transport"));
        assert!(list.items.iter().any(|item| item.id == "show-tool-output"));
        let serialized = serde_json::to_value(settings).unwrap();
        assert!(serialized.get("decisionIntelligence").is_none());
    }
}
