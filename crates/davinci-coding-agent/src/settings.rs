use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub packages: Vec<PackageSource>,
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
    pub thinking_budgets: Option<davinci_ai::ThinkingBudgets>,
    #[serde(default, rename = "branchSummary")]
    pub branch_summary: Option<BranchSummarySettings>,
    #[serde(default, rename = "httpProxy")]
    pub http_proxy: Option<String>,
    #[serde(default, rename = "sessionDir")]
    pub session_dir: Option<String>,
    #[serde(default, rename = "sessionBackend")]
    pub session_backend: Option<String>,
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
    /// davinci: tool lines show their result rows (`ctrl+t` for the
    /// session). No TypeScript counterpart.
    #[serde(default, rename = "showToolOutput")]
    pub show_tool_output: Option<bool>,
    /// `web_search` provider keys. No TypeScript counterpart.
    #[serde(default, rename = "webSearch")]
    pub web_search: Option<WebSearchSettings>,
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
    #[serde(default)]
    pub terminal: Option<TerminalSettings>,
    #[serde(default)]
    pub images: Option<ImageBlockSettings>,
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
    /// Tool permissions (`permissions.rs`): the mode and the allow/deny
    /// rules. Not a vendor key; the project file's block is read only when
    /// the project is trusted.
    #[serde(default)]
    pub permissions: Option<PermissionSettings>,
    #[serde(default)]
    pub learning: Option<crate::native_extensions::learning::LearningConfig>,
    /// Settings keys this struct does not model (for example `subagents`,
    /// written by extensions). They are carried through untouched so a rewrite
    /// by `pi install`/`pi remove` cannot silently drop another tool's config.
    #[serde(flatten, default)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// `webSearch` in settings: provider keys for the `web_search` tool.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WebSearchSettings {
    #[serde(
        default,
        rename = "braveApiKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub brave_api_key: Option<String>,
}

/// `permissions` in either settings file. Rules are `tool` or
/// `tool(glob)`; see `davinci_agent::PermissionRule`.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct PermissionSettings {
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
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
    /// Absolute token count or a percentage string (for example, `"75%"`).
    /// Kept as JSON so malformed user settings can be ignored without making
    /// the entire settings file unreadable.
    #[serde(default)]
    pub threshold: Option<serde_json::Value>,
}

/// Parse the persisted threshold shape used by pi's settings file.
/// Invalid values are deliberately ignored so one malformed optional setting
/// cannot prevent the rest of the settings file from loading.
pub fn parse_compaction_threshold(
    value: &serde_json::Value,
) -> Option<davinci_agent::CompactionThreshold> {
    if let Some(tokens) = value.as_u64() {
        return (tokens >= 1_000).then_some(davinci_agent::CompactionThreshold::Tokens(tokens));
    }
    let text = value.as_str()?.trim();
    let percent = text.strip_suffix('%')?.trim().parse::<u8>().ok()?;
    (1..=100)
        .contains(&percent)
        .then_some(davinci_agent::CompactionThreshold::Percent(percent))
}

/// Normalize a value entered in the interactive settings UI into the
/// persisted JSON representation. Separators are accepted for readability,
/// and `k`/`m` suffixes are converted to absolute token counts.
pub fn normalize_compaction_threshold_input(text: &str) -> Option<serde_json::Value> {
    let compact = text
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|character| !matches!(character, ',' | '_' | ' ' | '\t' | '\r' | '\n'))
        .collect::<String>();
    if compact.is_empty() {
        return None;
    }

    if let Some(value) = compact.strip_suffix('%') {
        if value.is_empty() || !value.chars().all(|character| character.is_ascii_digit()) {
            return None;
        }
        let percent = value.parse::<u16>().ok()?;
        return (1..=100)
            .contains(&percent)
            .then(|| serde_json::json!(format!("{percent}%")));
    }

    let (number, multiplier) = if let Some(value) = compact.strip_suffix('k') {
        (value, 1_000_f64)
    } else if let Some(value) = compact.strip_suffix('m') {
        (value, 1_000_000_f64)
    } else {
        (compact.as_str(), 1_f64)
    };
    if number.is_empty() || number.matches('.').count() > 1 {
        return None;
    }
    if !number
        .chars()
        .all(|character| character.is_ascii_digit() || character == '.')
    {
        return None;
    }
    let scaled = number.parse::<f64>().ok()? * multiplier;
    if !scaled.is_finite() || scaled < 1_000_f64 || scaled > u64::MAX as f64 {
        return None;
    }
    let tokens = scaled.round() as u64;
    (tokens >= 1_000).then(|| serde_json::json!(tokens))
}

