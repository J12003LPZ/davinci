use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::templates::strip_frontmatter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub base_dir: PathBuf,
    /// The plugin that ships it, as Claude Code namespaces plugin
    /// commands: invoked as `/<namespace>:<name>`. `None` for the user's and
    /// the project's own, invoked as `/<name>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

impl Skill {
    /// What follows `/` to invoke it: `superpowers:writing-plans` for a
    /// plugin's skill, `my-skill` for the user's own.
    pub fn command_name(&self) -> String {
        command_name(self.namespace.as_deref(), &self.name)
    }
}

/// `namespace:name`, or `name` alone.
pub fn command_name(namespace: Option<&str>, name: &str) -> String {
    match namespace {
        Some(namespace) => format!("{namespace}:{name}"),
        None => name.to_string(),
    }
}

/// The skill `/command` names: its command name first, then the legacy
/// `skill:<name>`, then a plugin skill's bare name. Names match without
/// regard to case, as the `/` menu does. A prompt template (a command) that
/// answers to the same name keeps it, as the menu lists the command: a skill
/// never shadows one.
///
/// Precedence, as the `/` menu lists them: a command (prompt template) by its
/// full name, then a skill by its full name (the user's `deploy`, a plugin's
/// `ops:deploy`), then a plugin command by its bare name, then a plugin
/// skill by its bare name. With "Skill commands" off only `/skill:<name>`
/// runs a skill.
pub fn find_skill_command<'a>(
    command: &str,
    skills: &'a [Skill],
    templates: &[crate::PromptTemplate],
) -> Option<&'a Skill> {
    find_skill_command_in(command, skills, templates, skill_commands_enabled())
}

fn find_skill_command_in<'a>(
    command: &str,
    skills: &'a [Skill],
    templates: &[crate::PromptTemplate],
    commands_enabled: bool,
) -> Option<&'a Skill> {
    if let Some(name) = command.strip_prefix("skill:") {
        return skills.iter().find(|skill| skill.name == name).or_else(|| {
            skills
                .iter()
                .find(|skill| skill.name.eq_ignore_ascii_case(name))
        });
    }
    if !commands_enabled {
        return None;
    }
    let answers = |name: String| name.eq_ignore_ascii_case(command);
    if templates
        .iter()
        .any(|template| answers(template.command_name()))
    {
        return None;
    }
    if let Some(skill) = skills.iter().find(|skill| answers(skill.command_name())) {
        return Some(skill);
    }
    if templates
        .iter()
        .any(|template| template.namespace.is_some() && answers(template.name.clone()))
    {
        return None;
    }
    skills
        .iter()
        .find(|skill| skill.namespace.is_some() && answers(skill.name.clone()))
}

static SKILL_COMMANDS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The "Skill commands" setting: whether `/<name>` and `/<plugin>:<name>`
/// run skills. `/skill:<name>` always does.
pub fn set_skill_commands_enabled(enabled: bool) {
    SKILL_COMMANDS.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

pub fn skill_commands_enabled() -> bool {
    SKILL_COMMANDS.load(std::sync::atomic::Ordering::Relaxed)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDescriptor {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub base_dir: PathBuf,
}

impl From<&Skill> for SkillDescriptor {
    fn from(skill: &Skill) -> Self {
        Self {
            name: skill.name.clone(),
            description: skill.description.clone(),
            path: skill.path.clone(),
            base_dir: skill.base_dir.clone(),
        }
    }
}

pub fn describe_skill(skill: &Skill) -> SkillDescriptor {
    SkillDescriptor::from(skill)
}

pub fn discover_skills(roots: &[PathBuf]) -> Vec<Skill> {
    let mut skills = Vec::new();
    for root in roots {
        if root.is_file() {
            if let Some(skill) = load_skill(root) {
                skills.push(skill);
            }
            continue;
        }
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).max_depth(3).into_iter().flatten() {
            let path = entry.path();
            if path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md")
                || path.extension().and_then(|e| e.to_str()) == Some("md")
            {
                if let Some(skill) = load_skill(path) {
                    skills.push(skill);
                }
            }
        }
    }
    skills
}

fn load_skill(path: &Path) -> Option<Skill> {
    let raw = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = crate::templates::parse_frontmatter(&raw);
    let parent_name = path
        .parent()
        .and_then(|parent| parent.file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let file_stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("skill");
    let is_declared = path.file_name().and_then(|name| name.to_str()) == Some("SKILL.md");
    let name = frontmatter
        .get("name")
        .cloned()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| {
            if is_declared {
                parent_name.to_string()
            } else {
                file_stem.to_string()
            }
        });
    let description = frontmatter
        .get("description")
        .cloned()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            body.lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("")
                .to_string()
        });
    Some(Skill {
        name,
        path: path.to_path_buf(),
        description,
        body: raw,
        base_dir: path.parent().unwrap_or(path).to_path_buf(),
        namespace: None,
    })
}

