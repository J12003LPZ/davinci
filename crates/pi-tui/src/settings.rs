use crate::fuzzy::fuzzy_filter;
use crate::render::Component;

#[derive(Debug, Clone)]
pub struct SettingItem {
    pub id: String,
    pub label: String,
    pub description: Option<String>,
    pub current_value: String,
    pub values: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SettingsList {
    pub items: Vec<SettingItem>,
    pub selected: usize,
    pub query: String,
    pub max_visible: usize,
}

impl SettingsList {
    pub fn new(items: Vec<SettingItem>, max_visible: usize) -> Self {
        Self {
            items,
            selected: 0,
            query: String::new(),
            max_visible,
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let filtered = self.filtered();
        if filtered.is_empty() {
            return;
        }
        let len = filtered.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    pub fn selected_item(&self) -> Option<SettingItem> {
        self.filtered().get(self.selected).cloned()
    }

    pub fn cycle(&mut self) {
        if let Some(item) = self.filtered().get(self.selected).cloned() {
            if let Some(item) = self.items.iter_mut().find(|i| i.id == item.id) {
                if item.values.is_empty() {
                    return;
                }
                let current = item
                    .values
                    .iter()
                    .position(|v| v == &item.current_value)
                    .unwrap_or(0);
                item.current_value = item.values[(current + 1) % item.values.len()].clone();
            }
        }
    }

    fn filtered(&self) -> Vec<SettingItem> {
        if self.query.is_empty() {
            return self.items.clone();
        }
        let labels: Vec<String> = self.items.iter().map(|i| i.label.clone()).collect();
        let kept = fuzzy_filter(&self.query, &labels);
        self.items
            .iter()
            .filter(|item| kept.contains(&item.label))
            .cloned()
            .collect()
    }
}

impl Component for SettingsList {
    fn render(&self, width: usize) -> Vec<String> {
        let filtered = self.filtered();
        filtered
            .iter()
            .take(self.max_visible)
            .enumerate()
            .map(|(index, item)| {
                let prefix = if index == self.selected { "> " } else { "  " };
                let line = format!("{prefix}{}  {}", item.label, item.current_value);
                if line.len() > width {
                    line.chars().take(width).collect()
                } else {
                    line
                }
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        if data == " " || data == "\n" {
            self.cycle();
        } else {
            self.query.push_str(data);
        }
    }

    fn invalidate(&mut self) {}
}

#[derive(Debug, Clone)]
pub struct InteractiveSettingsConfig {
    pub theme: String,
    pub double_escape: String,
    pub quiet_startup: bool,
    pub autocomplete_max_visible: u32,
    pub tree_filter_mode: String,
    pub mermaid_mode: String,
    pub enable_analytics: bool,
    pub auto_compact: bool,
    pub steering_mode: String,
    pub follow_up_mode: String,
    pub transport: String,
    pub http_idle_timeout: String,
    pub hide_thinking: bool,
    pub cache_miss_notices: bool,
    pub collapse_changelog: bool,
    pub install_telemetry: bool,
    pub default_project_trust: String,
    pub tui_mode: String,
    pub fullscreen_exit_output: String,
    pub fullscreen_scrollbar: String,
    pub fullscreen_copy_on_select: bool,
    pub show_images: bool,
    pub image_width_cells: u32,
    pub auto_resize_images: bool,
    pub block_images: bool,
    pub skill_commands: bool,
    pub show_hardware_cursor: bool,
    pub editor_padding: u32,
    pub output_padding: u32,
    pub clear_on_shrink: bool,
    pub terminal_progress: bool,
    pub warnings_anthropic_extra_usage: bool,
    pub model_thinking_summary: String,
}

impl Default for InteractiveSettingsConfig {
    fn default() -> Self {
        Self {
            theme: "dark".into(),
            double_escape: "tree".into(),
            quiet_startup: false,
            autocomplete_max_visible: 8,
            tree_filter_mode: "default".into(),
            mermaid_mode: "streaming".into(),
            enable_analytics: false,
            auto_compact: true,
            steering_mode: "one-at-a-time".into(),
            follow_up_mode: "one-at-a-time".into(),
            transport: "auto".into(),
            http_idle_timeout: "5 min".into(),
            hide_thinking: false,
            cache_miss_notices: false,
            collapse_changelog: false,
            install_telemetry: false,
            default_project_trust: "Ask".into(),
            tui_mode: "regular".into(),
            fullscreen_exit_output: "transcript".into(),
            fullscreen_scrollbar: "auto".into(),
            fullscreen_copy_on_select: true,
            show_images: true,
            image_width_cells: 80,
            auto_resize_images: true,
            block_images: false,
            skill_commands: true,
            show_hardware_cursor: false,
            editor_padding: 0,
            output_padding: 1,
            clear_on_shrink: false,
            terminal_progress: true,
            warnings_anthropic_extra_usage: true,
            model_thinking_summary: "none".into(),
        }
    }
}

pub fn format_http_idle_timeout(timeout_ms: u64) -> String {
    match timeout_ms {
        30_000 => "30 sec".into(),
        60_000 => "1 min".into(),
        120_000 => "2 min".into(),
        300_000 => "5 min".into(),
        0 => "disabled".into(),
        other => format!("{} sec", other / 1000),
    }
}

pub fn parse_http_idle_timeout(label: &str) -> Option<u64> {
    match label {
        "30 sec" => Some(30_000),
        "1 min" => Some(60_000),
        "2 min" => Some(120_000),
        "5 min" => Some(300_000),
        "disabled" => Some(0),
        _ => None,
    }
}

fn bool_item(id: &str, label: &str, description: &str, value: bool) -> SettingItem {
    SettingItem {
        id: id.into(),
        label: label.into(),
        description: Some(description.into()),
        current_value: if value { "true" } else { "false" }.into(),
        values: vec!["true".into(), "false".into()],
    }
}

pub fn interactive_settings_list(config: &InteractiveSettingsConfig) -> SettingsList {
    SettingsList::new(
        vec![
            bool_item(
                "autocompact",
                "Auto-compact",
                "Automatically compact context when it gets too large",
                config.auto_compact,
            ),
            bool_item(
                "show-images",
                "Show images",
                "Render images inline in terminal",
                config.show_images,
            ),
            SettingItem {
                id: "image-width-cells".into(),
                label: "Image width".into(),
                description: Some("Preferred inline image width in terminal cells".into()),
                current_value: config.image_width_cells.to_string(),
                values: vec!["60".into(), "80".into(), "120".into()],
            },
            bool_item(
                "auto-resize-images",
                "Auto-resize images",
                "Resize large images to 2000x2000 max for better model compatibility",
                config.auto_resize_images,
            ),
            bool_item(
                "block-images",
                "Block images",
                "Prevent images from being sent to LLM providers",
                config.block_images,
            ),
            bool_item(
                "skill-commands",
                "Skill commands",
                "Register skills as /skill:name commands",
                config.skill_commands,
            ),
            bool_item(
                "show-hardware-cursor",
                "Show hardware cursor",
                "Show the terminal cursor while still positioning it for IME support",
                config.show_hardware_cursor,
            ),
            SettingItem {
                id: "editor-padding".into(),
                label: "Editor padding".into(),
                description: Some("Horizontal padding for input editor (0-3)".into()),
                current_value: config.editor_padding.to_string(),
                values: vec!["0".into(), "1".into(), "2".into(), "3".into()],
            },
            SettingItem {
                id: "output-padding".into(),
                label: "Output padding".into(),
                description: Some(
                    "Horizontal padding for user messages, assistant messages, and thinking".into(),
                ),
                current_value: config.output_padding.to_string(),
                values: vec!["0".into(), "1".into()],
            },
            SettingItem {
                id: "autocomplete-max-visible".into(),
                label: "Autocomplete max items".into(),
                description: Some("Max visible items in autocomplete dropdown (3-20)".into()),
                current_value: config.autocomplete_max_visible.to_string(),
                values: vec![
                    "3".into(),
                    "5".into(),
                    "7".into(),
                    "10".into(),
                    "15".into(),
                    "20".into(),
                ],
            },
            bool_item(
                "clear-on-shrink",
                "Clear on shrink",
                "Clear empty rows when content shrinks (may cause flicker)",
                config.clear_on_shrink,
            ),
            bool_item(
                "terminal-progress",
                "Terminal progress",
                "Show OSC 9;4 progress indicators in the terminal tab bar",
                config.terminal_progress,
            ),
            SettingItem {
                id: "steering-mode".into(),
                label: "Steering mode".into(),
                description: Some(
                    "Enter while streaming queues steering messages. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once."
                        .into(),
                ),
                current_value: config.steering_mode.clone(),
                values: vec!["one-at-a-time".into(), "all".into()],
            },
            SettingItem {
                id: "follow-up-mode".into(),
                label: "Follow-up mode".into(),
                description: Some(
                    "Queue follow-up messages until agent stops. 'one-at-a-time': deliver one, wait for response. 'all': deliver all at once."
                        .into(),
                ),
                current_value: config.follow_up_mode.clone(),
                values: vec!["one-at-a-time".into(), "all".into()],
            },
            SettingItem {
                id: "transport".into(),
                label: "Transport".into(),
                description: Some(
                    "Preferred transport for providers that support multiple transports".into(),
                ),
                current_value: config.transport.clone(),
                values: vec![
                    "sse".into(),
                    "websocket".into(),
                    "websocket-cached".into(),
                    "auto".into(),
                ],
            },
            SettingItem {
                id: "http-idle-timeout".into(),
                label: "HTTP idle timeout".into(),
                description: Some(
                    "Maximum idle gap while waiting for HTTP headers or body chunks. Disable for local models that pause longer than five minutes."
                        .into(),
                ),
                current_value: config.http_idle_timeout.clone(),
                values: vec![
                    "30 sec".into(),
                    "1 min".into(),
                    "2 min".into(),
                    "5 min".into(),
                    "disabled".into(),
                ],
            },
            bool_item(
                "hide-thinking",
                "Hide thinking",
                "Hide thinking blocks in assistant responses",
                config.hide_thinking,
            ),
            SettingItem {
                id: "mermaid-rendering".into(),
                label: "Mermaid diagrams".into(),
                description: Some("Render Mermaid code blocks as Unicode diagrams".into()),
                current_value: config.mermaid_mode.clone(),
                values: vec!["off".into(), "final".into(), "streaming".into()],
            },
            bool_item(
                "cache-miss-notices",
                "Cache miss notices",
                "Show transcript notices for significant prompt-cache misses and compaction costs",
                config.cache_miss_notices,
            ),
            bool_item(
                "collapse-changelog",
                "Collapse changelog",
                "Show condensed changelog after updates",
                config.collapse_changelog,
            ),
            bool_item(
                "quiet-startup",
                "Quiet startup",
                "Disable verbose printing at startup",
                config.quiet_startup,
            ),
            bool_item(
                "install-telemetry",
                "Install telemetry",
                "Send an anonymous version/update ping after changelog-detected updates",
                config.install_telemetry,
            ),
            SettingItem {
                id: "default-project-trust".into(),
                label: "Default project trust".into(),
                description: Some(
                    "Fallback behavior when no extension or saved trust decision decides project trust"
                        .into(),
                ),
                current_value: config.default_project_trust.clone(),
                values: vec!["Ask".into(), "Always trust".into(), "Never trust".into()],
            },
            SettingItem {
                id: "double-escape-action".into(),
                label: "Double-escape action".into(),
                description: Some("Action when pressing Escape twice with empty editor".into()),
                current_value: config.double_escape.clone(),
                values: vec!["tree".into(), "fork".into(), "none".into()],
            },
            SettingItem {
                id: "tree-filter-mode".into(),
                label: "Tree filter mode".into(),
                description: Some("Default filter when opening /tree".into()),
                current_value: config.tree_filter_mode.clone(),
                values: vec![
                    "default".into(),
                    "no-tools".into(),
                    "user-only".into(),
                    "labeled-only".into(),
                    "all".into(),
                ],
            },
            SettingItem {
                id: "tui-mode".into(),
                label: "TUI mode".into(),
                description: Some("Interface layout; fullscreen mode is experimental".into()),
                current_value: config.tui_mode.clone(),
                values: vec!["regular".into(), "fullscreen".into()],
            },
            SettingItem {
                id: "fullscreen-exit-output".into(),
                label: "Fullscreen exit output".into(),
                description: Some(
                    "Print the transcript or only a session resume hint when exiting fullscreen mode"
                        .into(),
                ),
                current_value: config.fullscreen_exit_output.clone(),
                values: vec!["transcript".into(), "resume-hint".into()],
            },
            SettingItem {
                id: "fullscreen-scrollbar".into(),
                label: "Fullscreen scrollbar".into(),
                description: Some(
                    "Scrollbar behavior in fullscreen mode; has no effect in regular mode".into(),
                ),
                current_value: config.fullscreen_scrollbar.clone(),
                values: vec!["auto".into(), "always".into(), "hidden".into()],
            },
            bool_item(
                "fullscreen-copy-on-select",
                "Fullscreen copy on select",
                "Automatically copy selected text in fullscreen mode; disable to copy selections with Ctrl+X",
                config.fullscreen_copy_on_select,
            ),
            SettingItem {
                id: "theme".into(),
                label: "Theme".into(),
                description: Some("Color theme for the interface".into()),
                current_value: config.theme.clone(),
                values: vec!["dark".into(), "light".into(), "pi".into()],
            },
            SettingItem {
                id: "warnings".into(),
                label: "Warnings".into(),
                description: Some("Configure warning prompts".into()),
                current_value: if config.warnings_anthropic_extra_usage {
                    "configure".into()
                } else {
                    "off".into()
                },
                values: Vec::new(),
            },
            SettingItem {
                id: "model-thinking".into(),
                label: "Per-model thinking".into(),
                description: Some("Override thinking level per model".into()),
                current_value: config.model_thinking_summary.clone(),
                values: Vec::new(),
            },
            bool_item(
                "enable-analytics",
                "Share anonymous usage data",
                "Opt-in analytics data sharing",
                config.enable_analytics,
            ),
        ],
        12,
    )
}

pub fn default_interactive_settings(
    theme: &str,
    double_escape: &str,
    quiet_startup: bool,
    autocomplete_max_visible: u32,
    tree_filter_mode: &str,
    mermaid_mode: &str,
    enable_analytics: bool,
) -> SettingsList {
    interactive_settings_list(&InteractiveSettingsConfig {
        theme: theme.into(),
        double_escape: double_escape.into(),
        quiet_startup,
        autocomplete_max_visible,
        tree_filter_mode: tree_filter_mode.into(),
        mermaid_mode: mermaid_mode.into(),
        enable_analytics,
        ..InteractiveSettingsConfig::default()
    })
}