/// Persist a validated threshold in a settings value.
pub fn set_compaction_threshold(settings: &mut Settings, text: &str) -> Result<(), String> {
    let value = normalize_compaction_threshold_input(text)
        .ok_or_else(|| format!("invalid compaction threshold: {text}"))?;
    let compaction = settings.compaction.get_or_insert_with(Default::default);
    compaction.threshold = Some(value);
    Ok(())
}

/// Remove an explicit threshold while preserving other compaction settings.
pub fn clear_compaction_threshold(settings: &mut Settings) {
    if let Some(compaction) = settings.compaction.as_mut() {
        compaction.threshold = None;
        if compaction.enabled.is_none()
            && compaction.reserve_tokens.is_none()
            && compaction.keep_recent_tokens.is_none()
        {
            settings.compaction = None;
        }
    }
}

pub fn format_compaction_threshold(
    threshold: Option<davinci_agent::CompactionThreshold>,
) -> String {
    match threshold {
        Some(davinci_agent::CompactionThreshold::Percent(percent)) => format!("{percent}%"),
        Some(davinci_agent::CompactionThreshold::Tokens(tokens)) => format_token_count(tokens),
        None => "default".into(),
    }
}

fn format_token_count(tokens: u64) -> String {
    let digits = tokens.to_string();
    let first_group = digits.len() % 3;
    let mut formatted = String::with_capacity(digits.len() + digits.len() / 3);
    if first_group != 0 {
        formatted.push_str(&digits[..first_group]);
    }
    let start = first_group;
    for (index, chunk) in digits[start..].as_bytes().chunks(3).enumerate() {
        if start != 0 || index != 0 {
            formatted.push(',');
        }
        formatted.push_str(std::str::from_utf8(chunk).unwrap_or_default());
    }
    formatted
}

