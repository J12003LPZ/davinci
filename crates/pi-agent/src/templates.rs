use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct PromptTemplate {
    pub name: String,
    pub path: PathBuf,
    pub body: String,
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
        if let Ok(body) = fs::read_to_string(path) {
            templates.push(PromptTemplate {
                name: path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("template")
                    .to_string(),
                path: path.to_path_buf(),
                body,
            });
        }
        return;
    }
    if path.is_dir() {
        for entry in WalkDir::new(path).max_depth(3).into_iter().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md") {
                collect(p, templates);
            }
        }
    }
}

pub fn render_template(template: &PromptTemplate, vars: &[(&str, &str)]) -> String {
    let mut body = template.body.clone();
    for (key, value) in vars {
        body = body.replace(&format!("{{{{{key}}}}}"), value);
    }
    body
}
