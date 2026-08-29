use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub packages: Vec<String>,
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
    #[serde(default)]
    pub compaction: Option<CompactionSettings>,
    #[serde(default)]
    pub retry: Option<RetrySettings>,
    #[serde(default, rename = "thinkingBudgets")]
    pub thinking_budgets: Option<pi_ai::ThinkingBudgets>,
    #[serde(default, rename = "branchSummary")]
    pub branch_summary: Option<BranchSummarySettings>,
    #[serde(default, rename = "httpProxy")]
    pub http_proxy: Option<String>,
    #[serde(default, rename = "sessionDir")]
    pub session_dir: Option<String>,
    #[serde(default, rename = "defaultThinkingLevel")]
    pub default_thinking_level: Option<String>,
    #[serde(default, rename = "websocketConnectTimeoutMs")]
    pub websocket_connect_timeout_ms: Option<u64>,
    #[serde(default, rename = "lastChangelogVersion")]
    pub last_changelog_version: Option<String>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub prompts: Option<Vec<String>>,
    #[serde(default)]
    pub themes: Option<Vec<String>>,
    #[serde(default, rename = "defaultTools")]
    pub default_tools: Option<Vec<String>>,
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
    #[serde(default, rename = "npmCommand")]
    pub npm_command: Option<Vec<String>>,
    #[serde(default, rename = "defaultModel")]
    pub default_model: Option<String>,
    #[serde(default, rename = "defaultProvider")]
    pub default_provider: Option<String>,
    #[serde(default, rename = "shellPath")]
    pub shell_path: Option<String>,
    #[serde(default, rename = "shellCommandPrefix")]
    pub shell_command_prefix: Option<String>,
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
    #[serde(default, rename = "codeBlockIndent")]
    pub code_block_indent: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompactionSettings {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, rename = "reserveTokens")]
    pub reserve_tokens: Option<u64>,
    #[serde(default, rename = "keepRecentTokens")]
    pub keep_recent_tokens: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderRetrySettings {
    #[serde(default, rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    #[serde(default, rename = "maxRetries")]
    pub max_retries: Option<u32>,
    #[serde(default, rename = "maxRetryDelayMs")]
    pub max_retry_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RetrySettings {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default, rename = "maxRetries")]
    pub max_retries: Option<u32>,
    #[serde(default, rename = "baseDelayMs")]
    pub base_delay_ms: Option<u64>,
    #[serde(default)]
    pub provider: Option<ProviderRetrySettings>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BranchSummarySettings {
    #[serde(default, rename = "reserveTokens")]
    pub reserve_tokens: Option<u64>,
    #[serde(default, rename = "skipPrompt")]
    pub skip_prompt: Option<bool>,
}

pub const CONFIG_DIR_NAME: &str = ".pi";
pub const DEFAULT_RETRY_BASE_DELAY_MS: u64 = 2_000;
pub const DEFAULT_PROVIDER_MAX_RETRY_DELAY_MS: u64 = 60_000;

pub fn settings_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join("settings.json")
}

pub fn load_settings(agent_dir: &Path) -> Settings {
    load_settings_file(&settings_path(agent_dir))
}

pub fn load_settings_file(path: &Path) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| parse_settings_json(&raw))
        .unwrap_or_default()
}

pub fn load_merged_settings(agent_dir: &Path, cwd: &Path) -> Settings {
    let global = load_settings_value(&settings_path(agent_dir));
    let project = load_settings_value(&cwd.join(CONFIG_DIR_NAME).join("settings.json"));
    let merged = deep_merge_json(global, project);
    serde_json::from_value(migrate_settings(merged)).unwrap_or_default()
}

fn load_settings_value(path: &Path) -> serde_json::Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| parse_settings_value(&raw))
        .unwrap_or(serde_json::Value::Object(Default::default()))
}

fn parse_settings_json(raw: &str) -> Option<Settings> {
    let value = parse_settings_value(raw)?;
    serde_json::from_value(migrate_settings(value)).ok()
}

fn parse_settings_value(raw: &str) -> Option<serde_json::Value> {
    let stripped = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    serde_json::from_str(stripped).ok()
}