impl CompactionSettings {
    pub fn compaction_threshold(&self) -> Option<davinci_agent::CompactionThreshold> {
        self.threshold.as_ref().and_then(parse_compaction_threshold)
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PackageSource {
    Spec(String),
    Filtered(PackageFilter),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PackageFilter {
    pub source: String,
    #[serde(default)]
    pub autoload: Option<bool>,
    #[serde(default)]
    pub extensions: Option<Vec<String>>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub prompts: Option<Vec<String>>,
    #[serde(default)]
    pub themes: Option<Vec<String>>,
}

impl PackageSource {
    pub fn from_spec(source: impl Into<String>) -> Self {
        Self::Spec(source.into())
    }

    pub fn source(&self) -> &str {
        match self {
            Self::Spec(source) => source,
            Self::Filtered(filter) => &filter.source,
        }
    }

    pub fn autoload(&self) -> bool {
        match self {
            Self::Spec(_) => true,
            Self::Filtered(filter) => filter.autoload.unwrap_or(true),
        }
    }

    pub fn resource_patterns(&self, kind: &str) -> Option<&[String]> {
        let Self::Filtered(filter) = self else {
            return None;
        };
        match kind {
            "extensions" => filter.extensions.as_deref(),
            "skills" => filter.skills.as_deref(),
            "prompts" => filter.prompts.as_deref(),
            "themes" => filter.themes.as_deref(),
            _ => None,
        }
    }

    /// TS `collectPackageResources` filter: autoload=false starts empty; patterns glob-match.
    pub fn allows_resource(&self, kind: &str, relative: &str) -> bool {
        match (self.autoload(), self.resource_patterns(kind)) {
            (false, None) => false,
            (false, Some(patterns)) => patterns.iter().any(|pattern| glob_match(pattern, relative)),
            (true, None) => true,
            (true, Some(patterns)) => patterns.iter().any(|pattern| glob_match(pattern, relative)),
        }
    }
}

impl From<&str> for PackageSource {
    fn from(value: &str) -> Self {
        Self::from_spec(value)
    }
}

impl From<String> for PackageSource {
    fn from(value: String) -> Self {
        Self::from_spec(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PiManifest {
    pub extensions: Option<Vec<String>>,
    pub skills: Option<Vec<String>>,
    pub prompts: Option<Vec<String>>,
    pub themes: Option<Vec<String>>,
}

/// TS `readPiManifest` — `package.json` `"pi"` object with string-array resource fields.
pub fn read_pi_manifest(package_json: &Path) -> Option<PiManifest> {
    let raw = fs::read_to_string(package_json).ok()?;
    let value: serde_json::Value = serde_json::from_str(raw.trim_start_matches('\u{feff}')).ok()?;
    let pi = value.get("pi")?.as_object()?;
    let mut manifest = PiManifest::default();
    for (field, slot) in [
        ("extensions", &mut manifest.extensions),
        ("skills", &mut manifest.skills),
        ("prompts", &mut manifest.prompts),
        ("themes", &mut manifest.themes),
    ] {
        if let Some(entries) = pi.get(field).and_then(|value| value.as_array()) {
            if entries.iter().all(|entry| entry.is_string()) {
                *slot = Some(
                    entries
                        .iter()
                        .filter_map(|entry| entry.as_str().map(str::to_string))
                        .collect(),
                );
            }
        }
    }
    Some(manifest)
}

fn manifest_entries<'a>(manifest: &'a PiManifest, kind: &str) -> Option<&'a [String]> {
    match kind {
        "extensions" => manifest.extensions.as_deref(),
        "skills" => manifest.skills.as_deref(),
        "prompts" => manifest.prompts.as_deref(),
        "themes" => manifest.themes.as_deref(),
        _ => None,
    }
}

fn is_override_pattern(value: &str) -> bool {
    value.starts_with('!') || value.starts_with('+') || value.starts_with('-')
}

pub fn collect_package_resources(pkg: &PackageSource, kind: &str) -> Vec<PathBuf> {
    let root = Path::new(pkg.source());
    if let Some(manifest) = read_pi_manifest(&root.join("package.json")) {
        if let Some(entries) = manifest_entries(&manifest, kind) {
            return collect_manifest_resources(root, kind, entries, pkg);
        }
    }
    let dir = if root.join(kind).is_dir() {
        root.join(kind)
    } else {
        root.to_path_buf()
    };
    collect_dir_resources(root, &dir, pkg, kind)
}

fn collect_manifest_resources(
    root: &Path,
    kind: &str,
    entries: &[String],
    pkg: &PackageSource,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in entries.iter().filter(|item| !is_override_pattern(item)) {
        if entry.contains('*') || entry.contains('?') {
            collect_glob_files(root, entry, &mut out);
            continue;
        }
        let path = root.join(entry);
        if path.is_dir() {
            out.extend(collect_dir_resources(root, &path, pkg, kind));
        } else if path.is_file() {
            push_if_allowed(root, path, pkg, kind, &mut out);
        }
    }
    out
}

fn collect_glob_files(root: &Path, pattern: &str, out: &mut Vec<PathBuf>) {
    collect_glob_files_from(root, root, pattern, out);
}

fn collect_glob_files_from(root: &Path, dir: &Path, pattern: &str, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_glob_files_from(root, &path, pattern, out);
            continue;
        }
        if !path.is_file() {
            continue;
        }
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if glob_match(pattern, &relative) {
            out.push(path);
        }
    }
}

fn collect_dir_resources(root: &Path, dir: &Path, pkg: &PackageSource, kind: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !dir.exists() {
        return out;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(collect_dir_resources(root, &path, pkg, kind));
            } else if path.is_file() {
                push_if_allowed(root, path, pkg, kind, &mut out);
            }
        }
    }
    out
}

fn push_if_allowed(
    root: &Path,
    path: PathBuf,
    pkg: &PackageSource,
    kind: &str,
    out: &mut Vec<PathBuf>,
) {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .replace('\\', "/");
    if pkg.allows_resource(kind, &relative) {
        out.push(path);
    }
}

