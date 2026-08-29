use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub theme: Option<String>,
    #[serde(default)]
    pub quiet_startup: bool,
    #[serde(default)]
    pub trusted_projects: Vec<String>,
    #[serde(default)]
    pub double_escape_action: Option<String>,
    #[serde(default)]
    pub autocomplete_max_visible: Option<u32>,
    #[serde(default, rename = "treeFilterMode")]
    pub tree_filter_mode: Option<String>,
    #[serde(default)]
    pub markdown: MarkdownSettings,
    #[serde(default, rename = "enableAnalytics")]
    pub enable_analytics: Option<bool>,
    #[serde(default, rename = "trackingId")]
    pub tracking_id: Option<String>,
    #[serde(default, rename = "enabledModels")]
    pub enabled_models: Option<Vec<String>>,
    #[serde(default, rename = "autoCompact")]
    pub auto_compact: Option<bool>,
    #[serde(default, rename = "steeringMode")]
    pub steering_mode: Option<String>,
    #[serde(default, rename = "followUpMode")]
    pub follow_up_mode: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default, rename = "httpIdleTimeoutMs")]
    pub http_idle_timeout_ms: Option<u64>,
    #[serde(default, rename = "hideThinkingBlock")]
    pub hide_thinking_block: Option<bool>,
    #[serde(default, rename = "showCacheMissNotices")]
    pub show_cache_miss_notices: Option<bool>,
    #[serde(default, rename = "collapseChangelog")]
    pub collapse_changelog: Option<bool>,
    #[serde(default, rename = "enableInstallTelemetry")]
    pub enable_install_telemetry: Option<bool>,
    #[serde(default, rename = "defaultProjectTrust")]
    pub default_project_trust: Option<String>,
    #[serde(default, rename = "tuiMode")]
    pub tui_mode: Option<String>,
    #[serde(default, rename = "fullscreenExitOutput")]
    pub fullscreen_exit_output: Option<String>,
    #[serde(default, rename = "fullscreenScrollbar")]
    pub fullscreen_scrollbar: Option<String>,
    #[serde(default, rename = "fullscreenCopyOnSelect")]
    pub fullscreen_copy_on_select: Option<bool>,
    #[serde(default, rename = "showImages")]
    pub show_images: Option<bool>,
    #[serde(default, rename = "imageWidthCells")]
    pub image_width_cells: Option<u32>,
    #[serde(default, rename = "autoResizeImages")]
    pub auto_resize_images: Option<bool>,
    #[serde(default, rename = "blockImages")]
    pub block_images: Option<bool>,
    #[serde(default, rename = "enableSkillCommands")]
    pub enable_skill_commands: Option<bool>,
    #[serde(default, rename = "showHardwareCursor")]
    pub show_hardware_cursor: Option<bool>,
    #[serde(default, rename = "editorPaddingX")]
    pub editor_padding_x: Option<u32>,
    #[serde(default, rename = "outputPad")]
    pub output_pad: Option<u32>,
    #[serde(default, rename = "clearOnShrink")]
    pub clear_on_shrink: Option<bool>,
    #[serde(default, rename = "showTerminalProgress")]
    pub show_terminal_progress: Option<bool>,
    #[serde(default)]
    pub warnings: WarningSettings,
    #[serde(default, rename = "externalEditor")]
    pub external_editor: Option<String>,
    #[serde(default, rename = "modelThinkingLevels")]
    pub model_thinking_levels: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WarningSettings {
    #[serde(default, rename = "anthropicExtraUsage")]
    pub anthropic_extra_usage: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MarkdownSettings {
    #[serde(default)]
    pub mermaid: Option<String>,
}

pub fn settings_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("settings.json")
}

pub fn load_settings(agent_dir: &Path) -> Settings {
    fs::read_to_string(settings_path(agent_dir))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn save_settings(agent_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    fs::write(
        settings_path(agent_dir),
        serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?,
    )
    .map_err(|err| err.to_string())
}

pub fn should_run_first_time_setup(settings_path: &Path) -> bool {
    if std::env::var("PI_EXPERIMENTAL").ok().as_deref() != Some("1") {
        return false;
    }
    if std::env::var("PI_CODING_AGENT_DIR").is_ok() {
        return false;
    }
    !settings_path.exists()
}

pub fn set_enable_analytics(settings: &mut Settings, enabled: bool) {
    settings.enable_analytics = Some(enabled);
    if enabled && settings.tracking_id.is_none() {
        settings.tracking_id = Some(Uuid::new_v4().to_string());
    }
}

pub fn default_project_trust_label(value: Option<&str>) -> String {
    match value {
        Some("always") => "Always trust".into(),
        Some("never") => "Never trust".into(),
        _ => "Ask".into(),
    }
}

pub fn default_project_trust_value(label: &str) -> &'static str {
    match label {
        "Always trust" => "always",
        "Never trust" => "never",
        _ => "ask",
    }
}