/// TS `_expandSkillCommand` (`/skill:name args` → XML skill block), also
/// for `/name` and a plugin's `/plugin:name`, as Claude Code invokes skills.
pub fn expand_skill_command(text: &str, skills: &[Skill]) -> String {
    expand_skill_command_with(text, skills, &[]).0
}

/// The leading `/command` of `text` and what follows it.
fn split_command(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    Some(match rest.split_once(char::is_whitespace) {
        Some((command, args)) => (command, args.trim()),
        None => (rest, ""),
    })
}

/// Expands a skill command, returning the text and the skill it named.
fn expand_skill_command_with(
    text: &str,
    skills: &[Skill],
    templates: &[crate::PromptTemplate],
) -> (String, Option<String>) {
    let Some((command, args)) = split_command(text) else {
        return (text.to_string(), None);
    };
    let Some(skill) = find_skill_command(command, skills, templates) else {
        return (text.to_string(), None);
    };
    let raw = fs::read_to_string(&skill.path).unwrap_or_else(|_| skill.body.clone());
    let body = strip_frontmatter(&raw).trim().to_string();
    let location = skill.path.display();
    let base = if skill.base_dir.as_os_str().is_empty() {
        skill
            .path
            .parent()
            .unwrap_or(skill.path.as_path())
            .display()
            .to_string()
    } else {
        skill.base_dir.display().to_string()
    };
    let skill_block = format!(
        "<skill name=\"{name}\" location=\"{location}\">\nReferences are relative to {base}.\n\n{body}\n</skill>",
        name = skill.name
    );
    let text = if args.is_empty() {
        skill_block
    } else {
        format!("{skill_block}\n\n{args}")
    };
    (text, Some(skill.name.clone()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedUserText {
    pub text: String,
    pub skills: Vec<String>,
}

/// Expand a skill command (`/name`, `/plugin:name`, `/skill:name`) then prompt templates, returning the final text and injected skill names.
pub fn expand_user_text_with_metadata(
    text: &str,
    skills: &[Skill],
    templates: &[crate::PromptTemplate],
) -> ExpandedUserText {
    let (after_skill, matched) = expand_skill_command_with(text, skills, templates);
    let matched_skills: Vec<String> = matched.into_iter().collect();
    let text = crate::expand_prompt_template(&after_skill, templates);
    ExpandedUserText {
        text,
        skills: matched_skills,
    }
}

/// Expand a skill command (`/name`, `/plugin:name`, `/skill:name`) then prompt templates. Used by AgentSession.prompt.
pub fn expand_user_text(
    text: &str,
    skills: &[Skill],
    templates: &[crate::PromptTemplate],
) -> String {
    expand_user_text_with_metadata(text, skills, templates).text
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn expand_skill_command_wraps_body_and_args() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join("test");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: test\ndescription: Test skill\n---\n\nUse the skill body.\n",
        )
        .unwrap();
        let skills = discover_skills(&[skill_dir.clone()]);
        assert_eq!(skills[0].name, "test");
        let expanded = expand_skill_command("/skill:test explain this", &skills);
        assert!(expanded.contains("<skill name=\"test\" location=\""));
        assert!(expanded.contains("Use the skill body."));
        assert!(expanded.contains("explain this"));
        assert!(expanded.contains(&format!(
            "References are relative to {}",
            skill_dir.display()
        )));
        assert_eq!(
            expand_skill_command("/skill:missing hi", &skills),
            "/skill:missing hi"
        );
        assert_eq!(expand_skill_command("plain", &skills), "plain");
    }

    fn skill_at(dir: &Path, name: &str, body: &str, namespace: Option<&str>) -> Skill {
        let skill_dir = dir.join(namespace.unwrap_or("user")).join(name);
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            format!("---\nname: {name}\ndescription: d\n---\n\n{body}\n"),
        )
        .unwrap();
        let mut skill = discover_skills(&[skill_dir]).remove(0);
        skill.namespace = namespace.map(str::to_string);
        skill
    }

    fn template(name: &str, body: &str, namespace: Option<&str>) -> crate::PromptTemplate {
        crate::PromptTemplate {
            name: name.into(),
            path: PathBuf::from(format!("/virtual/{name}.md")),
            body: body.into(),
            description: String::new(),
            argument_hint: None,
            namespace: namespace.map(str::to_string),
        }
    }

    #[test]
    fn skills_run_as_slash_commands_the_way_claude_code_names_them() {
        let dir = tempdir().unwrap();
        let skills = vec![
            skill_at(dir.path(), "my-skill", "Mine.", None),
            skill_at(dir.path(), "writing-plans", "Plan it.", Some("superpowers")),
        ];
        assert_eq!(skills[0].command_name(), "my-skill");
        assert_eq!(skills[1].command_name(), "superpowers:writing-plans");
        // The user's own skill by its bare name, with arguments.
        let mine = expand_user_text_with_metadata("/my-skill now", &skills, &[]);
        assert!(
            mine.text.contains("<skill name=\"my-skill\""),
            "{}",
            mine.text
        );
        assert!(mine.text.ends_with("now"));
        assert_eq!(mine.skills, ["my-skill"]);
        // A plugin's skill by `/plugin:skill`, by `/skill:name`, and by its
        // bare name when nothing else claims it.
        for typed in [
            "/superpowers:writing-plans",
            "/skill:writing-plans",
            "/writing-plans",
        ] {
            let res = expand_user_text_with_metadata(typed, &skills, &[]);
            assert!(res.text.contains("Plan it."), "{typed}: {}", res.text);
            assert_eq!(res.skills, ["writing-plans"], "{typed}");
        }
        // Another plugin's namespace does not reach it.
        assert_eq!(
            expand_user_text("/other:writing-plans", &skills, &[]),
            "/other:writing-plans"
        );
        // A slash in the middle of text is not a command.
        assert_eq!(
            expand_user_text("see /my-skill", &skills, &[]),
            "see /my-skill"
        );
    }

    #[test]
    fn a_prompt_template_keeps_its_slash_over_a_skill_of_the_same_name() {
        let dir = tempdir().unwrap();
        let skills = vec![
            skill_at(dir.path(), "review", "Skill review.", None),
            skill_at(dir.path(), "deploy", "Plugin deploy.", Some("ops")),
        ];
        let templates = vec![
            template("review", "Template review: $1", None),
            template("deploy", "Mine: $1", None),
        ];
        assert_eq!(
            expand_user_text("/review x", &skills, &templates),
            "Template review: x"
        );
        // The user's command wins the bare name; the plugin's skill keeps its
        // namespaced one.
        assert_eq!(
            expand_user_text("/deploy y", &skills, &templates),
            "Mine: y"
        );
        assert!(expand_user_text("/ops:deploy", &skills, &templates).contains("Plugin deploy."));
        // `/skill:review` still names the skill explicitly.
        assert!(expand_user_text("/skill:review", &skills, &templates).contains("Skill review."));
    }

    #[test]
    fn multibyte_whitespace_after_a_slash_word_does_not_panic() {
        let dir = tempdir().unwrap();
        let skills = vec![skill_at(dir.path(), "my-skill", "Mine.", None)];
        for text in [
            "/etc/hosts\u{3000}why is this broken",
            "/\u{8def}\u{5f84}\u{a0}foo",
            "/my-skill\u{2028}next",
        ] {
            let _ = expand_user_text(text, &skills, &[]);
        }
        let expanded = expand_user_text("/my-skill\u{3000}now", &skills, &[]);
        assert!(
            expanded.contains("Mine.") && expanded.ends_with("now"),
            "{expanded}"
        );
    }

    #[test]
    fn a_plugin_command_and_skill_of_one_name_run_the_command_the_menu_lists() {
        let dir = tempdir().unwrap();
        let skills = vec![skill_at(dir.path(), "deploy", "Skill deploy.", Some("ops"))];
        let templates = vec![template("deploy", "Command deploy: $1", Some("ops"))];
        assert_eq!(
            expand_user_text("/ops:deploy x", &skills, &templates),
            "Command deploy: x"
        );
        assert_eq!(
            expand_user_text("/deploy y", &skills, &templates),
            "Command deploy: y"
        );
        // The skill stays reachable by its explicit name.
        assert!(expand_user_text("/skill:deploy", &skills, &templates).contains("Skill deploy."));
    }

    #[test]
    fn the_users_own_skill_keeps_its_bare_name_over_a_plugin_command() {
        let dir = tempdir().unwrap();
        let skills = vec![skill_at(dir.path(), "deploy", "My deploy.", None)];
        let templates = vec![template("deploy", "Plugin deploy: $1", Some("ops"))];
        assert!(expand_user_text("/deploy", &skills, &templates).contains("My deploy."));
        assert_eq!(
            expand_user_text("/ops:deploy z", &skills, &templates),
            "Plugin deploy: z"
        );
    }

    #[test]
    fn with_skill_commands_off_only_the_explicit_form_runs_a_skill() {
        let dir = tempdir().unwrap();
        let skills = vec![
            skill_at(dir.path(), "tmp", "Tmp skill.", None),
            skill_at(dir.path(), "plan-it", "Plan.", Some("kit")),
        ];
        for typed in ["tmp", "kit:plan-it", "plan-it"] {
            assert!(
                find_skill_command_in(typed, &skills, &[], false).is_none(),
                "{typed}"
            );
            assert!(
                find_skill_command_in(typed, &skills, &[], true).is_some(),
                "{typed}"
            );
        }
        assert!(find_skill_command_in("skill:tmp", &skills, &[], false).is_some());
    }

    #[test]
    fn a_differently_cased_command_still_runs_its_template() {
        let dir = tempdir().unwrap();
        let skills = vec![skill_at(dir.path(), "review", "Skill.", None)];
        let templates = vec![template("review", "Template: $1", None)];
        assert_eq!(
            expand_user_text("/Review x", &skills, &templates),
            "Template: x"
        );
    }

    #[test]
    fn skill_commands_match_without_case_as_the_menu_does() {
        let dir = tempdir().unwrap();
        let skills = vec![skill_at(dir.path(), "Review", "Reviewing.", Some("Kit"))];
        assert!(expand_user_text("/kit:review", &skills, &[]).contains("Reviewing."));
        assert!(expand_user_text("/review", &skills, &[]).contains("Reviewing."));
        assert!(expand_user_text("/SKILL:review", &skills, &[]).starts_with("/SKILL:"));
    }

    #[test]
    fn a_plugin_command_runs_as_plugin_colon_command() {
        let templates = vec![template(
            "commit",
            "Commit with care: $ARGUMENTS",
            Some("git-tools"),
        )];
        assert_eq!(templates[0].command_name(), "git-tools:commit");
        assert_eq!(
            expand_user_text("/git-tools:commit fix login", &[], &templates),
            "Commit with care: fix login"
        );
        assert_eq!(
            expand_user_text("/commit fix", &[], &templates),
            "Commit with care: fix"
        );
        assert_eq!(
            expand_user_text("/other:commit", &[], &templates),
            "/other:commit"
        );
    }

    #[test]
    fn expand_user_text_runs_skill_then_template() {
        let templates = vec![crate::PromptTemplate {
            namespace: None,
            name: "review".into(),
            path: PathBuf::from("/virtual/review.md"),
            body: "Review this code: $1".into(),
            description: "Review template".into(),
            argument_hint: None,
        }];
        assert_eq!(
            expand_user_text("/review src/index.ts", &[], &templates),
            "Review this code: src/index.ts"
        );
    }

    #[test]
    fn explicit_skill_expansion_remains_backward_compatible() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join("my-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: my-skill\ndescription: Test skill description\n---\n\n## Instructions\nDo something useful.\n",
        )
        .unwrap();
        let skills = discover_skills(&[skill_dir]);
        assert_eq!(skills.len(), 1);
        let expanded = expand_skill_command("/skill:my-skill do it", &skills);
        assert!(expanded.contains("<skill name=\"my-skill\""));
        assert!(expanded.contains("Do something useful."));
        assert!(expanded.contains("do it"));
    }

    #[test]
    fn expand_user_text_with_metadata_captures_injected_skills() {
        let dir = tempdir().unwrap();
        let skill_dir = dir.path().join("debug-sqlx");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let path = skill_dir.join("SKILL.md");
        std::fs::write(
            &path,
            "---\nname: debug-sqlx\ndescription: Debug SQLx\n---\n\n## Instructions\nFix sqlx.\n",
        )
        .unwrap();
        let skills = discover_skills(&[skill_dir]);
        let res = expand_user_text_with_metadata("/skill:debug-sqlx run check", &skills, &[]);
        assert_eq!(res.skills, vec!["debug-sqlx".to_string()]);
        assert!(res.text.contains("Fix sqlx."));
    }

    #[test]
    fn ordinary_prompt_never_injects_unselected_skill_body() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("migration");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: migration\ndescription: Create and validate schema migrations.\n---\nSECRET_SKILL_BODY_SENTINEL\n",
        )
        .unwrap();
        let skills = discover_skills(&[skill_dir]);
        let expanded = expand_user_text("Fix the parser", &skills, &[]);
        assert_eq!(expanded, "Fix the parser");
        assert!(!expanded.contains("SECRET_SKILL_BODY_SENTINEL"));
    }
}