fn glob_match(pattern: &str, path: &str) -> bool {
    let path = path.replace('\\', "/");
    let pattern = pattern.replace('\\', "/");
    if pattern == "*" || pattern == "**" || pattern == "**/*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("**/") {
        return path == suffix
            || path.ends_with(&format!("/{suffix}"))
            || glob_match(suffix, &path);
    }
    if let Some((head, tail)) = pattern.split_once('*') {
        return path.starts_with(head)
            && path[head.len()..].find(tail).is_some_and(|index| {
                path[head.len() + index..].ends_with(tail) || tail.is_empty()
            });
    }
    path == pattern || path.ends_with(&format!("/{pattern}"))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TerminalSettings {
    #[serde(default, rename = "showImages")]
    pub show_images: Option<bool>,
    #[serde(default, rename = "imageWidthCells")]
    pub image_width_cells: Option<u32>,
    #[serde(default, rename = "clearOnShrink")]
    pub clear_on_shrink: Option<bool>,
    #[serde(default, rename = "showTerminalProgress")]
    pub show_terminal_progress: Option<bool>,
    #[serde(default)]
    pub hyperlinks: Option<serde_json::Value>,
    #[serde(default)]
    pub images: Option<serde_json::Value>,
    #[serde(default, rename = "trueColor")]
    pub true_color: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImageBlockSettings {
    #[serde(default, rename = "autoResize")]
    pub auto_resize: Option<bool>,
    #[serde(default, rename = "blockImages")]
    pub block_images: Option<bool>,
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
    let Ok(raw) = fs::read_to_string(path) else {
        return Settings::default();
    };
    match parse_settings_json(&raw) {
        Some(settings) => settings,
        None => {
            // One typo would otherwise drop every deny rule and the mode
            // without a word. Say so once per file per run: the file is
            // re-read by every sheet that opens.
            static WARNED: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());
            let mut warned = WARNED.lock().unwrap_or_else(|err| err.into_inner());
            if !warned.iter().any(|seen| seen == path) {
                warned.push(path.to_path_buf());
                eprintln!(
                    "pi: {} is not valid settings JSON; using defaults for it",
                    path.display()
                );
            }
            Settings::default()
        }
    }
}

pub fn load_merged_settings(agent_dir: &Path, cwd: &Path) -> Settings {
    load_merged_settings_with_override(agent_dir, cwd, None)
}

