//! Custom Agent Profiles (`.davinci/agents/*.md` and legacy `.pi/agents/*.md`).
//!
//! Provides reusable, declarative agent profile definitions with scoped permissions,
//! tools, memory scope, and system prompts.

use davinci_agent::parse_frontmatter;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Scopes for agent memory retrieval and persistence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Agent does not access or store vector memory.
    None,
    /// Agent accesses shared project memory (default).
    Project,
    /// Agent accesses isolated memory for this agent profile within the project.
    AgentProject,
    /// Agent accesses isolated memory for this agent profile across projects.
    AgentGlobal,
}

impl Default for MemoryScope {
    fn default() -> Self {
        Self::Project
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "project" => Ok(Self::Project),
            "agent_project" | "agent-project" => Ok(Self::AgentProject),
            "agent_global" | "agent-global" => Ok(Self::AgentGlobal),
            other => Err(format!("Unknown memory scope: '{}'", other)),
        }
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryScope::None => write!(f, "none"),
            MemoryScope::Project => write!(f, "project"),
            MemoryScope::AgentProject => write!(f, "agent_project"),
            MemoryScope::AgentGlobal => write!(f, "agent_global"),
        }
    }
}

/// A parsed custom agent profile.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentProfile {
    /// Unique profile name (e.g. "code-improver").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Model to use ("inherit" or specific provider/model).
    pub model: String,
    /// Permission mode ("read-only", "ask", "edits", "auto").
    pub permission_mode: String,
    /// Allowed tools for this agent.
    pub tools: Vec<String>,
    /// Memory scope.
    pub memory_scope: MemoryScope,
    /// Maximum context token budget before truncation/compaction.
    pub max_context_tokens: Option<usize>,
    /// System prompt body.
    pub system_prompt: String,
    /// File path this profile was loaded from.
    pub path: PathBuf,
    /// Whether this profile is from the local project (vs user global).
    pub is_project: bool,
}

impl AgentProfile {
    /// Parse an `AgentProfile` from markdown text with YAML frontmatter.
    pub fn parse(content: &str, path: &Path, is_project: bool) -> Result<Self, String> {
        let (frontmatter, body) = parse_frontmatter(content);

        let default_name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed")
            .to_string();

        let name = frontmatter
            .get("name")
            .cloned()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or(default_name);

        let description = frontmatter.get("description").cloned().unwrap_or_default();

        let model = frontmatter
            .get("model")
            .cloned()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| "inherit".to_string());

        let permission_mode = frontmatter
            .get("permission_mode")
            .or_else(|| frontmatter.get("permissions"))
            .cloned()
            .filter(|p| !p.trim().is_empty())
            .unwrap_or_else(|| "read-only".to_string());

        let tools = frontmatter
            .get("tools")
            .map(|t| parse_tools_list(t))
            .unwrap_or_default();

        let memory_scope = frontmatter
            .get("memory_scope")
            .or_else(|| frontmatter.get("memory"))
            .map(|m| m.parse::<MemoryScope>())
            .transpose()?
            .unwrap_or_default();

        let max_context_tokens = frontmatter
            .get("max_context_tokens")
            .and_then(|t| t.parse::<usize>().ok());

