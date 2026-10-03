//! Explicit user control of the active Codex service tier.
use davinci_ai::{CodexServiceTier, FastCapability};
use std::path::Path;

pub fn toggle(agent: &mut davinci_agent::Agent, capability: FastCapability) -> Result<(), String> {
    if agent.provider != "openai-codex"
        || crate::current_runtime_model(agent)
            .is_some_and(|model| model.api != "openai-codex-responses")
    {
        return Err("Fast mode requires the OpenAI Codex subscription route".into());
    }
    let next = if agent.service_tier == CodexServiceTier::Fast {
        CodexServiceTier::Standard
    } else {
        CodexServiceTier::Fast.validate_capability(capability, &agent.model_id)?;
        CodexServiceTier::Fast
    };
    agent.service_tier = next;
    Ok(())
}

pub fn persist(agent_dir: &Path, tier: CodexServiceTier) -> Result<(), String> {
    std::fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    let path = crate::settings::settings_path(agent_dir);
    crate::settings::with_settings_lock(&path, || {
        let mut value = match std::fs::read_to_string(&path) {
            Ok(raw) => {
                serde_json::from_str::<serde_json::Value>(&raw).map_err(|err| err.to_string())?
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => serde_json::json!({}),
            Err(err) => return Err(err.to_string()),
        };
        let object = value
            .as_object_mut()
            .ok_or("settings must be a JSON object")?;
        if tier == CodexServiceTier::Standard {
            object.remove("serviceTier");
        } else {
            object.insert("serviceTier".into(), tier.settings_value().into());
        }
        let text = serde_json::to_vec_pretty(&value).map_err(|err| err.to_string())?;
        davinci_sys::fs::atomic_write(&path, &text).map_err(|err| err.to_string())
    })
}

pub fn toggle_and_persist(
    agent: &mut davinci_agent::Agent,
    capability: FastCapability,
    agent_dir: &Path,
) -> String {
    if let Err(err) = toggle(agent, capability) {
        return err;
    }
    let label = speed_label(agent, capability);
    match persist(agent_dir, agent.service_tier) {
        Ok(()) => format!("Speed {label}"),
        Err(err) => format!("Speed {label} · this run only ({err})"),
    }
}

pub fn speed_label(agent: &davinci_agent::Agent, capability: FastCapability) -> String {
    if agent.provider != "openai-codex"
        || crate::current_runtime_model(agent)
            .is_some_and(|model| model.api != "openai-codex-responses")
    {
        return String::new();
    }
    let label = agent.service_tier.label();
    match (agent.service_tier, capability) {
        (CodexServiceTier::Fast, FastCapability::Unknown) => {
            format!("{label} · capability unverified")
        }
        (CodexServiceTier::Fast, FastCapability::Unsupported) => {
            format!("{label} requested · blocked: not advertised")
        }
        _ => label.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use davinci_ai::{CodexServiceTier, FastCapability};

    #[test]
    fn saved_unsupported_fast_is_labeled_blocked_and_can_be_disabled() {
        let mut agent = davinci_agent::Agent::new("system");
        agent.provider = "openai-codex".into();
        agent.model_id = "fixture-model".into();
        agent.service_tier = CodexServiceTier::Fast;
        assert!(speed_label(&agent, FastCapability::Unsupported).contains("blocked"));
        toggle(&mut agent, FastCapability::Unsupported).unwrap();
        assert_eq!(agent.service_tier, CodexServiceTier::Standard);
        assert!(toggle(&mut agent, FastCapability::Unsupported).is_err());
    }

    #[test]
    fn fast_toggle_preserves_model_effort_and_session() {
        let mut agent = davinci_agent::Agent::new("system");
        agent.provider = "openai-codex".into();
        agent.model_id = "fixture-model".into();
        agent.thinking_level = davinci_protocol::ThinkingLevel::High;
        for (before, after) in [
            (CodexServiceTier::Standard, CodexServiceTier::Fast),
            (CodexServiceTier::Fast, CodexServiceTier::Standard),
            (CodexServiceTier::Flex, CodexServiceTier::Fast),
        ] {
            agent.service_tier = before;
            toggle(&mut agent, FastCapability::Unknown).unwrap();
            assert_eq!(agent.service_tier, after);
            assert_eq!(agent.model_id, "fixture-model");
            assert_eq!(agent.thinking_level, davinci_protocol::ThinkingLevel::High);
            assert_eq!(agent.system_prompt, "system");
        }
        agent.service_tier = CodexServiceTier::Standard;
        assert!(toggle(&mut agent, FastCapability::Unsupported).is_err());
        assert_eq!(agent.service_tier, CodexServiceTier::Standard);
        agent.provider = "anthropic".into();
        assert!(speed_label(&agent, FastCapability::Supported).is_empty());
        assert!(toggle(&mut agent, FastCapability::Supported).is_err());
        assert_eq!(agent.service_tier, CodexServiceTier::Standard);
    }

    #[test]
    fn fast_persistence_normalizes_and_clears_without_losing_settings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"serviceTier":"priority","customSetting":42}"#,
        )
        .unwrap();
        persist(dir.path(), CodexServiceTier::Fast).unwrap();
        let read = || {
            serde_json::from_str::<serde_json::Value>(
                &std::fs::read_to_string(dir.path().join("settings.json")).unwrap(),
            )
            .unwrap()
        };
        assert_eq!(read()["serviceTier"], "fast");
        assert_eq!(read()["customSetting"], 42);
        persist(dir.path(), CodexServiceTier::Standard).unwrap();
        assert!(read().get("serviceTier").is_none());
        assert_eq!(read()["customSetting"], 42);
    }

    #[test]
    fn fast_persistence_failure_keeps_session_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("settings.json")).unwrap();
        let mut agent = davinci_agent::Agent::new("system");
        agent.provider = "openai-codex".into();
        let result = toggle_and_persist(&mut agent, FastCapability::Unknown, dir.path());
        assert!(result.contains("this run only"));
        assert_eq!(agent.service_tier, CodexServiceTier::Fast);
    }

    #[test]
    fn other_providers_cannot_toggle_or_display_fast() {
        for provider in ["anthropic", "google", "azure-openai", "openai", "ollama"] {
            let dir = tempfile::tempdir().unwrap();
            let mut agent = davinci_agent::Agent::new("system");
            agent.provider = provider.into();
            agent.service_tier = CodexServiceTier::Fast;
            let result = toggle_and_persist(&mut agent, FastCapability::Supported, dir.path());
            assert!(result.contains("requires the OpenAI Codex subscription route"));
            assert_eq!(agent.service_tier, CodexServiceTier::Fast);
            assert!(speed_label(&agent, FastCapability::Supported).is_empty());
            assert!(!dir.path().join("settings.json").exists());
        }
    }
}
