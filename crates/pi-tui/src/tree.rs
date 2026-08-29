//! Session tree selector matching TS `tree-selector.ts`.

use crate::render::Component;

pub const FILTER_MODES: &[&str] = &["default", "no-tools", "user-only", "labeled-only", "all"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterMode {
    Default,
    NoTools,
    UserOnly,
    LabeledOnly,
    All,
}

impl FilterMode {
    pub fn parse(value: &str) -> Self {
        match value {
            "no-tools" => Self::NoTools,
            "user-only" => Self::UserOnly,
            "labeled-only" => Self::LabeledOnly,
            "all" => Self::All,
            _ => Self::Default,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::NoTools => "no-tools",
            Self::UserOnly => "user-only",
            Self::LabeledOnly => "labeled-only",
            Self::All => "all",
        }
    }

    pub fn cycle(self, delta: isize) -> Self {
        let modes = [
            Self::Default,
            Self::NoTools,
            Self::UserOnly,
            Self::LabeledOnly,
            Self::All,
        ];
        let index = modes.iter().position(|mode| *mode == self).unwrap_or(0) as isize;
        let next = (index + delta).rem_euclid(modes.len() as isize) as usize;
        modes[next]
    }

    pub fn status_label(self) -> &'static str {
        match self {
            Self::Default => "",
            Self::NoTools => " [no-tools]",
            Self::UserOnly => " [user]",
            Self::LabeledOnly => " [labeled]",
            Self::All => " [all]",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTreeEntry {
    pub id: String,
    pub parent_id: Option<String>,
    pub entry_type: String,
    pub role: Option<String>,
    pub content: Option<serde_json::Value>,
    pub stop_reason: Option<String>,
    pub error_message: Option<String>,
    pub tool_call_id: Option<String>,
    pub tool_name: Option<String>,
    pub command: Option<String>,
    pub custom_type: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
    pub label: Option<String>,
    pub name: Option<String>,
    pub tokens_before: Option<u64>,
    pub summary: Option<String>,
}

impl SessionTreeEntry {
    pub fn message(id: &str, parent_id: Option<&str>, role: &str, text: &str) -> Self {
        Self {
            id: id.into(),
            parent_id: parent_id.map(str::to_string),
            entry_type: "message".into(),
            role: Some(role.into()),
            content: Some(serde_json::json!([{"type":"text","text":text}])),
            stop_reason: None,
            error_message: None,
            tool_call_id: None,
            tool_name: None,
            command: None,
            custom_type: None,
            model_id: None,
            thinking_level: None,
            label: None,
            name: None,
            tokens_before: None,
            summary: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTreeNode {
    pub entry: SessionTreeEntry,
    pub children: Vec<SessionTreeNode>,
    pub label: Option<String>,
    pub label_timestamp: Option<String>,
}

#[derive(Debug, Clone)]
struct GutterInfo {
    position: usize,
    show: bool,
}

#[derive(Debug, Clone)]
struct FlatNode {
    node: SessionTreeNode,
    indent: usize,
    show_connector: bool,
    is_last: bool,
    gutters: Vec<GutterInfo>,
    is_virtual_root_child: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeAction {
    None,
    Select(String),
    Cancel,
    Copy(Option<String>),
    LabelChange { id: String, label: Option<String> },
}

#[derive(Debug, Clone)]
pub struct TreeSelector {
    flat: Vec<FlatNode>,
    filtered: Vec<FlatNode>,
    pub selected: usize,
    pub filter_mode: FilterMode,
    pub search_query: String,
    current_leaf_id: Option<String>,
    max_visible: usize,
    folded: Vec<String>,
    multiple_roots: bool,
    show_label_timestamps: bool,
    label_edit: Option<(String, String)>,
    visible_parent: std::collections::HashMap<String, Option<String>>,
    visible_children: std::collections::HashMap<Option<String>, Vec<String>>,
}

impl TreeSelector {
    pub fn new(
        roots: Vec<SessionTreeNode>,
        current_leaf_id: Option<String>,
        max_visible: usize,
        filter_mode: FilterMode,
    ) -> Self {
        let mut selector = Self {
            flat: Vec::new(),
            filtered: Vec::new(),
            selected: 0,
            filter_mode,
            search_query: String::new(),
            current_leaf_id,
            max_visible: max_visible.max(5),
            folded: Vec::new(),
            multiple_roots: roots.len() > 1,
            show_label_timestamps: false,
            label_edit: None,
            visible_parent: std::collections::HashMap::new(),
            visible_children: std::collections::HashMap::new(),
        };
        selector.flat = flatten_tree(&roots, selector.current_leaf_id.as_deref());
        selector.apply_filter();
        selector
    }

    pub fn selected_id(&self) -> Option<String> {
        self.filtered
            .get(self.selected)
            .map(|node| node.node.entry.id.clone())
    }

    pub fn selected_copy_text(&self) -> Option<String> {
        self.filtered
            .get(self.selected)
            .and_then(|node| entry_copy_text(&node.node))
    }

    pub fn update_node_label(&mut self, entry_id: &str, label: Option<String>) {
        for flat in self.flat.iter_mut().chain(self.filtered.iter_mut()) {
            if flat.node.entry.id == entry_id {
                flat.node.label = label.clone();
                flat.node.label_timestamp = label.as_ref().map(|_| "now".into());
            }
        }
    }

    pub fn handle_key(&mut self, data: &str) -> TreeAction {
        if let Some((id, mut buffer)) = self.label_edit.take() {
            return match data {
                "\r" | "\n" => {
                    let label = buffer.trim();
                    let label = if label.is_empty() {
                        None
                    } else {
                        Some(label.to_string())
                    };
                    self.update_node_label(&id, label.clone());
                    TreeAction::LabelChange { id, label }
                }
                "\x1b" => TreeAction::None,
                "\x7f" | "\x08" => {
                    buffer.pop();
                    self.label_edit = Some((id, buffer));
                    TreeAction::None
                }
                other if !other.chars().any(char::is_control) => {
                    buffer.push_str(other);
                    self.label_edit = Some((id, buffer));
                    TreeAction::None
                }
                _ => {
                    self.label_edit = Some((id, buffer));
                    TreeAction::None
                }
            };
        }
        match data {
            "\x1b[A" | "k" => {
                self.move_sel(-1);
                TreeAction::None
            }
            "\x1b[B" | "j" => {
                self.move_sel(1);
                TreeAction::None
            }
            "\r" | "\n" => self
                .selected_id()
                .map(TreeAction::Select)
                .unwrap_or(TreeAction::None),
            "\x1b" => {
                if !self.search_query.is_empty() {
                    self.search_query.clear();
                    self.folded.clear();
                    self.apply_filter();
                    TreeAction::None
                } else {
                    TreeAction::Cancel
                }
            }
            "\x04" => {
                self.filter_mode = FilterMode::Default;
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x14" => {
                self.filter_mode = if self.filter_mode == FilterMode::NoTools {
                    FilterMode::Default
                } else {
                    FilterMode::NoTools
                };
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x15" => {
                self.filter_mode = if self.filter_mode == FilterMode::UserOnly {
                    FilterMode::Default
                } else {
                    FilterMode::UserOnly
                };
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x0c" => {
                self.filter_mode = if self.filter_mode == FilterMode::LabeledOnly {
                    FilterMode::Default
                } else {
                    FilterMode::LabeledOnly
                };
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x01" => {
                self.filter_mode = if self.filter_mode == FilterMode::All {
                    FilterMode::Default
                } else {
                    FilterMode::All
                };
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x0f" => {
                self.filter_mode = self.filter_mode.cycle(1);
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x1b[79;6u" | "\x0f\x10" => {
                self.filter_mode = self.filter_mode.cycle(-1);
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            "\x1b[1;5D" | "\x1b[1;3D" | "\x1b[5D" | "\x1b[3D" => self.fold_or_up(),
            "\x1b[1;5C" | "\x1b[1;3C" | "\x1b[5C" | "\x1b[3C" => self.unfold_or_down(),
            "\x18" => TreeAction::Copy(self.selected_copy_text()),
            "L" => {
                if let Some(id) = self.selected_id() {
                    let current = self
                        .filtered
                        .get(self.selected)
                        .and_then(|node| node.node.label.clone())
                        .unwrap_or_default();
                    self.label_edit = Some((id, current));
                }
                TreeAction::None
            }
            "T" => {
                self.show_label_timestamps = !self.show_label_timestamps;
                TreeAction::None
            }
            "\x1b[D" | "\x1b[5~" => {
                self.selected = self.selected.saturating_sub(self.max_visible);
                TreeAction::None
            }
            "\x1b[C" | "\x1b[6~" => {
                if !self.filtered.is_empty() {
                    self.selected = (self.selected + self.max_visible).min(self.filtered.len() - 1);
                }
                TreeAction::None
            }
            "\x7f" | "\x08" => {
                self.search_query.pop();
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            other
                if !other.chars().any(|ch| {
                    let code = ch as u32;
                    code < 32 || code == 0x7f || (0x80..=0x9f).contains(&code)
                }) =>
            {
                self.search_query.push_str(other);
                self.folded.clear();
                self.apply_filter();
                TreeAction::None
            }
            _ => TreeAction::None,
        }
    }

    fn fold_or_up(&mut self) -> TreeAction {
        let current_id = self.selected_id();
        if let Some(id) = current_id {
            if self.is_foldable(&id) && !self.folded.iter().any(|item| item == &id) {
                self.folded.push(id);
                self.apply_filter();
                return TreeAction::None;
            }
        }
        self.selected = self.find_branch_segment_start(-1);
        TreeAction::None
    }

    fn unfold_or_down(&mut self) -> TreeAction {
        let current_id = self.selected_id();
        if let Some(id) = &current_id {
            if let Some(index) = self.folded.iter().position(|item| item == id) {
                self.folded.remove(index);
                self.apply_filter();
                return TreeAction::None;
            }
        }
        self.selected = self.find_branch_segment_start(1);
        TreeAction::None
    }

    fn is_foldable(&self, entry_id: &str) -> bool {
        let children = self.visible_children.get(&Some(entry_id.to_string()));
        if children.is_none_or(|items| items.is_empty()) {
            return false;
        }
        match self.visible_parent.get(entry_id) {
            None | Some(None) => true,
            Some(Some(parent)) => self
                .visible_children
                .get(&Some(parent.clone()))
                .is_some_and(|siblings| siblings.len() > 1),
        }
    }

    fn find_branch_segment_start(&self, direction: isize) -> usize {
        let Some(selected_id) = self.selected_id() else {
            return self.selected;
        };
        let index_by_id: std::collections::HashMap<String, usize> = self
            .filtered
            .iter()
            .enumerate()
            .map(|(i, node)| (node.node.entry.id.clone(), i))
            .collect();
        let mut current_id = selected_id;
        if direction > 0 {
            loop {
                let children = self
                    .visible_children
                    .get(&Some(current_id.clone()))
                    .cloned()
                    .unwrap_or_default();
                if children.is_empty() {
                    return *index_by_id.get(&current_id).unwrap_or(&self.selected);
                }
                if children.len() > 1 {
                    return *index_by_id.get(&children[0]).unwrap_or(&self.selected);
                }
                current_id = children[0].clone();
            }
        }
        loop {
            let parent = self.visible_parent.get(&current_id).cloned().flatten();
            let Some(parent_id) = parent else {
                return *index_by_id.get(&current_id).unwrap_or(&self.selected);
            };
            let children = self
                .visible_children
                .get(&Some(parent_id.clone()))
                .cloned()
                .unwrap_or_default();
            if children.len() > 1 {
                if let Some(&segment) = index_by_id.get(&current_id) {
                    if segment < self.selected {
                        return segment;
                    }
                }
            }
            current_id = parent_id;
        }
    }

    fn rebuild_visible_maps(&mut self) {
        self.visible_parent.clear();
        self.visible_children.clear();
        self.visible_children.insert(None, Vec::new());
        let visible: std::collections::HashSet<String> = self
            .filtered
            .iter()
            .map(|node| node.node.entry.id.clone())
            .collect();
        let entry_parent: std::collections::HashMap<String, Option<String>> = self
            .flat
            .iter()
            .map(|node| {
                (
                    node.node.entry.id.clone(),
                    node.node.entry.parent_id.clone(),
                )
            })
            .collect();
        for flat in &self.filtered {
            let id = flat.node.entry.id.clone();
            let mut ancestor = entry_parent.get(&id).cloned().flatten();
            while let Some(current) = ancestor.clone() {
                if visible.contains(&current) {
                    break;
                }
                ancestor = entry_parent.get(&current).cloned().flatten();
            }
            self.visible_parent.insert(id.clone(), ancestor.clone());
            self.visible_children.entry(ancestor).or_default().push(id);
        }
    }

    fn move_sel(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            return;
        }
        let len = self.filtered.len() as isize;
        self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
    }

    fn apply_filter(&mut self) {
        let last_id = self
            .filtered
            .get(self.selected)
            .map(|node| node.node.entry.id.clone());
        let search_tokens: Vec<String> = self
            .search_query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_string)
            .collect();
        self.filtered = self
            .flat
            .iter()
            .filter(|flat| passes_filter(flat, self.filter_mode, self.current_leaf_id.as_deref()))
            .filter(|flat| {
                if search_tokens.is_empty() {
                    return true;
                }
                let text = searchable_text(&flat.node).to_lowercase();
                search_tokens.iter().all(|token| text.contains(token))
            })
            .cloned()
            .collect();
        if !self.folded.is_empty() {
            let skip = folded_descendants(&self.flat, &self.folded);
            self.filtered
                .retain(|flat| !skip.iter().any(|id| id == &flat.node.entry.id));
        }
        recalculate_visual(&mut self.filtered, &self.flat);
        self.rebuild_visible_maps();
        self.multiple_roots = self
            .visible_children
            .get(&None)
            .is_some_and(|roots| roots.len() > 1);
        if let Some(id) = last_id {
            if let Some(index) = self
                .filtered
                .iter()
                .position(|node| node.node.entry.id == id)
            {
                self.selected = index;
            } else if self.selected >= self.filtered.len() {
                self.selected = self.filtered.len().saturating_sub(1);
            }
        } else if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
        }
    }

    fn status_line(&self) -> String {
        let extra = if self.show_label_timestamps {
            " [+label time]"
        } else {
            ""
        };
        if self.filtered.is_empty() {
            format!("  (0/0){}{extra}", self.filter_mode.status_label())
        } else {
            format!(
                "  ({}/{}){}{extra}",
                self.selected + 1,
                self.filtered.len(),
                self.filter_mode.status_label()
            )
        }
    }
}

fn passes_filter(flat: &FlatNode, mode: FilterMode, current_leaf: Option<&str>) -> bool {
    let entry = &flat.node.entry;
    let is_current = current_leaf == Some(entry.id.as_str());
    if entry.entry_type == "message"
        && entry.role.as_deref() == Some("assistant")
        && !is_current
        && !has_text_content(&entry.content)
    {
        let error_or_aborted = entry
            .stop_reason
            .as_deref()
            .is_some_and(|reason| reason != "stop" && reason != "toolUse");
        if !error_or_aborted {
            return false;
        }
    }
    let is_settings = matches!(
        entry.entry_type.as_str(),
        "label" | "custom" | "model_change" | "thinking_level_change" | "session_info"
    );
    match mode {
        FilterMode::UserOnly => {
            entry.entry_type == "message" && entry.role.as_deref() == Some("user")
        }
        FilterMode::NoTools => {
            !(is_settings
                || (entry.entry_type == "message" && entry.role.as_deref() == Some("toolResult")))
        }
        FilterMode::LabeledOnly => flat.node.label.is_some(),
        FilterMode::All => true,
        FilterMode::Default => !is_settings,
    }
}

fn has_text_content(content: &Option<serde_json::Value>) -> bool {
    let Some(content) = content else {
        return false;
    };
    if let Some(text) = content.as_str() {
        return !text.trim().is_empty();
    }
    if let Some(items) = content.as_array() {
        return items.iter().any(|item| {
            item.get("type").and_then(|t| t.as_str()) == Some("text")
                && item
                    .get("text")
                    .and_then(|t| t.as_str())
                    .is_some_and(|text| !text.trim().is_empty())
        });
    }
    false
}

fn extract_content(content: &Option<serde_json::Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.chars().take(200).collect();
    }
    if let Some(items) = content.as_array() {
        let text: String = items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
        return text.chars().take(200).collect();
    }
    String::new()
}

fn extract_full_content(content: &Option<serde_json::Value>) -> String {
    let Some(content) = content else {
        return String::new();
    };
    if let Some(text) = content.as_str() {
        return text.to_string();
    }
    if let Some(items) = content.as_array() {
        return items
            .iter()
            .filter_map(|item| {
                if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                    item.get("text").and_then(|t| t.as_str())
                } else {
                    None
                }
            })
            .collect();
    }
    String::new()
}

fn entry_copy_text(node: &SessionTreeNode) -> Option<String> {
    let entry = &node.entry;
    let text = match entry.entry_type.as_str() {
        "message" if entry.role.as_deref() == Some("bashExecution") => entry.command.clone(),
        "message" => {
            let content = extract_full_content(&entry.content);
            if content.is_empty() && entry.role.as_deref() == Some("assistant") {
                entry.error_message.clone()
            } else {
                Some(content)
            }
        }
        "custom_message" => Some(extract_full_content(&entry.content)),
        "compaction" | "branch_summary" => entry.summary.clone(),
        _ => None,
    };
    text.filter(|value| !value.trim().is_empty())
}

fn slice_by_column(text: &str, start: usize, width: usize) -> String {
    let mut out = String::new();
    let mut col = 0usize;
    for ch in text.chars() {
        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
        if col + w <= start {
            col += w;
            continue;
        }
        if crate::render::visible_width(&out) + w > width {
            break;
        }
        out.push(ch);
        col += w;
    }
    out
}

fn searchable_text(node: &SessionTreeNode) -> String {
    let mut parts = Vec::new();
    if let Some(label) = &node.label {
        parts.push(label.clone());
    }
    let entry = &node.entry;
    match entry.entry_type.as_str() {
        "message" => {
            if let Some(role) = &entry.role {
                parts.push(role.clone());
            }
            parts.push(extract_content(&entry.content));
            if let Some(command) = &entry.command {
                parts.push(command.clone());
            }
        }
        "custom_message" => {
            if let Some(custom) = &entry.custom_type {
                parts.push(custom.clone());
            }
            parts.push(extract_content(&entry.content));
        }
        "compaction" => parts.push("compaction".into()),
        "branch_summary" => {
            parts.push("branch summary".into());
            if let Some(summary) = &entry.summary {
                parts.push(summary.clone());
            }
        }
        "session_info" => {
            parts.push("title".into());
            if let Some(name) = &entry.name {
                parts.push(name.clone());
            }
        }
        "model_change" => {
            parts.push("model".into());
            if let Some(id) = &entry.model_id {
                parts.push(id.clone());
            }
        }
        "thinking_level_change" => {
            parts.push("thinking".into());
            if let Some(level) = &entry.thinking_level {
                parts.push(level.clone());
            }
        }
        "custom" => {
            parts.push("custom".into());
            if let Some(custom) = &entry.custom_type {
                parts.push(custom.clone());
            }
        }
        "label" => {
            parts.push("label".into());
            if let Some(label) = &entry.label {
                parts.push(label.clone());
            }
        }
        _ => {}
    }
    parts.join(" ")
}

fn display_text(node: &SessionTreeNode) -> String {
    let entry = &node.entry;
    let normalize = |s: String| s.replace(['\n', '\t'], " ").trim().to_string();
    match entry.entry_type.as_str() {
        "message" => match entry.role.as_deref() {
            Some("user") => format!("user: {}", normalize(extract_content(&entry.content))),
            Some("assistant") => {
                let text = normalize(extract_content(&entry.content));
                if !text.is_empty() {
                    format!("assistant: {text}")
                } else if entry.stop_reason.as_deref() == Some("aborted") {
                    "assistant: (aborted)".into()
                } else if let Some(err) = &entry.error_message {
                    format!(
                        "assistant: {}",
                        normalize(err.clone()).chars().take(80).collect::<String>()
                    )
                } else {
                    "assistant: (no content)".into()
                }
            }
            Some("toolResult") => format!("[{}]", entry.tool_name.as_deref().unwrap_or("tool")),
            Some("bashExecution") => {
                format!(
                    "[bash]: {}",
                    normalize(entry.command.clone().unwrap_or_default())
                )
            }
            Some(role) => format!("[{role}]"),
            None => String::new(),
        },
        "custom_message" => format!(
            "[{}]: {}",
            entry.custom_type.as_deref().unwrap_or("custom"),
            normalize(extract_content(&entry.content))
        ),
        "compaction" => format!(
            "[compaction: {}k tokens]",
            entry.tokens_before.unwrap_or(0) / 1000
        ),
        "branch_summary" => format!(
            "[branch summary]: {}",
            normalize(entry.summary.clone().unwrap_or_default())
        ),
        "model_change" => format!("[model: {}]", entry.model_id.as_deref().unwrap_or("")),
        "thinking_level_change" => {
            format!(
                "[thinking: {}]",
                entry.thinking_level.as_deref().unwrap_or("")
            )
        }
        "custom" => format!("[custom: {}]", entry.custom_type.as_deref().unwrap_or("")),
        "label" => format!("[label: {}]", entry.label.as_deref().unwrap_or("(cleared)")),
        "session_info" => match &entry.name {
            Some(name) if !name.is_empty() => format!("[title: {name}]"),
            _ => "[title: empty]".into(),
        },
        _ => String::new(),
    }
}

fn flatten_tree(roots: &[SessionTreeNode], current_leaf: Option<&str>) -> Vec<FlatNode> {
    let mut result = Vec::new();
    let contains_active = contains_active_map(roots, current_leaf);
    let multiple_roots = roots.len() > 1;
    let mut ordered: Vec<&SessionTreeNode> = roots.iter().collect();
    ordered.sort_by_key(|node| {
        std::cmp::Reverse(*contains_active.get(&node.entry.id).unwrap_or(&false))
    });
    #[derive(Clone)]
    struct StackItem {
        node: SessionTreeNode,
        indent: usize,
        just_branched: bool,
        show_connector: bool,
        is_last: bool,
        gutters: Vec<GutterInfo>,
        is_virtual_root_child: bool,
    }
    let mut stack = Vec::new();
    for (i, node) in ordered.iter().rev().enumerate() {
        let is_last = i == 0;
        stack.push(StackItem {
            node: (*node).clone(),
            indent: if multiple_roots { 1 } else { 0 },
            just_branched: multiple_roots,
            show_connector: multiple_roots,
            is_last,
            gutters: Vec::new(),
            is_virtual_root_child: multiple_roots,
        });
    }
    while let Some(item) = stack.pop() {
        result.push(FlatNode {
            node: SessionTreeNode {
                entry: item.node.entry.clone(),
                children: Vec::new(),
                label: item.node.label.clone(),
                label_timestamp: item.node.label_timestamp.clone(),
            },
            indent: item.indent,
            show_connector: item.show_connector,
            is_last: item.is_last,
            gutters: item.gutters.clone(),
            is_virtual_root_child: item.is_virtual_root_child,
        });
        let children = item.node.children;
        let multiple_children = children.len() > 1;
        let mut ordered_children = children;
        ordered_children.sort_by_key(|child| {
            std::cmp::Reverse(*contains_active.get(&child.entry.id).unwrap_or(&false))
        });
        let child_indent = if multiple_children || (item.just_branched && item.indent > 0) {
            item.indent + 1
        } else {
            item.indent
        };
        let connector_displayed = item.show_connector && !item.is_virtual_root_child;
        let current_display_indent = if multiple_roots {
            item.indent.saturating_sub(1)
        } else {
            item.indent
        };
        let connector_position = current_display_indent.saturating_sub(1);
        let child_gutters = if connector_displayed {
            let mut next = item.gutters;
            next.push(GutterInfo {
                position: connector_position,
                show: !item.is_last,
            });
            next
        } else {
            item.gutters
        };
        for (i, child) in ordered_children.into_iter().rev().enumerate() {
            let child_is_last = i == 0;
            stack.push(StackItem {
                node: child,
                indent: child_indent,
                just_branched: multiple_children,
                show_connector: multiple_children,
                is_last: child_is_last,
                gutters: child_gutters.clone(),
                is_virtual_root_child: false,
            });
        }
    }
    result
}

fn contains_active_map(
    roots: &[SessionTreeNode],
    leaf_id: Option<&str>,
) -> std::collections::HashMap<String, bool> {
    let mut all = Vec::new();
    let mut stack: Vec<&SessionTreeNode> = roots.iter().collect();
    while let Some(node) = stack.pop() {
        all.push(node);
        for child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    let mut contains = std::collections::HashMap::new();
    for node in all.into_iter().rev() {
        let mut has = leaf_id == Some(node.entry.id.as_str());
        for child in &node.children {
            if *contains.get(&child.entry.id).unwrap_or(&false) {
                has = true;
            }
        }
        contains.insert(node.entry.id.clone(), has);
    }
    contains
}

fn folded_descendants(flat: &[FlatNode], folded: &[String]) -> Vec<String> {
    let mut skip = Vec::new();
    for node in flat {
        if let Some(parent) = &node.node.entry.parent_id {
            if folded.contains(parent) || skip.iter().any(|id| id == parent) {
                skip.push(node.node.entry.id.clone());
            }
        }
    }
    skip
}

fn recalculate_visual(filtered: &mut [FlatNode], _flat: &[FlatNode]) {
    let len = filtered.len();
    let indents: Vec<usize> = filtered.iter().map(|node| node.indent).collect();
    for (index, node) in filtered.iter_mut().enumerate() {
        node.show_connector = node.indent > 0;
        node.is_last = index + 1 == len
            || indents
                .get(index + 1)
                .is_some_and(|next| *next <= node.indent);
    }
}

pub fn build_session_tree(entries: Vec<SessionTreeEntry>) -> Vec<SessionTreeNode> {
    let mut labels = std::collections::HashMap::new();
    for entry in &entries {
        if entry.entry_type == "label" {
            if let Some(target) = entry
                .content
                .as_ref()
                .and_then(|value| value.get("targetId").or_else(|| value.get("target_id")))
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .or_else(|| entry.parent_id.clone())
            {
                if let Some(label) = &entry.label {
                    labels.insert(target, label.clone());
                }
            }
        }
    }
    let mut children: std::collections::HashMap<Option<String>, Vec<String>> =
        std::collections::HashMap::new();
    let mut by_id = std::collections::HashMap::new();
    for entry in entries {
        children
            .entry(entry.parent_id.clone())
            .or_default()
            .push(entry.id.clone());
        by_id.insert(entry.id.clone(), entry);
    }
    fn build(
        id: &str,
        by_id: &std::collections::HashMap<String, SessionTreeEntry>,
        children: &std::collections::HashMap<Option<String>, Vec<String>>,
        labels: &std::collections::HashMap<String, String>,
    ) -> Option<SessionTreeNode> {
        let entry = by_id.get(id)?.clone();
        let kids = children
            .get(&Some(id.to_string()))
            .into_iter()
            .flatten()
            .filter_map(|child| build(child, by_id, children, labels))
            .collect();
        Some(SessionTreeNode {
            label: labels.get(id).cloned(),
            label_timestamp: None,
            children: kids,
            entry,
        })
    }
    children
        .get(&None)
        .into_iter()
        .flatten()
        .filter_map(|id| build(id, &by_id, &children, &labels))
        .collect()
}

const TREE_GUTTER_WIDTH: usize = 2;
const MIN_VISIBLE_ANCHOR_CONTENT_WIDTH: usize = 4;
const MAX_VISIBLE_ANCHOR_CONTENT_WIDTH: usize = 20;
const MIN_ANCHOR_CONTEXT_WIDTH: usize = 2;
const MAX_ANCHOR_CONTEXT_WIDTH: usize = 12;

fn render_horizontal_viewport(
    gutter: &str,
    body: &str,
    anchor_col: usize,
    is_selected: bool,
    width: usize,
) -> String {
    let gutter_width = crate::render::visible_width(gutter).max(TREE_GUTTER_WIDTH);
    let viewport_width = width.saturating_sub(gutter_width);
    let body_width = crate::render::visible_width(body);
    let max_scroll = body_width.saturating_sub(viewport_width);
    let mut scroll = 0;
    if is_selected && max_scroll > 0 {
        let min_visible = MAX_VISIBLE_ANCHOR_CONTENT_WIDTH
            .min(MIN_VISIBLE_ANCHOR_CONTENT_WIDTH.max(viewport_width / 3));
        if anchor_col > viewport_width.saturating_sub(min_visible) {
            let context =
                MAX_ANCHOR_CONTEXT_WIDTH.min(MIN_ANCHOR_CONTEXT_WIDTH.max(viewport_width / 4));
            scroll = max_scroll.min(anchor_col.saturating_sub(context));
        }
    }
    let clipped = if scroll > 0 || body_width > viewport_width {
        slice_by_column(body, scroll, viewport_width)
    } else {
        body.to_string()
    };
    format!("{gutter}{clipped}")
}

impl Component for TreeSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let mut lines = vec![
            "  Session Tree".into(),
            "  ↑↓ move · ←→ page · branch · L label · ctrl+x copy · filters · cycle".into(),
        ];
        if self.search_query.is_empty() {
            lines.push("  Type to search:".into());
        } else {
            lines.push(format!("  Type to search: {}", self.search_query));
        }
        if self.filtered.is_empty() {
            lines.push("  No entries found".into());
            lines.push(self.status_line());
            return lines;
        }
        let start = self
            .selected
            .saturating_sub(self.max_visible / 2)
            .min(self.filtered.len().saturating_sub(self.max_visible));
        let end = (start + self.max_visible).min(self.filtered.len());
        for (offset, flat) in self.filtered[start..end].iter().enumerate() {
            let index = start + offset;
            let cursor = if index == self.selected { "› " } else { "  " };
            let display_indent = if self.multiple_roots {
                flat.indent.saturating_sub(1)
            } else {
                flat.indent
            };
            let connector = flat.show_connector && !flat.is_virtual_root_child;
            let connector_position = if connector {
                display_indent.saturating_sub(1) as isize
            } else {
                -1
            };
            let total_chars = display_indent * 3;
            let mut prefix = String::new();
            for i in 0..total_chars {
                let level = i / 3;
                let pos_in_level = i % 3;
                if let Some(gutter) = flat.gutters.iter().find(|gutter| gutter.position == level) {
                    prefix.push(if pos_in_level == 0 && gutter.show {
                        '│'
                    } else {
                        ' '
                    });
                } else if connector && level as isize == connector_position {
                    prefix.push(match pos_in_level {
                        0 => {
                            if flat.is_last {
                                '└'
                            } else {
                                '├'
                            }
                        }
                        1 => {
                            if self.folded.iter().any(|id| id == &flat.node.entry.id) {
                                '⊞'
                            } else if self.is_foldable(&flat.node.entry.id) {
                                '⊟'
                            } else {
                                '─'
                            }
                        }
                        _ => ' ',
                    });
                } else {
                    prefix.push(' ');
                }
            }
            let label = flat
                .node
                .label
                .as_ref()
                .map(|label| format!("[{label}] "))
                .unwrap_or_default();
            let timestamp = if self.show_label_timestamps && flat.node.label.is_some() {
                flat.node
                    .label_timestamp
                    .as_deref()
                    .map(|value| format!("{value} "))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let content = display_text(&flat.node);
            let body = format!("{prefix}{label}{timestamp}{content}");
            let viewport = render_horizontal_viewport(
                cursor,
                &body,
                crate::render::visible_width(&prefix),
                index == self.selected,
                width,
            );
            lines.push(viewport);
        }
        if let Some((_, buffer)) = &self.label_edit {
            lines.push("  Label (empty to remove):".into());
            lines.push(format!("  {buffer}"));
            lines.push("  enter save  escape cancel".into());
        }
        lines.push(self.status_line());
        lines
            .into_iter()
            .map(|line| {
                if crate::render::visible_width(&line) <= width {
                    line
                } else {
                    slice_by_column(&line, 0, width)
                }
            })
            .collect()
    }

    fn handle_input(&mut self, data: &str) {
        let _ = self.handle_key(data);
    }

    fn invalidate(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tree() -> Vec<SessionTreeNode> {
        let user = SessionTreeEntry::message("u1", None, "user", "hello");
        let assistant = SessionTreeEntry::message("a1", Some("u1"), "assistant", "hi");
        let sibling = SessionTreeEntry::message("u2", Some("u1"), "user", "again");
        let tool = SessionTreeEntry {
            role: Some("toolResult".into()),
            tool_name: Some("bash".into()),
            ..SessionTreeEntry::message("t1", Some("a1"), "toolResult", "")
        };
        let settings = SessionTreeEntry {
            entry_type: "model_change".into(),
            model_id: Some("gpt".into()),
            role: None,
            content: None,
            ..SessionTreeEntry::message("m1", Some("a1"), "assistant", "")
        };
        build_session_tree(vec![user, assistant, sibling, tool, settings])
    }

    #[test]
    fn filter_modes_and_tree_chrome_match_ts() {
        let mut roots = sample_tree();
        if let Some(child) = roots
            .get_mut(0)
            .and_then(|root| root.children.iter_mut().find(|node| node.entry.id == "u2"))
        {
            child.label = Some("checkpoint".into());
        }
        let mut tree = TreeSelector::new(roots, Some("u2".into()), 12, FilterMode::Default);
        let rendered = tree.render(80).join("\n");
        assert!(rendered.contains("Session Tree"));
        assert!(rendered.contains("user: hello"));
        assert!(rendered.contains("assistant: hi"));
        assert!(rendered.contains("├─") || rendered.contains("└─"));
        assert!(!rendered.contains("[model:"));
        tree.handle_key("\x14");
        assert_eq!(tree.filter_mode, FilterMode::NoTools);
        assert!(tree.render(80).join("\n").contains("[no-tools]"));
        assert!(
            !tree.render(80).join("\n").contains("[bash]")
                && !tree.render(80).join("\n").contains("[tool]")
        );
        tree.handle_key("\x15");
        assert_eq!(tree.filter_mode, FilterMode::UserOnly);
        let user_only = tree.render(80).join("\n");
        assert!(user_only.contains("[user]"));
        assert!(user_only.contains("user: hello"));
        assert!(!user_only.contains("assistant:"));
        tree.handle_key("\x0c");
        assert_eq!(tree.filter_mode, FilterMode::LabeledOnly);
        assert!(tree.render(80).join("\n").contains("[labeled]"));
        tree.handle_key("\x01");
        assert_eq!(tree.filter_mode, FilterMode::All);
        let all = tree.render(80).join("\n");
        assert!(all.contains("[all]"));
        assert!(all.contains("[model: gpt]"));
        tree.handle_key("\x04");
        assert_eq!(tree.filter_mode, FilterMode::Default);
        tree.handle_key("\x0f");
        assert_eq!(tree.filter_mode, FilterMode::NoTools);
        tree.search_query.clear();
        tree.handle_key("hello");
        assert!(tree.render(80).join("\n").contains("Type to search: hello"));
        assert_eq!(tree.handle_key("\x1b"), TreeAction::None);
        assert!(tree.search_query.is_empty());
        assert_eq!(tree.handle_key("\x1b"), TreeAction::Cancel);
    }

    #[test]
    fn fold_copy_label_and_viewport_match_ts() {
        let mut roots = sample_tree();
        if let Some(child) = roots
            .get_mut(0)
            .and_then(|root| root.children.iter_mut().find(|node| node.entry.id == "u2"))
        {
            child.label = Some("checkpoint".into());
        }
        let mut tree = TreeSelector::new(roots, Some("u2".into()), 12, FilterMode::Default);
        tree.selected = 0;
        tree.handle_key("\x1b[1;5D");
        assert!(
            tree.folded.contains(&"u1".to_string()),
            "root with visible children should fold: {:?}",
            tree.folded
        );
        tree.handle_key("\x1b[1;5C");
        tree.selected = tree
            .filtered
            .iter()
            .position(|node| node.node.entry.id == "u1")
            .unwrap_or(0);
        assert_eq!(
            tree.handle_key("\x18"),
            TreeAction::Copy(Some("hello".into()))
        );
        tree.handle_key("L");
        assert!(tree
            .render(80)
            .join("\n")
            .contains("Label (empty to remove):"));
        tree.handle_key("saved");
        assert_eq!(
            tree.handle_key("\r"),
            TreeAction::LabelChange {
                id: "u1".into(),
                label: Some("saved".into()),
            }
        );
        tree.handle_key("T");
        assert!(tree.render(80).join("\n").contains("[+label time]"));
        let narrow = tree.render(12);
        assert!(narrow
            .iter()
            .all(|line| crate::render::visible_width(line) <= 12
                || line.contains("Session Tree")
                || line.contains("move")));
    }
}