pub fn load_merged_settings_with_override(
    agent_dir: &Path,
    cwd: &Path,
    override_trust: Option<bool>,
) -> Settings {
    let global_value = load_settings_value(&settings_path(agent_dir));
    let global: Settings =
        serde_json::from_value(migrate_settings(global_value.clone())).unwrap_or_default();
    if !crate::trust::resolve_project_trusted(
        agent_dir,
        cwd,
        override_trust,
        global.default_project_trust.as_deref(),
        &global.trusted_projects,
    ) {
        return global;
    }
    let project = load_settings_value(&cwd.join(CONFIG_DIR_NAME).join("settings.json"));
    let merged = deep_merge_json(global_value, project);
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

/// `webSearch.braveApiKey` from settings reaches `web_search` the way the
/// proxy does: through the environment the tool already reads, and only
/// when the environment does not already say.
pub fn apply_web_search_settings(web_search: Option<&WebSearchSettings>) {
    let Some(key) = web_search
        .and_then(|settings| settings.brave_api_key.as_deref())
        .map(str::trim)
        .filter(|key| !key.is_empty())
    else {
        return;
    };
    if std::env::var_os("BRAVE_API_KEY").is_none() {
        std::env::set_var("BRAVE_API_KEY", key);
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

    pub fn compaction_settings(&self) -> davinci_agent::CompactionSettings {
        davinci_agent::CompactionSettings {
            enabled: self.compaction_enabled(),
            reserve_tokens: self
                .compaction
                .as_ref()
                .and_then(|settings| settings.reserve_tokens)
                .unwrap_or(davinci_agent::DEFAULT_RESERVE_TOKENS),
            keep_recent_tokens: self
                .compaction
                .as_ref()
                .and_then(|settings| settings.keep_recent_tokens)
                .unwrap_or(davinci_agent::DEFAULT_KEEP_RECENT_TOKENS),
            threshold: self
                .compaction
                .as_ref()
                .and_then(|settings| settings.compaction_threshold()),
        }
    }

    pub fn compaction_threshold(&self) -> Option<davinci_agent::CompactionThreshold> {
        self.compaction
            .as_ref()
            .and_then(CompactionSettings::compaction_threshold)
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
            .unwrap_or(davinci_agent::DEFAULT_RESERVE_TOKENS)
    }

    pub fn show_images(&self) -> bool {
        self.terminal
            .as_ref()
            .and_then(|terminal| terminal.show_images)
            .or(self.show_images)
            .unwrap_or(true)
    }

    pub fn image_width_cells(&self) -> u32 {
        self.terminal
            .as_ref()
            .and_then(|terminal| terminal.image_width_cells)
            .or(self.image_width_cells)
            .map(|width| width.max(1))
            .unwrap_or(60)
    }

    pub fn clear_on_shrink(&self) -> bool {
        self.terminal
            .as_ref()
            .and_then(|terminal| terminal.clear_on_shrink)
            .or(self.clear_on_shrink)
            .unwrap_or(false)
    }

    pub fn show_terminal_progress(&self) -> bool {
        self.terminal
            .as_ref()
            .and_then(|terminal| terminal.show_terminal_progress)
            .or(self.show_terminal_progress)
            .unwrap_or(false)
    }

    pub fn image_auto_resize(&self) -> bool {
        self.images
            .as_ref()
            .and_then(|images| images.auto_resize)
            .or(self.auto_resize_images)
            .unwrap_or(true)
    }

    pub fn install_telemetry_enabled(&self) -> bool {
        davinci_ai::is_install_telemetry_enabled(self.enable_install_telemetry)
    }

    pub fn block_images(&self) -> bool {
        self.images
            .as_ref()
            .and_then(|images| images.block_images)
            .or(self.block_images)
            .unwrap_or(false)
    }

    pub fn terminal_capability_overrides(&self) -> (Option<String>, Option<bool>, Option<bool>) {
        let terminal = self.terminal.as_ref();
        let images = terminal.and_then(|item| item.images.as_ref());
        let image_kind = match images {
            Some(serde_json::Value::String(kind)) if kind == "kitty" || kind == "iterm2" => {
                Some(kind.clone())
            }
            Some(serde_json::Value::Bool(false)) => Some("off".into()),
            _ => None,
        };
        let true_color = terminal.and_then(|item| {
            item.true_color
                .as_ref()
                .and_then(serde_json::Value::as_bool)
        });
        let hyperlinks = terminal.and_then(|item| {
            item.hyperlinks
                .as_ref()
                .and_then(serde_json::Value::as_bool)
        });
        (image_kind, true_color, hyperlinks)
    }

    pub fn code_block_indent(&self) -> &str {
        self.markdown.code_block_indent.as_deref().unwrap_or("  ")
    }

    pub fn session_dir_normalized(&self) -> Option<String> {
        self.session_dir.as_deref().map(|dir| {
            davinci_session::expand_tilde(dir)
                .to_string_lossy()
                .into_owned()
        })
    }

    /// `jsonl` (default, TS coding-agent) or `sqlite`.
    pub fn session_backend(&self) -> &str {
        match self.session_backend.as_deref() {
            Some(value) if !value.is_empty() => value,
            _ => "jsonl",
        }
    }
}

/// Drop `null` members so a rewrite does not expand every unset field into an
/// explicit null. A null and a missing key parse identically here, and TS `pi`
/// writes only the keys that are set.
fn prune_nulls(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            map.retain(|_, entry| !entry.is_null());
            for entry in map.values_mut() {
                prune_nulls(entry);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                prune_nulls(item);
            }
        }
        _ => {}
    }
}

pub fn save_settings(agent_dir: &Path, settings: &Settings) -> Result<(), String> {
    fs::create_dir_all(agent_dir).map_err(|err| err.to_string())?;
    let path = settings_path(agent_dir);
    with_settings_lock(&path, || {
        let mut value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
        prune_nulls(&mut value);
        fs::write(
            &path,
            serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?,
        )
        .map_err(|err| err.to_string())
    })
}

/// TS `proper-lockfile` / `lockfile.lockSync` sibling lock (`settings.json.lock`).
pub fn settings_lock_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    path.with_file_name(name)
}

pub fn with_settings_lock<T>(
    path: &Path,
    write: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    let lock_path = settings_lock_path(path);
    let _guard = acquire_settings_lock(&lock_path)?;
    write()
}

struct SettingsLock(PathBuf);

