//! Startup loaded-resource listing matching TS `showLoadedResources`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::render::Component;
use crate::themes::Theme;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceScope {
    Project,
    User,
    Path,
}

impl ResourceScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
            Self::Path => "path",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceSourceInfo {
    pub source: String,
    pub scope: ResourceScope,
    pub base_dir: Option<String>,
}

impl Default for ResourceSourceInfo {
    fn default() -> Self {
        Self {
            source: "local".into(),
            scope: ResourceScope::Project,
            base_dir: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedResourceItem {
    pub path: String,
    pub compact_label: String,
    pub expanded_label: String,
    pub source: ResourceSourceInfo,
}

#[derive(Debug, Clone)]
pub struct ExpandableText {
    pub collapsed: String,
    pub expanded_text: String,
    pub expanded: bool,
}

impl ExpandableText {
    pub fn set_expanded(&mut self, expanded: bool) {
        self.expanded = expanded;
    }

    pub fn current(&self) -> &str {
        if self.expanded {
            &self.expanded_text
        } else {
            &self.collapsed
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct LoadedResources {
    pub sections: Vec<ExpandableText>,
    pub diagnostics: Vec<String>,
}

impl LoadedResources {
    pub fn set_expanded(&mut self, expanded: bool) {
        for section in &mut self.sections {
            section.set_expanded(expanded);
        }
    }

    pub fn add_section(
        &mut self,
        theme: &Theme,
        name: &str,
        compact_body: &str,
        expanded_body: &str,
        expanded: bool,
    ) {
        let header = section_header(theme, name);
        self.sections.push(ExpandableText {
            collapsed: format!("{header}\n{compact_body}"),
            expanded_text: format!("{header}\n{expanded_body}"),
            expanded,
        });
    }

    pub fn add_named_section(
        &mut self,
        theme: &Theme,
        name: &str,
        items: &[LoadedResourceItem],
        expanded: bool,
    ) {
        if items.is_empty() {
            return;
        }
        let compact = theme.fg(
            "dim",
            &format_compact_list(items.iter().map(|item| item.compact_label.as_str()), true),
        );
        let expanded_body = format_scope_groups(theme, items, |item| item.expanded_label.clone());
        self.add_section(theme, name, &compact, &expanded_body, expanded);
    }

    pub fn add_diagnostic(&mut self, theme: &Theme, title: &str, body: &str) {
        let header = theme.fg("warning", &format!("[{title}]"));
        self.diagnostics.push(format!("{header}\n{body}"));
    }
}

impl Component for LoadedResources {
    fn render(&self, _width: usize) -> Vec<String> {
        let mut lines = Vec::new();
        for section in &self.sections {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(section.current().lines().map(str::to_string));
        }
        for diagnostic in &self.diagnostics {
            if !lines.is_empty() {
                lines.push(String::new());
            }
            lines.extend(diagnostic.lines().map(str::to_string));
        }
        lines
    }

    fn invalidate(&mut self) {}
}

pub fn section_header(theme: &Theme, name: &str) -> String {
    theme.fg("mdHeading", &format!("[{name}]"))
}

pub fn format_display_path(path: &str, home: &str) -> String {
    if !home.is_empty() && (path == home || path.starts_with(&format!("{home}/"))) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    }
}

pub fn format_context_path(path: &str, cwd: &str, home: &str) -> String {
    let cwd = cwd.trim_end_matches('/');
    if path == cwd {
        return ".".into();
    }
    if let Some(rest) = path.strip_prefix(&format!("{cwd}/")) {
        return rest.to_string();
    }
    format_display_path(path, home)
}

pub fn format_compact_list<'a, I>(items: I, sort: bool) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut labels: Vec<String> = items
        .into_iter()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect();
    if sort {
        labels.sort();
    }
    format!("  {}", labels.join(", "))
}

pub fn infer_source_info(path: &str, cwd: &str, agent_dir: &str) -> ResourceSourceInfo {
    let normalized = path.replace('\\', "/");
    let cwd = cwd.replace('\\', "/");
    let agent_dir = agent_dir.replace('\\', "/");
    let project_root = format!("{}/.pi", cwd.trim_end_matches('/'));
    let scope = if !agent_dir.is_empty()
        && (normalized == agent_dir || normalized.starts_with(&format!("{agent_dir}/")))
    {
        ResourceScope::User
    } else if normalized == project_root || normalized.starts_with(&format!("{project_root}/")) {
        ResourceScope::Project
    } else {
        ResourceScope::Path
    };
    if let Some(pkg) = npm_source(&normalized) {
        return ResourceSourceInfo {
            source: format!("npm:{pkg}"),
            scope,
            base_dir: None,
        };
    }
    if let Some(repo) = git_source(&normalized) {
        return ResourceSourceInfo {
            source: format!("git:{repo}"),
            scope,
            base_dir: None,
        };
    }
    ResourceSourceInfo {
        source: "local".into(),
        scope,
        base_dir: None,
    }
}

pub fn format_scope_groups(
    theme: &Theme,
    items: &[LoadedResourceItem],
    format_path: impl Fn(&LoadedResourceItem) -> String,
) -> String {
    type ScopeGroup<'a> = (
        Vec<&'a LoadedResourceItem>,
        BTreeMap<String, Vec<&'a LoadedResourceItem>>,
    );
    let mut groups: BTreeMap<ResourceScope, ScopeGroup<'_>> = BTreeMap::new();
    for item in items {
        let entry = groups
            .entry(item.source.scope)
            .or_insert_with(|| (Vec::new(), BTreeMap::new()));
        if item.source.source.starts_with("npm:") || item.source.source.starts_with("git:") {
            entry
                .1
                .entry(item.source.source.clone())
                .or_default()
                .push(item);
        } else {
            entry.0.push(item);
        }
    }
    let mut lines = Vec::new();
    for scope in [
        ResourceScope::Project,
        ResourceScope::User,
        ResourceScope::Path,
    ] {
        let Some((paths, packages)) = groups.get(&scope) else {
            continue;
        };
        if paths.is_empty() && packages.is_empty() {
            continue;
        }
        lines.push(format!("  {}", theme.fg("accent", scope.as_str())));
        let mut sorted_paths = paths.clone();
        sorted_paths.sort_by(|a, b| a.path.cmp(&b.path));
        for item in sorted_paths {
            lines.push(theme.fg("dim", &format!("    {}", format_path(item))));
        }
        for (source, package_items) in packages {
            lines.push(format!("    {}", theme.fg("mdLink", source)));
            let mut sorted = package_items.clone();
            sorted.sort_by(|a, b| a.path.cmp(&b.path));
            for item in sorted {
                lines.push(theme.fg("dim", &format!("      {}", format_path(item))));
            }
        }
    }
    lines.join("\n")
}

pub fn format_collision_diagnostic(
    theme: &Theme,
    name: &str,
    winner: &str,
    losers: &[&str],
) -> String {
    let mut lines = vec![theme.fg("warning", &format!("  \"{name}\" collision:"))];
    lines.push(theme.fg("dim", &format!("    {} {winner}", theme.fg("success", "✓"))));
    for loser in losers {
        lines.push(theme.fg(
            "dim",
            &format!("    {} {loser} (skipped)", theme.fg("warning", "✗")),
        ));
    }
    lines.join("\n")
}

pub fn collect_name_collisions(items: &[(String, String)]) -> Vec<(String, String, Vec<&str>)> {
    let mut groups: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (name, path) in items {
        groups.entry(name.as_str()).or_default().push(path.as_str());
    }
    groups
        .into_iter()
        .filter(|(_, paths)| paths.len() > 1)
        .map(|(name, paths)| {
            let winner = paths[0].to_string();
            let losers = paths[1..].to_vec();
            (name.to_string(), winner, losers)
        })
        .collect()
}

pub fn theme_files_from_dir(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let name = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Theme>(&raw).ok())
            .map(|theme| theme.name)
            .unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("theme")
                    .to_string()
            });
        files.push((name, path));
    }
    files
}