        Ok(Self {
            name,
            description,
            model,
            permission_mode,
            tools,
            memory_scope,
            max_context_tokens,
            system_prompt: body,
            path: path.to_path_buf(),
            is_project,
        })
    }

    /// Validate profile fields against available tools and models.
    #[allow(dead_code)]
    pub fn validate(
        &self,
        available_tools: Option<&[String]>,
        available_models: Option<&[String]>,
    ) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("Agent profile name cannot be empty".to_string());
        }

        // Validate permission mode
        let valid_modes = ["read-only", "ask", "edits", "auto"];
        if !valid_modes.contains(&self.permission_mode.as_str()) {
            return Err(format!(
                "Profile '{}' has invalid permission mode '{}'. Valid modes: {:?}",
                self.name, self.permission_mode, valid_modes
            ));
        }

        // Validate tools
        if let Some(tools) = available_tools {
            for tool in &self.tools {
                if !tools.contains(tool) {
                    return Err(format!(
                        "Profile '{}' references unknown tool '{}'",
                        self.name, tool
                    ));
                }
            }
        }

        // Validate model
        if self.model != "inherit" {
            if let Some(models) = available_models {
                if !models.is_empty() {
                    let found = models.iter().any(|m| {
                        m == &self.model
                            || m.ends_with(&format!("/{}", self.model))
                            || self.model.ends_with(&format!("/{}", m))
                    });
                    if !found {
                        return Err(format!(
                            "Profile '{}' references unknown model '{}'",
                            self.name, self.model
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

/// Parse tools from YAML frontmatter formatted either as `[tool1, tool2]` or `tool1, tool2`.
fn parse_tools_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    let unbracketed = trimmed
        .strip_prefix('[')
        .unwrap_or(trimmed)
        .strip_suffix(']')
        .unwrap_or(trimmed);
    unbracketed
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Discovers all available agent profiles from user global directories and project directories.
///
/// Invariants:
/// - Profiles have unique names.
/// - If `project_trusted` is false, project directory profiles are NOT loaded.
/// - If `project_trusted` is true, a project profile overrides a user profile with the same name.
/// - Modern `.davinci` directories take precedence over legacy `.pi` directories.
pub fn discover_agent_profiles(
    cwd: &Path,
    user_home: Option<&Path>,
    project_trusted: bool,
) -> Vec<AgentProfile> {
    let mut profiles_by_name: BTreeMap<String, AgentProfile> = BTreeMap::new();

    // 1. User global directory profiles
    let home = user_home.map(PathBuf::from).or_else(|| {
        std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
            .ok()
            .map(PathBuf::from)
    });

    if let Some(h) = home {
        // Legacy user paths first
        let legacy_user_dirs = [
            h.join(".pi").join("agents"),
            h.join(".pi").join("agent").join("agents"),
        ];
        for dir in &legacy_user_dirs {
            load_profiles_from_dir(dir, false, &mut profiles_by_name);
        }

        // Modern user path overrides legacy
        let davinci_user_dir = h.join(".davinci").join("agent").join("agents");
        load_profiles_from_dir(&davinci_user_dir, false, &mut profiles_by_name);
    }

    // 2. Project profiles (only if trusted)
    if project_trusted {
        // Legacy project path first
        let legacy_project_dir = cwd.join(".pi").join("agents");
        load_profiles_from_dir(&legacy_project_dir, true, &mut profiles_by_name);

        // Modern project path overrides legacy and user profiles
        let davinci_project_dir = cwd.join(".davinci").join("agents");
        load_profiles_from_dir(&davinci_project_dir, true, &mut profiles_by_name);
    }

    profiles_by_name.into_values().collect()
}

fn load_profiles_from_dir(dir: &Path, is_project: bool, map: &mut BTreeMap<String, AgentProfile>) {
    if !dir.is_dir() {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|ext| ext.to_str()) == Some("md"))
        .collect();
    // Sort paths alphabetically for determinism
    paths.sort();

    for path in paths {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(profile) = AgentProfile::parse(&content, &path, is_project) {
                map.insert(profile.name.clone(), profile);
            }
        }
    }
}

/// Formats a list of discovered agent profiles for display in the terminal / interactive session.
pub fn format_agent_profiles_status(
    cwd: &Path,
    user_home: Option<&Path>,
    project_trusted: bool,
) -> String {
    let profiles = discover_agent_profiles(cwd, user_home, project_trusted);
    if profiles.is_empty() {
        let mut msg =
            "No custom agent profiles found in .davinci/agents/ or ~/.davinci/agent/agents/.\n"
                .to_string();
        if !project_trusted {
            msg.push_str("Note: Project is untrusted. Project-level profiles in .davinci/agents/ are ignored until trusted.\n");
        }
        return msg;
    }

    let mut out = format!("Custom Agent Profiles ({} available):\n\n", profiles.len());

    for p in &profiles {
        let source_label = if p.is_project { "project" } else { "user" };
        let tools_display = if p.tools.is_empty() {
            "(none)".to_string()
        } else {
            p.tools.join(", ")
        };
        out.push_str(&format!(
            "• {} ({}: {})\n  Description: {}\n  Model: {}\n  Permissions: {}\n  Tools: {}\n  Memory Scope: {}\n",
            p.name,
            source_label,
            p.path.display(),
            if p.description.is_empty() { "(no description)" } else { &p.description },
            p.model,
            p.permission_mode,
            tools_display,
            p.memory_scope,
        ));
        if let Some(tokens) = p.max_context_tokens {
            out.push_str(&format!("  Max Context Tokens: {}\n", tokens));
        }
        out.push('\n');
    }

    if !project_trusted {
        out.push_str(
            "Note: Project is untrusted. Project-level profiles in .davinci/agents/ are ignored.\n",
        );
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_parse_profile_frontmatter_and_body() {
        let content = r#"---
name: code-improver
description: Reviews code for correctness and maintainability.
model: inherit
permission_mode: read-only
tools: [read, grep, find, ls]
memory_scope: project
max_context_tokens: 20000
---

You are a code improvement specialist. Focus on clean architecture.
"#;
        let p = AgentProfile::parse(content, Path::new("code-improver.md"), true).unwrap();
        assert_eq!(p.name, "code-improver");
        assert_eq!(
            p.description,
            "Reviews code for correctness and maintainability."
        );
        assert_eq!(p.model, "inherit");
        assert_eq!(p.permission_mode, "read-only");
        assert_eq!(p.tools, vec!["read", "grep", "find", "ls"]);
        assert_eq!(p.memory_scope, MemoryScope::Project);
        assert_eq!(p.max_context_tokens, Some(20000));
        assert_eq!(
            p.system_prompt,
            "You are a code improvement specialist. Focus on clean architecture."
        );
        assert!(p.is_project);
    }

    #[test]
    fn test_profile_validation_success_and_failures() {
        let mut p = AgentProfile {
            name: "reviewer".into(),
            description: "Desc".into(),
            model: "inherit".into(),
            permission_mode: "read-only".into(),
            tools: vec!["read".into(), "grep".into()],
            memory_scope: MemoryScope::Project,
            max_context_tokens: None,
            system_prompt: "Prompt".into(),
            path: PathBuf::from("test.md"),
            is_project: false,
        };

        let available_tools = vec!["read".into(), "grep".into(), "write".into()];
        let available_models = vec!["openai/gpt-4o".into(), "anthropic/claude-3-5-sonnet".into()];

        // 1. Valid profile passes
        assert!(p
            .validate(Some(&available_tools), Some(&available_models))
            .is_ok());

        // 2. Invalid tool fails
        p.tools.push("unsupported_tool".into());
        let err = p
            .validate(Some(&available_tools), Some(&available_models))
            .unwrap_err();
        assert!(err.contains("unknown tool 'unsupported_tool'"));

        // Reset tools
        p.tools = vec!["read".into()];

        // 3. Invalid model fails
        p.model = "non-existent-model".into();
        let err2 = p
            .validate(Some(&available_tools), Some(&available_models))
            .unwrap_err();
        assert!(err2.contains("unknown model 'non-existent-model'"));

        // 4. Valid specific model passes
        p.model = "gpt-4o".into();
        assert!(p
            .validate(Some(&available_tools), Some(&available_models))
            .is_ok());

        // 5. Invalid permission mode fails
        p.permission_mode = "super-admin".into();
        let err3 = p
            .validate(Some(&available_tools), Some(&available_models))
            .unwrap_err();
        assert!(err3.contains("invalid permission mode"));
    }

    #[test]
    fn test_duplicate_precedence_project_overrides_user() {
        let user_dir = tempdir().unwrap();
        let project_dir = tempdir().unwrap();

        // Create user profile
        let user_agents = user_dir
            .path()
            .join(".davinci")
            .join("agent")
            .join("agents");
        fs::create_dir_all(&user_agents).unwrap();
        fs::write(
            user_agents.join("helper.md"),
            r#"---
name: helper
description: User global helper
model: inherit
permission_mode: read-only
tools: [read]
---
User prompt"#,
        )
        .unwrap();

        // Create project profile with the same name
        let project_agents = project_dir.path().join(".davinci").join("agents");
        fs::create_dir_all(&project_agents).unwrap();
        fs::write(
            project_agents.join("helper.md"),
            r#"---
name: helper
description: Project override helper
model: inherit
permission_mode: edits
tools: [read, edit, write]
---
Project prompt"#,
        )
        .unwrap();

        // When trusted, project profile overrides user profile
        let profiles = discover_agent_profiles(project_dir.path(), Some(user_dir.path()), true);
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].name, "helper");
        assert_eq!(profiles[0].description, "Project override helper");
        assert_eq!(profiles[0].permission_mode, "edits");
        assert!(profiles[0].is_project);
    }

    #[test]
    fn test_project_trust_gate_ignores_untrusted_project() {
        let user_dir = tempdir().unwrap();
        let project_dir = tempdir().unwrap();

        // Create user profile
        let user_agents = user_dir
            .path()
            .join(".davinci")
            .join("agent")
            .join("agents");
        fs::create_dir_all(&user_agents).unwrap();
        fs::write(
            user_agents.join("global.md"),
            r#"---
name: global-agent
description: Global agent
---
Prompt"#,
        )
        .unwrap();

        // Create project profile
        let project_agents = project_dir.path().join(".davinci").join("agents");
        fs::create_dir_all(&project_agents).unwrap();
        fs::write(
            project_agents.join("project.md"),
            r#"---
name: project-agent
description: Project agent
---
Prompt"#,
        )
        .unwrap();

        // Untrusted project: project profiles are ignored
        let profiles_untrusted =
            discover_agent_profiles(project_dir.path(), Some(user_dir.path()), false);
        assert_eq!(profiles_untrusted.len(), 1);
        assert_eq!(profiles_untrusted[0].name, "global-agent");

        // Trusted project: both are loaded
        let profiles_trusted =
            discover_agent_profiles(project_dir.path(), Some(user_dir.path()), true);
        assert_eq!(profiles_trusted.len(), 2);
    }

    #[test]
    fn test_legacy_pi_path_resolution() {
        let user_dir = tempdir().unwrap();
        let project_dir = tempdir().unwrap();

        // Create legacy user profile
        let legacy_user_agents = user_dir.path().join(".pi").join("agents");
        fs::create_dir_all(&legacy_user_agents).unwrap();
        fs::write(
            legacy_user_agents.join("legacy-user.md"),
            r#"---
name: legacy-user
description: Legacy user
---
Prompt"#,
        )
        .unwrap();

        // Create legacy project profile
        let legacy_project_agents = project_dir.path().join(".pi").join("agents");
        fs::create_dir_all(&legacy_project_agents).unwrap();
        fs::write(
            legacy_project_agents.join("legacy-proj.md"),
            r#"---
name: legacy-proj
description: Legacy project
---
Prompt"#,
        )
        .unwrap();

        let profiles = discover_agent_profiles(project_dir.path(), Some(user_dir.path()), true);
        assert_eq!(profiles.len(), 2);
        let names: Vec<&str> = profiles.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"legacy-user"));
        assert!(names.contains(&"legacy-proj"));
    }
}