impl Drop for SettingsLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn acquire_settings_lock(lock_path: &Path) -> Result<SettingsLock, String> {
    const MAX_ATTEMPTS: u32 = 10;
    const DELAY_MS: u64 = 20;
    let mut last_error = None;
    for attempt in 1..=MAX_ATTEMPTS {
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)
        {
            Ok(_) => return Ok(SettingsLock(lock_path.to_path_buf())),
            Err(err)
                if err.kind() == std::io::ErrorKind::AlreadyExists && attempt < MAX_ATTEMPTS =>
            {
                last_error = Some(err.to_string());
                std::thread::sleep(std::time::Duration::from_millis(DELAY_MS));
            }
            Err(err) => {
                return Err(if err.kind() == std::io::ErrorKind::AlreadyExists {
                    "Failed to acquire settings lock".into()
                } else {
                    err.to_string()
                });
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "Failed to acquire settings lock".into()))
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
) -> davinci_tui::InteractiveSettingsConfig {
    davinci_tui::InteractiveSettingsConfig {
        theme: settings.theme.clone().unwrap_or_else(|| theme.to_string()),
        double_escape: settings
            .double_escape_action
            .clone()
            .unwrap_or_else(|| "tree".into()),
        quiet_startup: settings.quiet_startup,
        autocomplete_max_visible: settings.autocomplete_max_visible.unwrap_or(5),
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
        auto_compact_threshold: format_compaction_threshold(settings.compaction_threshold()),
        steering_mode: settings
            .steering_mode
            .clone()
            .unwrap_or_else(|| "one-at-a-time".into()),
        follow_up_mode: settings
            .follow_up_mode
            .clone()
            .unwrap_or_else(|| "one-at-a-time".into()),
        transport: settings.transport.clone().unwrap_or_else(|| "auto".into()),
        http_idle_timeout: davinci_tui::format_http_idle_timeout(
            settings.http_idle_timeout_ms.unwrap_or(300_000),
        ),
        hide_thinking: settings.hide_thinking_block.unwrap_or(false),
        show_tool_output: settings.show_tool_output.unwrap_or(false),
        cache_miss_notices: settings.show_cache_miss_notices.unwrap_or(false),
        collapse_changelog: settings.collapse_changelog.unwrap_or(false),
        install_telemetry: settings.install_telemetry_enabled(),
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
        show_images: settings.show_images(),
        image_width_cells: settings.image_width_cells(),
        auto_resize_images: settings.image_auto_resize(),
        block_images: settings.block_images(),
        skill_commands: settings.enable_skill_commands.unwrap_or(true),
        show_hardware_cursor: settings.show_hardware_cursor.unwrap_or(false),
        editor_padding: settings.editor_padding_x.unwrap_or(0),
        output_padding: settings.output_pad.unwrap_or(1),
        clear_on_shrink: settings.clear_on_shrink(),
        terminal_progress: settings.show_terminal_progress(),
        warnings_anthropic_extra_usage: settings.warnings.anthropic_extra_usage.unwrap_or(true),
        model_thinking_summary: match &settings.model_thinking_levels {
            Some(levels) if !levels.is_empty() => format!("{} overrides", levels.len()),
            _ => "none".into(),
        },
    }
}

