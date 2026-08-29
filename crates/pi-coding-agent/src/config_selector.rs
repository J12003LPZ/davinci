use crate::package_manager::{
    resolve_current, PathMetadata, ResolvedPaths, ResolvedResource, RESOURCE_TYPES,
};
use crate::settings::{agent_dir, package_source_string, settings_path, SettingsDocument};
use pi_tui::component::Component;
use pi_tui::diff::visible_width;
use pi_tui::keys::{read_key, Key};
use pi_tui::screen::Tui;
use pi_tui::terminal::{
    disable_raw_input, enable_raw_input, enter_alt_screen, leave_alt_screen, TuiMode,
};
use pi_tui::widgets::Input;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteScope {
    Global,
    Project,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverrideState {
    Inherit,
    Load,
    Unload,
}

#[derive(Debug, Clone)]
pub struct ResourceItem {
    pub path: PathBuf,
    pub enabled: bool,
    pub metadata: PathMetadata,
    pub resource_type: String,
    pub display_name: String,
}

#[derive(Debug, Clone)]
pub struct ResourceSubgroup {
    pub resource_type: String,
    pub label: String,
    pub items: Vec<ResourceItem>,
}

#[derive(Debug, Clone)]
pub struct ResourceGroup {
    pub label: String,
    pub scope: String,
    pub origin: String,
    pub source: String,
    pub subgroups: Vec<ResourceSubgroup>,
}

fn format_base_dir(base_dir: &Path) -> String {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
    let display = if base_dir == home.as_path() {
        "~".into()
    } else if let Ok(rest) = base_dir.strip_prefix(&home) {
        format!("~/{}", rest.to_string_lossy().replace('\\', "/"))
    } else {
        base_dir.to_string_lossy().replace('\\', "/")
    };
    if display.ends_with('/') {
        display
    } else {
        format!("{display}/")
    }
}

fn group_label(metadata: &PathMetadata, agent: &Path) -> String {
    if metadata.origin == "package" {
        return format!("{} ({})", metadata.source, metadata.scope);
    }
    if metadata.source == "auto" {
        if let Some(base) = &metadata.base_dir {
            return if metadata.scope == "user" {
                format!("User ({})", format_base_dir(base))
            } else {
                format!("Project ({})", format_base_dir(base))
            };
        }
        return if metadata.scope == "user" {
            format!("User ({})", format_base_dir(agent))
        } else {
            "Project (.pi/)".into()
        };
    }
    if metadata.scope == "user" {
        "User settings".into()
    } else {
        "Project settings".into()
    }
}

fn display_name(path: &Path, resource_type: &str) -> String {
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let parent = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    if resource_type == "extensions" && parent != "extensions" {
        format!("{parent}/{file_name}")
    } else if resource_type == "skills" && file_name == "SKILL.md" {
        parent.to_string()
    } else {
        file_name.to_string()
    }
}

pub fn build_groups(resolved: &ResolvedPaths, agent: &Path) -> Vec<ResourceGroup> {
    let mut groups: Vec<ResourceGroup> = Vec::new();
    let add =
        |groups: &mut Vec<ResourceGroup>, resources: &[ResolvedResource], resource_type: &str| {
            for res in resources {
                let key = format!(
                    "{}:{}:{}:{}",
                    res.metadata.origin,
                    res.metadata.scope,
                    res.metadata.source,
                    res.metadata
                        .base_dir
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                );
                let label = group_label(&res.metadata, agent);
                if !groups.iter().any(|g| {
                    g.origin == res.metadata.origin
                        && g.scope == res.metadata.scope
                        && g.source == res.metadata.source
                        && g.label == label
                }) {
                    groups.push(ResourceGroup {
                        label: label.clone(),
                        scope: res.metadata.scope.clone(),
                        origin: res.metadata.origin.clone(),
                        source: res.metadata.source.clone(),
                        subgroups: Vec::new(),
                    });
                }
                let group = groups
                    .iter_mut()
                    .find(|g| {
                        g.origin == res.metadata.origin
                            && g.scope == res.metadata.scope
                            && g.source == res.metadata.source
                            && g.label == label
                    })
                    .expect("group exists");
                if !group
                    .subgroups
                    .iter()
                    .any(|sg| sg.resource_type == resource_type)
                {
                    let type_label = match resource_type {
                        "extensions" => "Extensions",
                        "skills" => "Skills",
                        "prompts" => "Prompts",
                        "themes" => "Themes",
                        other => other,
                    };
                    group.subgroups.push(ResourceSubgroup {
                        resource_type: resource_type.into(),
                        label: type_label.into(),
                        items: Vec::new(),
                    });
                }
                let subgroup = group
                    .subgroups
                    .iter_mut()
                    .find(|sg| sg.resource_type == resource_type)
                    .expect("subgroup exists");
                let _ = key;
                subgroup.items.push(ResourceItem {
                    path: res.path.clone(),
                    enabled: res.enabled,
                    metadata: res.metadata.clone(),
                    resource_type: resource_type.into(),
                    display_name: display_name(&res.path, resource_type),
                });
            }
        };
    add(&mut groups, &resolved.extensions, "extensions");
    add(&mut groups, &resolved.skills, "skills");
    add(&mut groups, &resolved.prompts, "prompts");
    add(&mut groups, &resolved.themes, "themes");
    groups.sort_by(|a, b| {
        let origin = match (a.origin.as_str(), b.origin.as_str()) {
            ("package", "package") => std::cmp::Ordering::Equal,
            ("package", _) => std::cmp::Ordering::Less,
            (_, "package") => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        };
        if origin != std::cmp::Ordering::Equal {
            return origin;
        }
        match (a.scope.as_str(), b.scope.as_str()) {
            ("user", "project") => std::cmp::Ordering::Less,
            ("project", "user") => std::cmp::Ordering::Greater,
            _ => a.source.cmp(&b.source),
        }
    });
    let type_order = |t: &str| match t {
        "extensions" => 0,
        "skills" => 1,
        "prompts" => 2,
        "themes" => 3,
        _ => 4,
    };
    for group in &mut groups {
        group
            .subgroups
            .sort_by_key(|sg| type_order(&sg.resource_type));
        for subgroup in &mut group.subgroups {
            subgroup
                .items
                .sort_by(|a, b| a.display_name.cmp(&b.display_name));
        }
    }
    groups
}

fn posix_rel(base: &Path, path: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn package_resource_pattern(item: &ResourceItem) -> String {
    let base = item
        .metadata
        .base_dir
        .clone()
        .unwrap_or_else(|| item.path.parent().unwrap_or(Path::new(".")).to_path_buf());
    posix_rel(&base, &item.path)
}

fn top_level_pattern(item: &ResourceItem, agent: &Path, cwd: &Path) -> String {
    let base = item.metadata.base_dir.clone().unwrap_or_else(|| {
        if item.metadata.scope == "project" {
            cwd.join(".pi")
        } else {
            agent.to_path_buf()
        }
    });
    posix_rel(&base, &item.path)
}

fn strip_override(entry: &str) -> &str {
    entry
        .strip_prefix('!')
        .or_else(|| entry.strip_prefix('+'))
        .or_else(|| entry.strip_prefix('-'))
        .unwrap_or(entry)
}

pub fn toggle_package_resource(item: &ResourceItem, enabled: bool, local: bool) {
    let mut doc = SettingsDocument::load(&settings_path(local));
    let mut packages = doc.packages();
    let idx = packages.iter().position(|pkg| {
        package_source_string(pkg).as_deref() == Some(item.metadata.source.as_str())
    });
    let Some(idx) = idx else {
        return;
    };
    if packages[idx].is_string() {
        packages[idx] = json!({ "source": packages[idx] });
    }
    let pattern = package_resource_pattern(item);
    let current = packages[idx]
        .get(&item.resource_type)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut updated: Vec<Value> = current
        .into_iter()
        .filter(|p| p.as_str().is_none_or(|s| strip_override(s) != pattern))
        .collect();
    updated.push(json!(format!(
        "{}{pattern}",
        if enabled { "+" } else { "-" }
    )));
    if let Some(obj) = packages[idx].as_object_mut() {
        obj.insert(item.resource_type.clone(), json!(updated));
        let has_filters = RESOURCE_TYPES.iter().any(|k| obj.get(*k).is_some());
        if !has_filters {
            packages[idx] = json!(obj
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or_default());
        }
    }
    doc.set_packages(packages);
    doc.save(&settings_path(local));
}

pub fn toggle_top_level_resource(item: &ResourceItem, enabled: bool, agent: &Path, cwd: &Path) {
    let local = item.metadata.scope == "project";
    let mut doc = SettingsDocument::load(&settings_path(local));
    let pattern = top_level_pattern(item, agent, cwd);
    let mut updated: Vec<String> = doc
        .resource_paths(&item.resource_type)
        .into_iter()
        .filter(|p| strip_override(p) != pattern)
        .collect();
    updated.push(format!("{}{pattern}", if enabled { "+" } else { "-" }));
    doc.set_resource_paths(&item.resource_type, updated);
    doc.save(&settings_path(local));
}

pub fn toggle_resource(item: &ResourceItem, enabled: bool) {
    if item.metadata.origin == "package" {
        toggle_package_resource(item, enabled, item.metadata.scope == "project");
    } else {
        toggle_top_level_resource(
            item,
            enabled,
            &agent_dir(),
            &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        );
    }
}

#[derive(Debug, Clone)]
enum FlatEntry {
    Group { label: String, scope: String },
    Subgroup { label: String },
    Item(ResourceItem),
}

pub struct ConfigSelector {
    pub write_scope: WriteScope,
    pub project_mode_available: bool,
    pub groups: Vec<ResourceGroup>,
    global_groups: Vec<ResourceGroup>,
    project_groups: Vec<ResourceGroup>,
    search: Input,
    filtered: Vec<FlatEntry>,
    pub selected: usize,
    max_visible: usize,
    inherited_enabled: BTreeMap<String, bool>,
}

impl ConfigSelector {
    pub fn from_scoped(
        global: &ResolvedPaths,
        project: &ResolvedPaths,
        write_scope: WriteScope,
        project_mode_available: bool,
    ) -> Self {
        let agent = agent_dir();
        let global_groups = build_groups(global, &agent);
        let project_groups = build_groups(project, &agent);
        let inherited_enabled = inherited_enabled_map(&global_groups);
        let groups = if write_scope == WriteScope::Project {
            project_groups.clone()
        } else {
            global_groups.clone()
        };
        let mut selector = Self {
            write_scope,
            project_mode_available,
            groups,
            global_groups,
            project_groups,
            search: Input::new("Search resources"),
            filtered: Vec::new(),
            selected: 0,
            max_visible: 16,
            inherited_enabled,
        };
        selector.rebuild_filter();
        selector
    }

    fn current_groups(&self) -> &[ResourceGroup] {
        if self.write_scope == WriteScope::Project {
            &self.project_groups
        } else {
            &self.global_groups
        }
    }

    fn item_key(item: &ResourceItem) -> String {
        format!("{}:{}", item.resource_type, item.path.display())
    }

    fn build_flat(&self) -> Vec<FlatEntry> {
        let mut flat = Vec::new();
        for group in self.current_groups() {
            flat.push(FlatEntry::Group {
                label: group.label.clone(),
                scope: group.scope.clone(),
            });
            for subgroup in &group.subgroups {
                flat.push(FlatEntry::Subgroup {
                    label: subgroup.label.clone(),
                });
                for item in &subgroup.items {
                    flat.push(FlatEntry::Item(item.clone()));
                }
            }
        }
        flat
    }

    fn rebuild_filter(&mut self) {
        let query = self.search.value.to_ascii_lowercase();
        let flat = self.build_flat();
        if query.trim().is_empty() {
            self.filtered = flat;
        } else {
            let matching: Vec<ResourceItem> = flat
                .iter()
                .filter_map(|e| match e {
                    FlatEntry::Item(item)
                        if item.display_name.to_ascii_lowercase().contains(&query)
                            || item.resource_type.to_ascii_lowercase().contains(&query)
                            || item
                                .path
                                .to_string_lossy()
                                .to_ascii_lowercase()
                                .contains(&query) =>
                    {
                        Some(item.clone())
                    }
                    _ => None,
                })
                .collect();
            let mut out = Vec::new();
            for entry in flat {
                match &entry {
                    FlatEntry::Group { label, .. } => {
                        if matching.iter().any(|item| {
                            self.current_groups().iter().any(|g| {
                                g.label == *label
                                    && g.subgroups
                                        .iter()
                                        .any(|sg| sg.items.iter().any(|i| i.path == item.path))
                            })
                        }) {
                            out.push(entry);
                        }
                    }
                    FlatEntry::Subgroup { label, .. } => {
                        if matching.iter().any(|item| {
                            self.current_groups().iter().any(|g| {
                                g.subgroups.iter().any(|sg| {
                                    sg.label == *label
                                        && sg.items.iter().any(|i| i.path == item.path)
                                })
                            })
                        }) {
                            out.push(entry);
                        }
                    }
                    FlatEntry::Item(item) => {
                        if matching.iter().any(|m| m.path == item.path) {
                            out.push(entry);
                        }
                    }
                }
            }
            self.filtered = out;
        }
        self.select_first_item();
        self.groups = self.current_groups().to_vec();
    }

    fn select_first_item(&mut self) {
        self.selected = self
            .filtered
            .iter()
            .position(|e| matches!(e, FlatEntry::Item(_)))
            .unwrap_or(0);
    }

    fn find_next_item(&self, from: usize, direction: isize) -> usize {
        let mut idx = from as isize + direction;
        while idx >= 0 && (idx as usize) < self.filtered.len() {
            if matches!(self.filtered[idx as usize], FlatEntry::Item(_)) {
                return idx as usize;
            }
            idx += direction;
        }
        from
    }

    pub fn selected_item(&self) -> Option<&ResourceItem> {
        match self.filtered.get(self.selected) {
            Some(FlatEntry::Item(item)) => Some(item),
            _ => None,
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.selected = self.find_next_item(self.selected, delta);
    }

    pub fn page_selection(&mut self, direction: isize) {
        if self.filtered.is_empty() {
            return;
        }
        if direction < 0 {
            let mut target = self.selected.saturating_sub(self.max_visible);
            while target < self.filtered.len()
                && !matches!(self.filtered[target], FlatEntry::Item(_))
            {
                target += 1;
            }
            if target < self.filtered.len() {
                self.selected = target;
            }
        } else {
            let mut target = (self.selected + self.max_visible).min(self.filtered.len() - 1);
            while target > 0 && !matches!(self.filtered[target], FlatEntry::Item(_)) {
                target -= 1;
            }
            self.selected = target;
        }
    }

    pub fn set_write_scope(&mut self, write_scope: WriteScope) {
        self.write_scope = write_scope;
        self.rebuild_filter();
    }

    pub fn apply_toggle(&mut self) -> Option<(ResourceItem, bool)> {
        let item = self.selected_item()?.clone();
        if self.write_scope == WriteScope::Project {
            let state = next_override_state(
                &item,
                project_override_state(&item),
                self.inherited_enabled
                    .get(&Self::item_key(&item))
                    .copied()
                    .unwrap_or(item.enabled),
            );
            if !set_project_resource_override(&item, state) {
                return None;
            }
            let enabled = match state {
                OverrideState::Load => true,
                OverrideState::Unload => false,
                OverrideState::Inherit => self
                    .inherited_enabled
                    .get(&Self::item_key(&item))
                    .copied()
                    .unwrap_or(item.enabled),
            };
            self.update_item_enabled(&item, enabled);
            return Some((item, enabled));
        }
        if item.metadata.scope != "user" {
            return None;
        }
        let enabled = !item.enabled;
        toggle_resource(&item, enabled);
        self.update_item_enabled(&item, enabled);
        Some((item, enabled))
    }

    fn update_item_enabled(&mut self, item: &ResourceItem, enabled: bool) {
        for groups in [
            &mut self.groups,
            &mut self.global_groups,
            &mut self.project_groups,
        ] {
            for group in groups.iter_mut() {
                for subgroup in &mut group.subgroups {
                    if let Some(found) = subgroup
                        .items
                        .iter_mut()
                        .find(|i| i.path == item.path && i.resource_type == item.resource_type)
                    {
                        found.enabled = enabled;
                    }
                }
            }
        }
        if let Some(FlatEntry::Item(found)) = self.filtered.get_mut(self.selected) {
            found.enabled = enabled;
        }
    }

    pub fn handle_input(&mut self, data: &str) -> bool {
        match data {
            "up" | "k" | "\u{1b}[A" => self.move_selection(-1),
            "down" | "j" | "\u{1b}[B" => self.move_selection(1),
            "pageup" | "\u{1b}[5~" => self.page_selection(-1),
            "pagedown" | "\u{1b}[6~" => self.page_selection(1),
            "escape" | "q" | "\u{1b}" | "ctrl+c" => return true,
            "tab" | "\t" if self.project_mode_available => {
                self.set_write_scope(if self.write_scope == WriteScope::Global {
                    WriteScope::Project
                } else {
                    WriteScope::Global
                });
            }
            " " | "enter" | "\r" | "\n" => {
                let _ = self.apply_toggle();
            }
            "backspace" | "\u{7f}" | "\u{8}" => {
                self.search.value.pop();
                self.rebuild_filter();
            }
            other
                if !other.is_empty()
                    && !other.starts_with('\u{1b}')
                    && other != " "
                    && other.chars().all(|c| !c.is_control()) =>
            {
                self.search.value.push_str(other);
                self.rebuild_filter();
            }
            _ => {}
        }
        false
    }

    fn checkbox(&self, item: &ResourceItem) -> String {
        if self.write_scope == WriteScope::Project {
            return match project_override_state(item) {
                OverrideState::Load => theme_fg("success", "[+]"),
                OverrideState::Unload => theme_fg("warning", "[-]"),
                OverrideState::Inherit => theme_fg("dim", if item.enabled { "[x]" } else { "[ ]" }),
            };
        }
        if item.enabled {
            theme_fg("success", "[x]")
        } else {
            theme_fg("dim", "[ ]")
        }
    }

    fn item_suffix(&self, item: &ResourceItem) -> String {
        if self.write_scope != WriteScope::Project {
            return String::new();
        }
        match project_override_state(item) {
            OverrideState::Load => theme_fg("muted", "  project load"),
            OverrideState::Unload => theme_fg("muted", "  project unload"),
            OverrideState::Inherit if item.metadata.scope == "user" => {
                theme_fg("dim", "  inherited global")
            }
            _ => String::new(),
        }
    }

    pub fn handle_key(&mut self, key: &Key) -> bool {
        self.handle_input(&key_to_config_input(key))
    }

    pub fn set_terminal_rows(&mut self, rows: usize) {
        self.max_visible = rows.saturating_sub(8).max(5);
    }
}

fn inherited_enabled_map(groups: &[ResourceGroup]) -> BTreeMap<String, bool> {
    let mut map = BTreeMap::new();
    for group in groups {
        for subgroup in &group.subgroups {
            for item in &subgroup.items {
                map.insert(
                    format!("{}:{}", item.resource_type, item.path.display()),
                    item.enabled,
                );
            }
        }
    }
    map
}

fn project_override_state(item: &ResourceItem) -> OverrideState {
    let doc = SettingsDocument::load(&settings_path(true));
    if item.metadata.origin == "package" {
        let Some(pkg) = doc.packages().into_iter().find(|pkg| {
            package_source_string(pkg).as_deref() == Some(item.metadata.source.as_str())
        }) else {
            return OverrideState::Inherit;
        };
        if !pkg.is_object() {
            return OverrideState::Inherit;
        }
        let Some(entries) = pkg.get(&item.resource_type).and_then(|v| v.as_array()) else {
            return OverrideState::Inherit;
        };
        if entries.is_empty() && pkg.get("autoload").and_then(|v| v.as_bool()) != Some(false) {
            return OverrideState::Unload;
        }
        let pattern = package_resource_pattern(item);
        let mut state = OverrideState::Inherit;
        for entry in entries {
            let Some(text) = entry.as_str() else {
                continue;
            };
            if strip_override(text) != pattern {
                continue;
            }
            state = if text.starts_with('!') || text.starts_with('-') {
                OverrideState::Unload
            } else {
                OverrideState::Load
            };
        }
        return state;
    }
    let pattern = top_level_pattern(
        item,
        &agent_dir(),
        &std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    );
    let mut state = OverrideState::Inherit;
    for entry in doc.resource_paths(&item.resource_type) {
        if strip_override(&entry) != pattern
            && strip_override(&entry) != item.path.to_string_lossy()
        {
            continue;
        }
        state = if entry.starts_with('!') || entry.starts_with('-') {
            OverrideState::Unload
        } else {
            OverrideState::Load
        };
    }
    state
}

fn set_project_resource_override(item: &ResourceItem, state: OverrideState) -> bool {
    if item.metadata.origin == "package" {
        set_project_package_override(item, state)
    } else {
        set_project_top_level_override(item, state)
    }
}

fn set_project_package_override(item: &ResourceItem, state: OverrideState) -> bool {
    let mut doc = SettingsDocument::load(&settings_path(true));
    let mut packages = doc.packages();
    let mut idx = packages.iter().position(|pkg| {
        package_source_string(pkg).as_deref() == Some(item.metadata.source.as_str())
    });
    if idx.is_none() {
        if state == OverrideState::Inherit {
            return false;
        }
        packages.push(json!({
            "source": item.metadata.source,
            "autoload": false
        }));
        idx = Some(packages.len() - 1);
    }
    let idx = idx.expect("package index");
    if packages[idx].is_string() {
        packages[idx] = json!({ "source": packages[idx] });
    }
    let pattern = package_resource_pattern(item);
    let current = packages[idx]
        .get(&item.resource_type)
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut updated: Vec<Value> = current
        .into_iter()
        .filter(|p| p.as_str().is_none_or(|s| strip_override(s) != pattern))
        .collect();
    if state != OverrideState::Inherit {
        updated.push(json!(format!(
            "{}{pattern}",
            if state == OverrideState::Load {
                "+"
            } else {
                "-"
            }
        )));
    }
    if let Some(obj) = packages[idx].as_object_mut() {
        if updated.is_empty() {
            obj.remove(&item.resource_type);
        } else {
            obj.insert(item.resource_type.clone(), json!(updated));
        }
        let has_filters = RESOURCE_TYPES.iter().any(|k| obj.get(*k).is_some());
        if !has_filters {
            if obj.get("autoload").and_then(|v| v.as_bool()) == Some(false) {
                packages.remove(idx);
            } else {
                packages[idx] = json!(obj
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default());
            }
        }
    }
    doc.set_packages(packages);
    doc.save(&settings_path(true));
    true
}

fn set_project_top_level_override(item: &ResourceItem, state: OverrideState) -> bool {
    let mut doc = SettingsDocument::load(&settings_path(true));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let pattern = if item.metadata.scope == "user" {
        item.path.to_string_lossy().into_owned()
    } else {
        top_level_pattern(item, &agent_dir(), &cwd)
    };
    let mut updated: Vec<String> = doc
        .resource_paths(&item.resource_type)
        .into_iter()
        .filter(|entry| {
            let target = strip_override(entry);
            !(entry.starts_with('!') || entry.starts_with('+') || entry.starts_with('-'))
                || (target != pattern && target != item.path.to_string_lossy())
        })
        .collect();
    if state != OverrideState::Inherit {
        if item.metadata.scope == "user" && !updated.iter().any(|e| e == &pattern) {
            updated.push(pattern.clone());
        }
        updated.push(format!(
            "{}{pattern}",
            if state == OverrideState::Load {
                "+"
            } else {
                "-"
            }
        ));
    }
    doc.set_resource_paths(&item.resource_type, updated);
    doc.save(&settings_path(true));
    true
}

fn next_override_state(
    _item: &ResourceItem,
    state: OverrideState,
    inherited_enabled: bool,
) -> OverrideState {
    match state {
        OverrideState::Inherit => {
            if inherited_enabled {
                OverrideState::Unload
            } else {
                OverrideState::Load
            }
        }
        OverrideState::Unload => {
            if inherited_enabled {
                OverrideState::Load
            } else {
                OverrideState::Inherit
            }
        }
        OverrideState::Load => {
            if inherited_enabled {
                OverrideState::Inherit
            } else {
                OverrideState::Unload
            }
        }
    }
}

impl Component for ConfigSelector {
    fn render(&self, width: usize) -> Vec<String> {
        let title = theme_bold(if self.write_scope == WriteScope::Project {
            "Project Local Resources"
        } else {
            "Global Resources"
        });
        let sep = theme_fg("muted", " · ");
        let switch = if self.project_mode_available {
            format!("tab switch mode{sep}")
        } else {
            String::new()
        };
        let action = if self.write_scope == WriteScope::Project {
            "space cycle inherit/+/-"
        } else {
            "space toggle"
        };
        let hint = format!("{switch}{action}{sep}esc close");
        let spacing = width
            .saturating_sub(visible_width(&title) + visible_width(&hint))
            .max(1);
        let scope_hint = if self.write_scope == WriteScope::Project {
            theme_fg(
                "muted",
                ".pi/settings.json · inherited global resources are dimmed",
            )
        } else {
            theme_fg("muted", "~/.pi/agent/settings.json")
        };
        let mut inner = vec![format!("{title}{:spacing$}{hint}", ""), scope_hint];
        inner.extend(self.search.render(width));
        inner.push(String::new());
        if self.filtered.is_empty() {
            inner.push(theme_fg("muted", "  No resources found"));
            return wrap_process_terminal_chrome(inner, width);
        }
        let start = self
            .selected
            .saturating_sub(self.max_visible / 2)
            .min(self.filtered.len().saturating_sub(self.max_visible));
        let end = (start + self.max_visible).min(self.filtered.len());
        for (i, entry) in self.filtered.iter().enumerate().take(end).skip(start) {
            match entry {
                FlatEntry::Group { label, scope } => {
                    let inherited = self.write_scope == WriteScope::Project && scope == "user";
                    let suffix = if inherited {
                        " · inherited global"
                    } else {
                        ""
                    };
                    inner.push(theme_fg(
                        if inherited { "dim" } else { "accent" },
                        &format!("  {label}{suffix}"),
                    ));
                }
                FlatEntry::Subgroup { label, .. } => {
                    inner.push(theme_fg("muted", &format!("    {label}")));
                }
                FlatEntry::Item(item) => {
                    let cursor = if i == self.selected { "> " } else { "  " };
                    let name = if self.write_scope == WriteScope::Project
                        && item.metadata.scope == "user"
                    {
                        theme_fg("dim", &item.display_name)
                    } else {
                        item.display_name.clone()
                    };
                    inner.push(format!(
                        "{cursor}    {} {}{}",
                        self.checkbox(item),
                        name,
                        self.item_suffix(item)
                    ));
                }
            }
        }
        if start > 0 || end < self.filtered.len() {
            let item_count = self
                .filtered
                .iter()
                .filter(|e| matches!(e, FlatEntry::Item(_)))
                .count();
            let current = self
                .filtered
                .iter()
                .take(self.selected + 1)
                .filter(|e| matches!(e, FlatEntry::Item(_)))
                .count()
                .max(1);
            inner.push(theme_fg("dim", &format!("  ({current}/{item_count})")));
        }
        wrap_process_terminal_chrome(inner, width)
    }
}

fn hex_rgb(hex: &str) -> (u8, u8, u8) {
    let h = hex.trim_start_matches('#');
    if h.len() < 6 {
        return (0, 0, 0);
    }
    (
        u8::from_str_radix(&h[0..2], 16).unwrap_or(0),
        u8::from_str_radix(&h[2..4], 16).unwrap_or(0),
        u8::from_str_radix(&h[4..6], 16).unwrap_or(0),
    )
}

fn theme_color_hex(name: &str) -> &'static str {
    match name {
        "accent" => "#8abeb7",
        "border" => "#5f87ff",
        "success" => "#b5bd68",
        "warning" => "#ffff00",
        "muted" => "#808080",
        "dim" => "#666666",
        _ => "#d4d4d4",
    }
}

fn theme_fg(name: &str, text: &str) -> String {
    let (r, g, b) = hex_rgb(theme_color_hex(name));
    format!("\u{1b}[38;2;{r};{g};{b}m{text}\u{1b}[39m")
}

fn theme_bold(text: &str) -> String {
    format!("\u{1b}[1m{text}\u{1b}[22m")
}

fn dynamic_border_line(width: usize) -> String {
    theme_fg("border", &"─".repeat(width.max(1)))
}

fn wrap_process_terminal_chrome(inner: Vec<String>, width: usize) -> Vec<String> {
    let border = dynamic_border_line(width);
    let mut lines = vec![String::new(), border.clone(), String::new()];
    lines.extend(inner);
    lines.push(String::new());
    lines.push(border);
    lines
}

fn key_to_config_input(key: &Key) -> String {
    match key {
        Key::Up => "up".into(),
        Key::Down => "down".into(),
        Key::Tab => "tab".into(),
        Key::Escape => "escape".into(),
        Key::Enter => "enter".into(),
        Key::Backspace => "backspace".into(),
        Key::Ctrl('c') => "ctrl+c".into(),
        Key::Char(c) => c.to_string(),
        Key::Unknown(raw) if raw.contains("[5~") => "pageup".into(),
        Key::Unknown(raw) if raw.contains("[6~") => "pagedown".into(),
        Key::Unknown(raw) => raw.clone(),
        other => format!("{other:?}"),
    }
}

fn terminal_size() -> (usize, usize) {
    if let Ok(out) = std::process::Command::new("stty").arg("size").output() {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            let mut parts = text.split_whitespace();
            if let (Some(rows), Some(cols)) = (parts.next(), parts.next()) {
                if let (Ok(rows), Ok(cols)) = (rows.parse::<usize>(), cols.parse::<usize>()) {
                    if rows > 0 && cols > 0 {
                        return (cols, rows);
                    }
                }
            }
        }
    }
    (80, 24)
}

pub fn render_config(write_scope: WriteScope, project_trusted: bool) -> String {
    let global = resolve_current(false);
    let project = if project_trusted {
        resolve_current(true)
    } else {
        global.clone()
    };
    ConfigSelector::from_scoped(&global, &project, write_scope, project_trusted)
        .render(80)
        .join("\n")
        + "\n"
}

pub fn run_interactive_config(write_scope: WriteScope, project_trusted: bool) -> String {
    let global = resolve_current(false);
    let project = if project_trusted {
        resolve_current(true)
    } else {
        global.clone()
    };
    let mut selector = ConfigSelector::from_scoped(&global, &project, write_scope, project_trusted);
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    if !stdin.is_terminal() || !stdout.is_terminal() {
        return render_config(write_scope, project_trusted);
    }
    let (cols, rows) = terminal_size();
    selector.set_terminal_rows(rows);
    let _ = enter_alt_screen(&mut stdout);
    let _ = enable_raw_input();
    let mut tui = Tui::new(TuiMode::Fullscreen, cols, rows);
    let mut raw_stdin = stdin;
    loop {
        tui.clear_children();
        tui.add_child_lines(selector.render(cols));
        let frame = tui.render_now(false);
        let _ = write!(stdout, "{frame}");
        let _ = stdout.flush();
        match read_key(&mut raw_stdin) {
            Ok(Some(key)) => {
                if selector.handle_key(&key) {
                    break;
                }
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }
    let _ = disable_raw_input();
    let _ = leave_alt_screen(&mut stdout);
    let _ = stdout.flush();
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package_manager::{install_and_persist, resolve_resources};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn config_lists_package_resources_and_toggles_filter() {
        let dir = tempdir().unwrap();
        let fixture = dir.path().join("fixture").join("npm").join("pi-config");
        fs::create_dir_all(fixture.join("extensions")).unwrap();
        fs::write(
            fixture.join("extensions").join("index.ts"),
            "export default function () {}",
        )
        .unwrap();
        let agent = dir.path().join("agent");
        fs::create_dir_all(&agent).unwrap();
        let _lock = crate::settings::test_env_lock();
        let previous_agent = std::env::var("PI_CODING_AGENT_DIR").ok();
        let previous_fixture = std::env::var("PI_PACKAGE_FIXTURE").ok();
        let previous_cwd = std::env::current_dir().ok();
        std::env::set_var("PI_CODING_AGENT_DIR", &agent);
        std::env::set_var("PI_PACKAGE_FIXTURE", dir.path().join("fixture"));
        std::env::set_var("PI_DISABLE_NETWORK", "1");
        let _ = std::env::set_current_dir(dir.path());
        install_and_persist("npm:pi-config", false).unwrap();
        let rendered = render_config(WriteScope::Global, false);
        assert!(rendered.contains("Global Resources"));
        assert!(rendered.contains("npm:pi-config (user)"));
        assert!(rendered.contains("Extensions"));
        assert!(rendered.contains("[x] index.ts") || rendered.contains("index.ts"));
        let resolved = resolve_resources(&agent, dir.path(), false);
        let groups = build_groups(&resolved, &agent);
        let item = groups
            .iter()
            .flat_map(|g| g.subgroups.iter().flat_map(|sg| sg.items.iter()))
            .find(|i| i.resource_type == "extensions")
            .cloned()
            .unwrap();
        toggle_resource(&item, false);
        let doc = SettingsDocument::load(&agent.join("settings.json"));
        let pkg = &doc.packages()[0];
        assert!(pkg.is_object());
        let filters = pkg
            .get("extensions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(filters
            .iter()
            .any(|v| v.as_str() == Some("-extensions/index.ts")));

        let mut selector =
            ConfigSelector::from_scoped(&resolved, &resolved, WriteScope::Global, true);
        selector.handle_input("zzz-no-such-resource");
        assert!(selector
            .render(80)
            .iter()
            .any(|l| l.contains("No resources found")));
        selector.search.value.clear();
        selector.handle_input("index");
        assert!(selector.render(80).iter().any(|l| l.contains("index.ts")));
        selector.handle_input("pagedown");
        selector.handle_input("pageup");
        selector.set_write_scope(WriteScope::Project);
        let project = render_config(WriteScope::Project, true);
        assert!(project.contains("Project Local Resources"));
        assert!(project.contains("space cycle inherit/+/-"));
        assert!(project.contains('─'));
        assert!(project.contains("\u{1b}[38;2;"));
        assert!(selector.handle_key(&Key::Escape));
        selector.handle_input(" ");
        let project_doc = SettingsDocument::load(&dir.path().join(".pi").join("settings.json"));
        let project_pkg = project_doc
            .packages()
            .into_iter()
            .find(|pkg| package_source_string(pkg).as_deref() == Some("npm:pi-config"));
        assert!(project_pkg.is_some());
        match previous_agent {
            Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
            None => std::env::remove_var("PI_CODING_AGENT_DIR"),
        }
        match previous_fixture {
            Some(v) => std::env::set_var("PI_PACKAGE_FIXTURE", v),
            None => std::env::remove_var("PI_PACKAGE_FIXTURE"),
        }
        if let Some(cwd) = previous_cwd {
            let _ = std::env::set_current_dir(cwd);
        }
    }
}
