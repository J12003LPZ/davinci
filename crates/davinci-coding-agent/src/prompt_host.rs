//! Host-side prompt profile selection for CLI and embed entrypoints.

use davinci_agent::PromptProfile;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptSelectionSource {
    Cli,
    Sdk,
    ProjectSetting,
    UserSetting,
    Environment,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPromptProfile {
    pub profile: PromptProfile,
    pub source: PromptSelectionSource,
}

pub fn resolve_prompt_profile(
    explicit: Option<PromptProfile>,
    merged_setting: Option<&str>,
    env_value: Option<&str>,
) -> Result<ResolvedPromptProfile, String> {
    if let Some(profile) = explicit {
        return Ok(ResolvedPromptProfile {
            profile,
            source: PromptSelectionSource::Cli,
        });
    }

    if let Some(value) = merged_setting {
        return Ok(ResolvedPromptProfile {
            profile: parse_prompt_profile(value)?,
            source: PromptSelectionSource::ProjectSetting,
        });
    }

    if let Some(value) = env_value {
        return Ok(ResolvedPromptProfile {
            profile: parse_prompt_profile(value)?,
            source: PromptSelectionSource::Environment,
        });
    }

    Ok(ResolvedPromptProfile {
        profile: PromptProfile::Stable,
        source: PromptSelectionSource::Default,
    })
}

fn parse_prompt_profile(value: &str) -> Result<PromptProfile, String> {
    PromptProfile::parse(value).ok_or_else(|| {
        format!("Invalid prompt profile '{value}'. Valid profiles: stable, preview, legacy-v1")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_profile_wins_without_evaluating_lower_precedence_values() {
        assert_eq!(
            resolve_prompt_profile(
                Some(PromptProfile::Preview),
                Some("invalid-setting"),
                Some("invalid-environment"),
            ),
            Ok(ResolvedPromptProfile {
                profile: PromptProfile::Preview,
                source: PromptSelectionSource::Cli,
            })
        );
    }

    #[test]
    fn merged_setting_wins_without_evaluating_environment_value() {
        assert_eq!(
            resolve_prompt_profile(None, Some("legacy-v1"), Some("invalid-environment")),
            Ok(ResolvedPromptProfile {
                profile: PromptProfile::LegacyV1,
                source: PromptSelectionSource::ProjectSetting,
            })
        );
    }

    #[test]
    fn environment_and_default_sources_are_reported() {
        assert_eq!(
            resolve_prompt_profile(None, None, Some("preview")),
            Ok(ResolvedPromptProfile {
                profile: PromptProfile::Preview,
                source: PromptSelectionSource::Environment,
            })
        );
        assert_eq!(
            resolve_prompt_profile(None, None, None),
            Ok(ResolvedPromptProfile {
                profile: PromptProfile::Stable,
                source: PromptSelectionSource::Default,
            })
        );
    }

    #[test]
    fn invalid_selected_setting_has_actionable_diagnostics() {
        let error = resolve_prompt_profile(None, Some("experimental"), Some("stable"))
            .expect_err("a present invalid setting must not fall through");
        assert!(error.contains("experimental"));
        assert!(error.contains("stable, preview, legacy-v1"));
    }

    #[test]
    fn invalid_selected_environment_has_actionable_diagnostics() {
        let error = resolve_prompt_profile(None, None, Some("experimental"))
            .expect_err("a present invalid environment value must fail");
        assert!(error.contains("experimental"));
        assert!(error.contains("stable, preview, legacy-v1"));
    }
}
