use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
    pub body: String,
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
    let body = fs::read_to_string(path).ok()?;
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("skill")
        .to_string();
    let description = body
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("")
        .to_string();
    Some(Skill {
        name,
        path: path.to_path_buf(),
        description,
        body,
    })
}
