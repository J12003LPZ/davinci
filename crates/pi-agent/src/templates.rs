use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    pub path: PathBuf,
    pub body: String,
}

pub fn discover_prompt_templates(roots: &[PathBuf]) -> Vec<PromptTemplate> {
    let mut templates = Vec::new();
    for root in roots {
        if root.is_file() {
            if let Some(template) = load_template(root) {
                templates.push(template);
            }
            continue;
        }
        if !root.is_dir() {
            continue;
        }
        for entry in WalkDir::new(root).max_depth(3).into_iter().flatten() {
            let path = entry.path();
            if matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("md") | Some("txt") | Some("prompt")
            ) {
                if let Some(template) = load_template(path) {
                    templates.push(template);
                }
            }
        }
    }
    templates
}

fn load_template(path: &Path) -> Option<PromptTemplate> {
    Some(PromptTemplate {
        name: path.file_stem()?.to_str()?.to_string(),
        path: path.to_path_buf(),
        body: fs::read_to_string(path).ok()?,
    })
}
