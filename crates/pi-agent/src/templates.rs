use std::fs;
use std::path::{Path, PathBuf};

use crate::skills::parse_simple_frontmatter;

#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub path: PathBuf,
    pub body: String,
    pub description: Option<String>,
    pub argument_hint: Option<String>,
}

pub fn load_prompt_templates(paths: &[PathBuf], disabled: bool) -> Vec<PromptTemplate> {
    if disabled {
        return Vec::new();
    }
    let mut templates = Vec::new();
    for path in paths {
        collect(path, &mut templates);
    }
    templates
}

fn collect(path: &Path, templates: &mut Vec<PromptTemplate>) {
    if path.is_file() {
        if let Some(template) = load_template_from_file(path) {
            templates.push(template);
        }
        return;
    }
    if path.is_dir() {
        let Ok(entries) = fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            let is_file = if p.is_symlink() {
                p.metadata().map(|m| m.is_file()).unwrap_or(false)
            } else {
                p.is_file()
            };
            if is_file && p.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(template) = load_template_from_file(&p) {
                    templates.push(template);
                }
            }
        }
    }
}

fn load_template_from_file(path: &Path) -> Option<PromptTemplate> {
    let raw = fs::read_to_string(path).ok()?;
    let (frontmatter, body) = parse_simple_frontmatter(&raw);
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("template")
        .to_string();
    let description = frontmatter
        .get("description")
        .cloned()
        .filter(|d| !d.trim().is_empty())
        .or_else(|| first_description(&body));
    let argument_hint = frontmatter
        .get("argument-hint")
        .cloned()
        .filter(|h| !h.trim().is_empty());
    Some(PromptTemplate {
        name,
        path: path.to_path_buf(),
        body,
        description,
        argument_hint,
    })
}

fn first_description(body: &str) -> Option<String> {
    let line = body.lines().find(|line| !line.trim().is_empty())?;
    let trimmed = line.trim();
    if trimmed.len() > 60 {
        Some(format!("{}...", &trimmed[..60]))
    } else {
        Some(trimmed.to_string())
    }
}

pub fn render_template(template: &PromptTemplate, vars: &[(&str, &str)]) -> String {
    let mut body = template.body.clone();
    for (key, value) in vars {
        body = body.replace(&format!("{{{{{key}}}}}"), value);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_frontmatter_description_and_body() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("review.md"),
            "---\ndescription: Review helper\nargument-hint: <file>\n---\nLook at $1\n",
        )
        .unwrap();
        let templates = load_prompt_templates(&[dir.path().to_path_buf()], false);
        assert_eq!(templates.len(), 1);
        assert_eq!(templates[0].name, "review");
        assert_eq!(templates[0].description.as_deref(), Some("Review helper"));
        assert_eq!(templates[0].argument_hint.as_deref(), Some("<file>"));
        assert_eq!(templates[0].body, "Look at $1");
    }
}