fn migrate_settings(mut settings: serde_json::Value) -> serde_json::Value {
    let Some(map) = settings.as_object_mut() else {
        return settings;
    };
    if !map.contains_key("steeringMode") {
        if let Some(queue) = map.remove("queueMode") {
            map.insert("steeringMode".into(), queue);
        }
    }
    if !map.contains_key("transport") {
        if let Some(websockets) = map.remove("websockets") {
            if let Some(enabled) = websockets.as_bool() {
                map.insert(
                    "transport".into(),
                    serde_json::Value::String(if enabled { "websocket" } else { "sse" }.into()),
                );
            }
        }
    }
    if let Some(retry) = map
        .get_mut("retry")
        .and_then(serde_json::Value::as_object_mut)
    {
        if let Some(max_delay) = retry.remove("maxDelayMs") {
            let provider = retry
                .entry("provider")
                .or_insert_with(|| serde_json::Value::Object(Default::default()));
            if let Some(provider) = provider.as_object_mut() {
                if !provider.contains_key("maxRetryDelayMs") {
                    provider.insert("maxRetryDelayMs".into(), max_delay);
                }
            }
        }
    }
    settings
}

fn deep_merge_json(mut base: serde_json::Value, overrides: serde_json::Value) -> serde_json::Value {
    match (&mut base, overrides) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(over_map)) => {
            for (key, value) in over_map {
                if value.is_null() {
                    continue;
                }
                match base_map.get_mut(&key) {
                    Some(existing) if existing.is_object() && value.is_object() => {
                        *existing = deep_merge_json(existing.clone(), value);
                    }
                    _ => {
                        base_map.insert(key, value);
                    }
                }
            }
            base
        }
        (_, overrides) => overrides,
    }
}

pub fn apply_http_proxy_settings(http_proxy: Option<&str>) {
    let Some(proxy) = http_proxy.map(str::trim).filter(|value| !value.is_empty()) else {
        return;
    };
    if std::env::var_os("HTTP_PROXY").is_none() {
        std::env::set_var("HTTP_PROXY", proxy);
    }
    if std::env::var_os("HTTPS_PROXY").is_none() {
        std::env::set_var("HTTPS_PROXY", proxy);
    }
}

impl Settings {
    pub fn compaction_enabled(&self) -> bool {
        self.compaction
            .as_ref()
            .and_then(|settings| settings.enabled)
            .or(self.auto_compact)
            .unwrap_or(true)
    }

    pub fn compaction_settings(&self) -> pi_agent::CompactionSettings {
        pi_agent::CompactionSettings {
            enabled: self.compaction_enabled(),
            reserve_tokens: self
                .compaction
                .as_ref()
                .and_then(|settings| settings.reserve_tokens)
                .unwrap_or(pi_agent::DEFAULT_RESERVE_TOKENS),
            keep_recent_tokens: self
                .compaction
                .as_ref()
                .and_then(|settings| settings.keep_recent_tokens)
                .unwrap_or(pi_agent::DEFAULT_KEEP_RECENT_TOKENS),
        }
    }

    pub fn retry_enabled(&self) -> bool {
        self.retry
            .as_ref()
            .and_then(|settings| settings.enabled)
            .unwrap_or(true)
    }

    pub fn retry_max_retries(&self) -> u32 {
        self.retry
            .as_ref()
            .and_then(|settings| settings.max_retries)
            .unwrap_or(3)
    }

    pub fn retry_base_delay_ms(&self) -> u64 {
        self.retry
            .as_ref()
            .and_then(|settings| settings.base_delay_ms)
            .unwrap_or(DEFAULT_RETRY_BASE_DELAY_MS)
    }

    pub fn provider_timeout_ms(&self) -> Option<u64> {
        self.retry
            .as_ref()
            .and_then(|settings| settings.provider.as_ref())
            .and_then(|provider| provider.timeout_ms)
    }

    pub fn provider_max_retries(&self) -> Option<u32> {
        self.retry
            .as_ref()
            .and_then(|settings| settings.provider.as_ref())
            .and_then(|provider| provider.max_retries)
    }

    pub fn provider_max_retry_delay_ms(&self) -> u64 {
        self.retry
            .as_ref()
            .and_then(|settings| settings.provider.as_ref())
            .and_then(|provider| provider.max_retry_delay_ms)
            .unwrap_or(DEFAULT_PROVIDER_MAX_RETRY_DELAY_MS)
    }

    pub fn branch_summary_skip_prompt(&self) -> bool {
        self.branch_summary
            .as_ref()
            .and_then(|settings| settings.skip_prompt)
            .unwrap_or(false)
    }

    pub fn branch_summary_reserve_tokens(&self) -> u64 {
        self.branch_summary
            .as_ref()
            .and_then(|settings| settings.reserve_tokens)
            .unwrap_or(pi_agent::DEFAULT_RESERVE_TOKENS)
    }