pub fn is_trusted(settings: &Settings, cwd: &Path, override_trust: Option<bool>) -> bool {
    crate::trust::resolve_project_trusted(
        &davinci_session::default_agent_dir(),
        cwd,
        override_trust,
        settings.default_project_trust.as_deref(),
        &settings.trusted_projects,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewriting_settings_keeps_unknown_keys_and_writes_no_nulls() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        std::fs::create_dir_all(dir.path()).unwrap();
        std::fs::write(
            &path,
            r#"{
  "theme": "dark",
  "extensions": ["a.ts", "b.ts"],
  "subagents": {"defaultExtensions": ["a.ts"]},
  "someFutureKey": 7
}"#,
        )
        .unwrap();

        let mut settings = load_settings(dir.path());
        settings.extensions.retain(|entry| entry != "a.ts");
        save_settings(dir.path(), &settings).unwrap();

        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        // The edit landed.
        assert_eq!(written["extensions"], serde_json::json!(["b.ts"]));
        // Keys this struct does not model survive the rewrite.
        assert_eq!(
            written["subagents"],
            serde_json::json!({"defaultExtensions": ["a.ts"]})
        );
        assert_eq!(written["someFutureKey"], serde_json::json!(7));
        // Unset fields are omitted, not written as explicit nulls.
        let object = written.as_object().expect("settings object");
        assert!(
            !object.values().any(serde_json::Value::is_null),
            "settings rewrite wrote null members: {written}"
        );
        assert!(!object.contains_key("defaultModel"));
    }

    #[test]
    fn autocomplete_max_visible_defaults_to_ts_five() {
        let config = to_interactive_config(&Settings::default(), "dark");
        assert_eq!(config.autocomplete_max_visible, 5);
        let config = to_interactive_config(
            &Settings {
                autocomplete_max_visible: Some(12),
                ..Settings::default()
            },
            "dark",
        );
        assert_eq!(config.autocomplete_max_visible, 12);
    }

    #[test]
    fn show_tool_output_round_trips_into_the_settings_list() {
        let config = to_interactive_config(&Settings::default(), "dark");
        assert!(!config.show_tool_output);
        let config = to_interactive_config(
            &Settings {
                show_tool_output: Some(true),
                ..Settings::default()
            },
            "dark",
        );
        assert!(config.show_tool_output);
        let list = davinci_tui::interactive_settings_list(&config);
        let item = list
            .items
            .iter()
            .find(|item| item.id == "show-tool-output")
            .expect("show-tool-output");
        assert_eq!(item.label, "Tool output");
        assert_eq!(item.current_value, "true");
    }

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
    fn settings_lock_writes_and_releases() {
        let dir = tempfile::tempdir().expect("temp");
        let agent = dir.path();
        let dark = Settings {
            theme: Some("dark".into()),
            ..Settings::default()
        };
        save_settings(agent, &dark).unwrap();
        assert_eq!(load_settings(agent).theme.as_deref(), Some("dark"));
        assert!(!settings_lock_path(&settings_path(agent)).exists());
        let lock = settings_lock_path(&settings_path(agent));
        std::fs::write(&lock, "held").unwrap();
        let err = save_settings(agent, &dark).unwrap_err();
        assert_eq!(err, "Failed to acquire settings lock");
        std::fs::remove_file(&lock).unwrap();
        let light = Settings {
            theme: Some("light".into()),
            ..Settings::default()
        };
        save_settings(agent, &light).unwrap();
        assert_eq!(load_settings(agent).theme.as_deref(), Some("light"));
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
        assert_eq!(config.auto_compact_threshold, "default");
        assert_eq!(config.http_idle_timeout, "disabled");
        assert_eq!(config.default_project_trust, "Never trust");
        assert_eq!(config.steering_mode, "all");
        assert_eq!(config.tree_filter_mode, "user-only");
        let list = davinci_tui::interactive_settings_list(&config);
        let ids: Vec<_> = list.items.iter().map(|item| item.id.as_str()).collect();
        assert!(ids.contains(&"http-idle-timeout"));
        assert!(ids.contains(&"hide-thinking"));
        assert!(ids.contains(&"cache-miss-notices"));
        assert!(ids.contains(&"steering-mode"));
        assert!(ids.contains(&"warnings"));
        assert!(ids.contains(&"model-thinking"));
        assert!(ids.contains(&"theme"));
        let threshold = list
            .items
            .iter()
            .find(|item| item.id == "autocompact-threshold")
            .expect("auto-compact threshold setting");
        assert_eq!(threshold.current_value, "default");
        assert_eq!(threshold.values, ["default", "90%", "75%", "50%", "25%"]);
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
                "thinkingBudgets": { "medium": 4096 },
                "defaultProjectTrust": "always"
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
        assert_eq!(Settings::default().session_backend(), "jsonl");
        assert_eq!(
            Settings {
                session_backend: Some("sqlite".into()),
                ..Settings::default()
            }
            .session_backend(),
            "sqlite"
        );
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

        let filtered: PackageSource = serde_json::from_str(
            r#"{"source":"/tmp/pkg","autoload":false,"extensions":["index.js"]}"#,
        )
        .expect("package object");
        assert_eq!(filtered.source(), "/tmp/pkg");
        assert!(!filtered.autoload());
        assert!(filtered.allows_resource("extensions", "index.js"));
        assert!(!filtered.allows_resource("extensions", "other.js"));
        assert!(!filtered.allows_resource("skills", "foo.md"));
        let spec = PackageSource::from_spec("npm:demo");
        assert!(spec.allows_resource("themes", "dark.json"));

        let pkg_dir = dir.path().join("manifest-pkg");
        std::fs::create_dir_all(pkg_dir.join("src")).ok();
        std::fs::write(
            pkg_dir.join("package.json"),
            r#"{ "name": "demo", "pi": { "extensions": ["src/index.js"], "skills": ["skills/*.md"] } }"#,
        )
        .expect("manifest");
        std::fs::write(
            pkg_dir.join("src").join("index.js"),
            "export default () => {}",
        )
        .ok();
        std::fs::create_dir_all(pkg_dir.join("skills")).ok();
        std::fs::write(pkg_dir.join("skills").join("review.md"), "# review").ok();
        std::fs::write(pkg_dir.join("skills").join("skip.txt"), "no").ok();
        let manifest_pkg = PackageSource::from_spec(pkg_dir.display().to_string());
        let extensions = collect_package_resources(&manifest_pkg, "extensions");
        assert!(extensions
            .iter()
            .any(|path| path.ends_with("src/index.js") || path.ends_with("src\\index.js")));
        let skills = collect_package_resources(&manifest_pkg, "skills");
        assert!(skills.iter().any(|path| path.ends_with("review.md")));
        assert!(!skills.iter().any(|path| path.ends_with("skip.txt")));
        assert!(read_pi_manifest(&pkg_dir.join("package.json"))
            .and_then(|manifest| manifest.extensions)
            .is_some());

        let nested: Settings = serde_json::from_str(
            r#"{
                "terminal": { "showImages": false, "imageWidthCells": 40, "images": false },
                "images": { "autoResize": false, "blockImages": true }
            }"#,
        )
        .expect("nested terminal");
        assert!(!nested.show_images());
        assert_eq!(nested.image_width_cells(), 40);
        assert!(!nested.image_auto_resize());
        assert!(nested.block_images());
        assert_eq!(
            nested.terminal_capability_overrides().0.as_deref(),
            Some("off")
        );
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

    #[test]
    fn compaction_threshold_parser_accepts_valid_values_and_ignores_invalid_values() {
        assert_eq!(
            parse_compaction_threshold(&serde_json::json!(12_000)),
            Some(davinci_agent::CompactionThreshold::Tokens(12_000))
        );
        assert_eq!(
            parse_compaction_threshold(&serde_json::json!("75%")),
            Some(davinci_agent::CompactionThreshold::Percent(75))
        );
        for value in [
            serde_json::json!(999),
            serde_json::json!("0%"),
            serde_json::json!("101%"),
            serde_json::json!("75"),
            serde_json::json!(-1),
        ] {
            assert_eq!(parse_compaction_threshold(&value), None);
        }
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "compaction": { "threshold": "75%" }
        }))
        .expect("settings with threshold");
        assert_eq!(
            settings.compaction_settings().threshold,
            Some(davinci_agent::CompactionThreshold::Percent(75))
        );
    }

    #[test]
    fn compaction_threshold_input_normalizes_presets_and_suffixes() {
        assert_eq!(
            normalize_compaction_threshold_input("200,000"),
            Some(serde_json::json!(200_000))
        );
        assert_eq!(
            normalize_compaction_threshold_input("1.5m"),
            Some(serde_json::json!(1_500_000))
        );
        assert_eq!(
            normalize_compaction_threshold_input(" 60% "),
            Some(serde_json::json!("60%"))
        );
        for value in ["", "banana", "500", "0%", "101%", "-5000"] {
            assert_eq!(normalize_compaction_threshold_input(value), None, "{value}");
        }
    }

    #[test]
    fn compaction_threshold_setting_can_be_set_and_cleared() {
        let mut settings = Settings::default();
        set_compaction_threshold(&mut settings, "200k").expect("valid threshold");
        assert_eq!(
            settings.compaction_settings().threshold,
            Some(davinci_agent::CompactionThreshold::Tokens(200_000))
        );
        clear_compaction_threshold(&mut settings);
        assert_eq!(settings.compaction_settings().threshold, None);
    }

    #[test]
    fn untrusted_project_settings_are_not_merged() {
        let dir = tempfile::tempdir().expect("temp");
        let agent_dir = dir.path().join("agent");
        let project = dir.path().join("proj");
        std::fs::create_dir_all(&agent_dir).ok();
        std::fs::create_dir_all(project.join(".pi")).ok();
        std::fs::write(agent_dir.join("settings.json"), r#"{ "theme": "dark" }"#).unwrap();
        std::fs::write(
            project.join(".pi").join("settings.json"),
            r#"{ "theme": "light" }"#,
        )
        .unwrap();
        let skipped = load_merged_settings(&agent_dir, &project);
        assert_eq!(skipped.theme.as_deref(), Some("dark"));
        let forced = load_merged_settings_with_override(&agent_dir, &project, Some(true));
        assert_eq!(forced.theme.as_deref(), Some("light"));
    }

    #[test]
    fn settings_without_learning_still_deserialize() {
        let settings: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert!(settings.learning.is_none());
    }
}