fn npm_source(path: &str) -> Option<String> {
    let marker = "node_modules/";
    let index = path.find(marker)?;
    let rest = &path[index + marker.len()..];
    if rest.starts_with('@') {
        let mut parts = rest.split('/');
        let scope = parts.next()?;
        let name = parts.next()?;
        Some(format!("{scope}/{name}"))
    } else {
        rest.split('/').next().map(str::to_string)
    }
}

fn git_source(path: &str) -> Option<String> {
    let marker = "/git/";
    let index = path.find(marker)?;
    let rest = &path[index + marker.len()..];
    let mut parts = rest.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    Some(format!("{owner}/{repo}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::themes::builtin_themes;

    #[test]
    fn compact_list_and_display_path_match_ts() {
        let theme = builtin_themes()[0].clone();
        assert_eq!(
            format_compact_list(["demo", "alpha"], true),
            "  alpha, demo"
        );
        assert_eq!(
            format_display_path("/home/me/.pi/agent/skills/demo/SKILL.md", "/home/me"),
            "~/.pi/agent/skills/demo/SKILL.md"
        );
        assert_eq!(
            format_context_path("/tmp/proj/AGENTS.md", "/tmp/proj", "/home/me"),
            "AGENTS.md"
        );
        let header = section_header(&theme, "Skills");
        assert!(header.contains("[Skills]"));
        let compact = theme.fg("dim", "  alpha, demo");
        let mut loaded = LoadedResources::default();
        loaded.add_section(&theme, "Skills", &compact, "  project\n    /tmp/a", false);
        let collapsed = loaded.render(40).join("\n");
        assert!(collapsed.contains("[Skills]"));
        assert!(collapsed.contains("alpha, demo"));
        assert!(!collapsed.contains("/tmp/a"));
        loaded.set_expanded(true);
        let expanded = loaded.render(40).join("\n");
        assert!(expanded.contains("/tmp/a"));
    }

    #[test]
    fn scope_groups_order_project_user_path_and_packages() {
        let theme = builtin_themes()[0].clone();
        let items = vec![
            LoadedResourceItem {
                path: "/home/me/.pi/agent/skills/user/SKILL.md".into(),
                compact_label: "user-skill".into(),
                expanded_label: "~/.pi/agent/skills/user/SKILL.md".into(),
                source: infer_source_info(
                    "/home/me/.pi/agent/skills/user/SKILL.md",
                    "/tmp/proj",
                    "/home/me/.pi/agent",
                ),
            },
            LoadedResourceItem {
                path: "/tmp/proj/.pi/skills/proj/SKILL.md".into(),
                compact_label: "proj-skill".into(),
                expanded_label: "/tmp/proj/.pi/skills/proj/SKILL.md".into(),
                source: infer_source_info(
                    "/tmp/proj/.pi/skills/proj/SKILL.md",
                    "/tmp/proj",
                    "/home/me/.pi/agent",
                ),
            },
            LoadedResourceItem {
                path: "/opt/extra/skill.md".into(),
                compact_label: "extra".into(),
                expanded_label: "/opt/extra/skill.md".into(),
                source: infer_source_info("/opt/extra/skill.md", "/tmp/proj", "/home/me/.pi/agent"),
            },
            LoadedResourceItem {
                path: "/tmp/proj/node_modules/@acme/ext/skills/pack/SKILL.md".into(),
                compact_label: "@acme/ext".into(),
                expanded_label: "skills/pack/SKILL.md".into(),
                source: infer_source_info(
                    "/tmp/proj/node_modules/@acme/ext/skills/pack/SKILL.md",
                    "/tmp/proj",
                    "/home/me/.pi/agent",
                ),
            },
        ];
        assert_eq!(items[0].source.scope, ResourceScope::User);
        assert_eq!(items[1].source.scope, ResourceScope::Project);
        assert_eq!(items[2].source.scope, ResourceScope::Path);
        assert_eq!(items[3].source.source, "npm:@acme/ext");
        let body = format_scope_groups(&theme, &items, |item| item.expanded_label.clone());
        let project_at = body.find("project").expect("project");
        let user_at = body.find("user").expect("user");
        let path_at = body.find("path").expect("path");
        assert!(project_at < user_at && user_at < path_at);
        assert!(body.contains("npm:@acme/ext"));
        let collision =
            format_collision_diagnostic(&theme, "demo", "/tmp/a/SKILL.md", &["/tmp/b/SKILL.md"]);
        assert!(collision.contains("\"demo\" collision:"));
        assert!(collision.contains("✓"));
        assert!(collision.contains("(skipped)"));
    }
}