    pub fn code_block_indent(&self) -> &str {
        self.markdown.code_block_indent.as_deref().unwrap_or("  ")
    }

    pub fn session_dir_normalized(&self) -> Option<String> {
        self.session_dir
            .as_deref()
            .map(|dir| pi_session::expand_tilde(dir).to_string_lossy().into_owned())
    }
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
        auto_compact: settings.compaction_enabled(),
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

    #[test]
    fn nested_settings_defaults_and_merge_match_ts() {
        let empty = Settings::default();
        assert!(empty.compaction_enabled());
        assert_eq!(empty.compaction_settings().reserve_tokens, 16_384);
        assert_eq!(empty.compaction_settings().keep_recent_tokens, 20_000);
        assert!(empty.retry_enabled());
        assert_eq!(empty.retry_max_retries(), 3);
        assert_eq!(empty.retry_base_delay_ms(), 2_000);
        assert_eq!(empty.provider_max_retry_delay_ms(), 60_000);
        assert!(!empty.branch_summary_skip_prompt());
        assert_eq!(empty.branch_summary_reserve_tokens(), 16_384);

        let dir = tempfile::tempdir().expect("temp");
        let agent_dir = dir.path().join("agent");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(agent_dir.join("x")).ok();
        std::fs::create_dir_all(project.join(".pi")).ok();
        std::fs::write(
            agent_dir.join("settings.json"),
            r#"{
                "sessionDir": "~/sessions",
                "httpProxy": "http://127.0.0.1:7890",
                "retry": { "provider": { "timeoutMs": 30000, "maxRetryDelayMs": 45000 } },
                "compaction": { "enabled": true, "reserveTokens": 10 },
                "retry": { "maxDelayMs": 12000, "provider": { "timeoutMs": 30000 } }
            }"#,
        )
        .expect("write");
        // rewrite without duplicate retry key
        std::fs::write(
            agent_dir.join("settings.json"),
            r#"{
                "sessionDir": "/tmp/sessions",
                "httpProxy": "http://127.0.0.1:7890",
                "compaction": { "enabled": true, "reserveTokens": 10 },
                "retry": { "maxDelayMs": 12000, "provider": { "timeoutMs": 30000 } },
                "branchSummary": { "skipPrompt": true, "reserveTokens": 99 },
                "thinkingBudgets": { "medium": 4096 }
            }"#,
        )
        .expect("write");
        std::fs::write(
            project.join(".pi").join("settings.json"),
            r#"{ "retry": { "provider": { "maxRetries": 2 } }, "sessionDir": "./sessions" }"#,
        )
        .expect("write");
        let merged = load_merged_settings(&agent_dir, &project);
        assert_eq!(merged.session_dir.as_deref(), Some("./sessions"));
        assert_eq!(
            Settings {
                session_dir: Some("~/sessions".into()),
                ..Settings::default()
            }
            .session_dir_normalized()
            .as_deref()
            .map(|path| path.ends_with("sessions")),
            Some(true)
        );
        assert_eq!(Settings::default().code_block_indent(), "  ");
        assert_eq!(merged.provider_timeout_ms(), Some(30_000));
        assert_eq!(merged.provider_max_retries(), Some(2));
        assert_eq!(merged.provider_max_retry_delay_ms(), 12_000);
        assert_eq!(merged.compaction_settings().reserve_tokens, 10);
        assert!(merged.branch_summary_skip_prompt());
        assert_eq!(
            merged.thinking_budgets.as_ref().and_then(|b| b.medium),
            Some(4096)
        );

        let previous_http = std::env::var_os("HTTP_PROXY");
        let previous_https = std::env::var_os("HTTPS_PROXY");
        std::env::remove_var("HTTP_PROXY");
        std::env::remove_var("HTTPS_PROXY");
        apply_http_proxy_settings(Some("   "));
        assert!(std::env::var_os("HTTP_PROXY").is_none());
        apply_http_proxy_settings(Some("http://127.0.0.1:7890"));
        assert_eq!(
            std::env::var("HTTP_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(
            std::env::var("HTTPS_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        apply_http_proxy_settings(Some("http://settings:9"));
        assert_eq!(
            std::env::var("HTTP_PROXY").ok().as_deref(),
            Some("http://127.0.0.1:7890")
        );
        match previous_http {
            Some(value) => std::env::set_var("HTTP_PROXY", value),
            None => std::env::remove_var("HTTP_PROXY"),
        }
        match previous_https {
            Some(value) => std::env::set_var("HTTPS_PROXY", value),
            None => std::env::remove_var("HTTPS_PROXY"),
        }
    }
}