pub fn to_interactive_config(
    settings: &Settings,
    theme: &str,
) -> pi_tui::InteractiveSettingsConfig {
    pi_tui::InteractiveSettingsConfig {
        theme: settings.theme.clone().unwrap_or_else(|| theme.to_string()),
        double_escape: settings
            .double_escape_action
            .clone()
            .unwrap_or_else(|| "tree".into()),
        quiet_startup: settings.quiet_startup,
        autocomplete_max_visible: settings.autocomplete_max_visible.unwrap_or(8),
        tree_filter_mode: settings
            .tree_filter_mode
            .clone()
            .unwrap_or_else(|| "default".into()),
        mermaid_mode: settings
            .markdown
            .mermaid
            .clone()
            .unwrap_or_else(|| "streaming".into()),
        enable_analytics: settings.enable_analytics.unwrap_or(false),
        auto_compact: settings.auto_compact.unwrap_or(true),
        steering_mode: settings
            .steering_mode
            .clone()
            .unwrap_or_else(|| "one-at-a-time".into()),
        follow_up_mode: settings
            .follow_up_mode
            .clone()
            .unwrap_or_else(|| "one-at-a-time".into()),
        transport: settings.transport.clone().unwrap_or_else(|| "auto".into()),
        http_idle_timeout: pi_tui::format_http_idle_timeout(
            settings.http_idle_timeout_ms.unwrap_or(300_000),
        ),
        hide_thinking: settings.hide_thinking_block.unwrap_or(false),
        cache_miss_notices: settings.show_cache_miss_notices.unwrap_or(false),
        collapse_changelog: settings.collapse_changelog.unwrap_or(false),
        install_telemetry: settings.enable_install_telemetry.unwrap_or(false),
        default_project_trust: default_project_trust_label(
            settings.default_project_trust.as_deref(),
        ),
        tui_mode: settings
            .tui_mode
            .clone()
            .unwrap_or_else(|| "regular".into()),
        fullscreen_exit_output: settings
            .fullscreen_exit_output
            .clone()
            .unwrap_or_else(|| "transcript".into()),
        fullscreen_scrollbar: settings
            .fullscreen_scrollbar
            .clone()
            .unwrap_or_else(|| "auto".into()),
        fullscreen_copy_on_select: settings.fullscreen_copy_on_select.unwrap_or(true),
        show_images: settings.show_images.unwrap_or(true),
        image_width_cells: settings.image_width_cells.unwrap_or(80),
        auto_resize_images: settings.auto_resize_images.unwrap_or(true),
        block_images: settings.block_images.unwrap_or(false),
        skill_commands: settings.enable_skill_commands.unwrap_or(true),
        show_hardware_cursor: settings.show_hardware_cursor.unwrap_or(false),
        editor_padding: settings.editor_padding_x.unwrap_or(0),
        output_padding: settings.output_pad.unwrap_or(1),
        clear_on_shrink: settings.clear_on_shrink.unwrap_or(false),
        terminal_progress: settings.show_terminal_progress.unwrap_or(true),
        warnings_anthropic_extra_usage: settings.warnings.anthropic_extra_usage.unwrap_or(true),
        model_thinking_summary: match &settings.model_thinking_levels {
            Some(levels) if !levels.is_empty() => format!("{} overrides", levels.len()),
            _ => "none".into(),
        },
    }
}

pub fn is_trusted(settings: &Settings, cwd: &Path, override_trust: Option<bool>) -> bool {
    if let Some(value) = override_trust {
        return value;
    }
    let canonical = cwd.to_string_lossy().into_owned();
    settings.trusted_projects.iter().any(|p| p == &canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_time_setup_gate_matches_ts() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("settings.json");
        std::env::set_var("PI_EXPERIMENTAL", "1");
        std::env::remove_var("PI_CODING_AGENT_DIR");
        assert!(should_run_first_time_setup(&path));
        std::env::remove_var("PI_EXPERIMENTAL");
        assert!(!should_run_first_time_setup(&path));
        std::env::set_var("PI_EXPERIMENTAL", "1");
        std::env::set_var("PI_CODING_AGENT_DIR", dir.path());
        assert!(!should_run_first_time_setup(&path));
        std::env::remove_var("PI_CODING_AGENT_DIR");
        std::fs::write(&path, "{}").expect("write");
        assert!(!should_run_first_time_setup(&path));
        std::env::remove_var("PI_EXPERIMENTAL");
    }

    #[test]
    fn analytics_generates_tracking_id_on_opt_in() {
        let mut settings = Settings::default();
        set_enable_analytics(&mut settings, false);
        assert_eq!(settings.enable_analytics, Some(false));
        assert!(settings.tracking_id.is_none());
        set_enable_analytics(&mut settings, true);
        assert_eq!(settings.enable_analytics, Some(true));
        let id = settings.tracking_id.clone().expect("tracking id");
        assert_eq!(id.len(), 36);
        set_enable_analytics(&mut settings, false);
        set_enable_analytics(&mut settings, true);
        assert_eq!(settings.tracking_id.as_deref(), Some(id.as_str()));
    }

    #[test]
    fn interactive_config_maps_ts_settings() {
        let mut settings = Settings {
            hide_thinking_block: Some(true),
            http_idle_timeout_ms: Some(0),
            default_project_trust: Some("never".into()),
            steering_mode: Some("all".into()),
            ..Settings::default()
        };
        settings.tree_filter_mode = Some("user-only".into());
        let config = to_interactive_config(&settings, "dark");
        assert!(config.hide_thinking);
        assert_eq!(config.http_idle_timeout, "disabled");
        assert_eq!(config.default_project_trust, "Never trust");
        assert_eq!(config.steering_mode, "all");
        assert_eq!(config.tree_filter_mode, "user-only");
        let list = pi_tui::interactive_settings_list(&config);
        let ids: Vec<_> = list.items.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"http-idle-timeout"));
        assert!(ids.contains(&"hide-thinking"));
        assert!(ids.contains(&"cache-miss-notices"));
        assert!(ids.contains(&"steering-mode"));
        assert!(ids.contains(&"warnings"));
        assert!(ids.contains(&"model-thinking"));
        assert!(ids.contains(&"theme"));
    }
}
