use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
    #[serde(rename = "disableModelInvocation", default)]
    pub disable_model_invocation: bool,
}

impl Skill {
    pub fn format_invocation(&self, additional_instructions: Option<&str>) -> String {
        let parent = Path::new(&self.file_path)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .display();
        let block = format!(
            "<skill name=\"{}\" location=\"{}\">\nReferences are relative to {}.\n\n{}\n</skill>",
            self.name, self.file_path, parent, self.content
        );
        match additional_instructions {
            Some(extra) => format!("{}\n\n{}", block, extra),
            None => block,
        }
    }
}

pub fn parse_skill_markdown(path: &Path, content: &str) -> Option<Skill> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|l| l.trim()) == Some("---") {
        if let Some(end_idx) = lines[1..].iter().position(|l| l.trim() == "---") {
            let frontmatter_lines = &lines[1..=end_idx];
            let body_lines = &lines[end_idx + 2..];
            let body = body_lines.join("\n");

            let mut name = String::new();
            let mut description = String::new();
            let mut disable_model = false;

            for line in frontmatter_lines {
                let trimmed = line.trim();
                if let Some(val) = trimmed.strip_prefix("name:") {
                    name = val.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if let Some(val) = trimmed.strip_prefix("description:") {
                    description = val.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if let Some(val) = trimmed.strip_prefix("disable-model-invocation:") {
                    disable_model = val.trim().parse::<bool>().unwrap_or(false);
                }
            }

            if name.is_empty() {
                name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unnamed")
                    .to_string();
            }

            return Some(Skill {
                name,
                description,
                content: body.trim().to_string(),
                file_path: path.display().to_string(),
                disable_model_invocation: disable_model,
            });
        }
    }

    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();

    Some(Skill {
        name: default_name,
        description: String::new(),
        content: content.trim().to_string(),
        file_path: path.display().to_string(),
        disable_model_invocation: false,
    })
}

pub fn load_skills_from_dir(dir: &Path) -> Vec<Skill> {
    let mut skills = Vec::new();
    if !dir.exists() || !dir.is_dir() {
        return skills;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(skill) = parse_skill_markdown(&path, &content) {
                        skills.push(skill);
                    }
                }
            } else if path.is_dir() {
                let skill_md = path.join("SKILL.md");
                if skill_md.exists() {
                    if let Ok(content) = std::fs::read_to_string(&skill_md) {
                        if let Some(skill) = parse_skill_markdown(&skill_md, &content) {
                            skills.push(skill);
                        }
                    }
                }
            }
        }
    }
    skills
}
