use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    pub description: String,
    #[serde(rename = "argumentHint", skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    pub template: String,
    #[serde(rename = "filePath")]
    pub file_path: String,
}

impl PromptTemplate {
    pub fn render(&self, argument: Option<&str>) -> String {
        let arg = argument.unwrap_or("");
        if self.template.contains("{{args}}") || self.template.contains("{{arg}}") {
            self.template
                .replace("{{args}}", arg)
                .replace("{{arg}}", arg)
        } else if !arg.is_empty() {
            format!("{}\n\n{}", self.template, arg)
        } else {
            self.template.clone()
        }
    }
}

pub fn parse_prompt_template(path: &Path, content: &str) -> Option<PromptTemplate> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|l| l.trim()) == Some("---") {
        if let Some(end_idx) = lines[1..].iter().position(|l| l.trim() == "---") {
            let frontmatter_lines = &lines[1..=end_idx];
            let body_lines = &lines[end_idx + 2..];
            let body = body_lines.join("\n");

            let mut description = String::new();
            let mut arg_hint = None;

            for line in frontmatter_lines {
                let trimmed = line.trim();
                if let Some(val) = trimmed.strip_prefix("description:") {
                    description = val.trim().trim_matches('"').trim_matches('\'').to_string();
                } else if let Some(val) = trimmed.strip_prefix("argument-hint:") {
                    arg_hint = Some(val.trim().trim_matches('"').trim_matches('\'').to_string());
                }
            }

            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unnamed")
                .to_string();

            return Some(PromptTemplate {
                name,
                description,
                argument_hint: arg_hint,
                template: body.trim().to_string(),
                file_path: path.display().to_string(),
            });
        }
    }

    let default_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("template")
        .to_string();

    Some(PromptTemplate {
        name: default_name,
        description: String::new(),
        argument_hint: None,
        template: content.trim().to_string(),
        file_path: path.display().to_string(),
    })
}

pub fn load_prompt_templates_from_dir(dir: &Path) -> Vec<PromptTemplate> {
    let mut templates = Vec::new();
    if !dir.exists() || !dir.is_dir() {
        return templates;
    }

    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Some(template) = parse_prompt_template(&path, &content) {
                        templates.push(template);
                    }
                }
            }
        }
    }
    templates
}
