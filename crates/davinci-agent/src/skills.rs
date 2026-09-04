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
    })
}

/// TS `_expandSkillCommand` (`/skill:name args` → XML skill block).
pub fn expand_skill_command(text: &str, skills: &[Skill]) -> String {
    if !text.starts_with("/skill:") {
        return text.to_string();
    }
    let rest = &text["/skill:".len()..];
    let (skill_name, args) = match rest.find(' ') {
        Some(index) => (&rest[..index], rest[index + 1..].trim()),
        None => (rest, ""),
    };
    let Some(skill) = skills.iter().find(|item| item.name == skill_name) else {
        return text.to_string();
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
    if args.is_empty() {
        skill_block
    } else {
        format!("{skill_block}\n\n{args}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpandedUserText {
    pub text: String,
    pub skills: Vec<String>,
}

/// Expand `/skill:name` then prompt templates, returning the final text and injected skill names.
pub fn expand_user_text_with_metadata(
    text: &str,
    skills: &[Skill],
    templates: &[crate::PromptTemplate],
) -> ExpandedUserText {
    let mut matched_skills = Vec::new();
    let after_skill = if let Some(rest) = text.strip_prefix("/skill:") {
        let (skill_name, _) = match rest.find(' ') {
            Some(index) => (&rest[..index], rest[index + 1..].trim()),
            None => (rest, ""),
        };
        if skills.iter().any(|item| item.name == skill_name) {
            matched_skills.push(skill_name.to_string());
        }
        expand_skill_command(text, skills)
    } else {
        text.to_string()
    };
    let text = crate::expand_prompt_template(&after_skill, templates);
    ExpandedUserText {
        text,
        skills: matched_skills,
    }
}

/// Expand `/skill:name` then prompt templates. Used by AgentSession.prompt.
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

    #[test]
    fn expand_user_text_runs_skill_then_template() {
        let templates = vec![crate::PromptTemplate {
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
}
