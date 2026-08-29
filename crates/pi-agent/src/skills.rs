use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub description: Option<String>,
    pub body: String,
}

pub fn load_skills(paths: &[PathBuf], disabled: bool) -> Vec<Skill> {
    if disabled {
        return Vec::new();
    }
    let mut skills = Vec::new();
    for path in paths {
        collect_skill(path, &mut skills);
    }
    skills
}

fn collect_skill(path: &Path, skills: &mut Vec<Skill>) {
    if path.is_file() {
        if let Ok(body) = fs::read_to_string(path) {
            skills.push(Skill {
                name: path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("skill")
                    .to_string(),
                path: path.to_path_buf(),
                description: first_heading(&body),
                body,
            });
        }
        return;
    }
    if path.is_dir() {
        for entry in WalkDir::new(path).max_depth(3).into_iter().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md") {
                collect_skill(p, skills);
            }
        }
    }
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .find(|l| l.starts_with('#'))
        .map(|l| l.trim_start_matches('#').trim().to_string())
}

pub fn discover_default_skill_dirs(cwd: &Path, agent_dir: &Path) -> Vec<PathBuf> {
    [cwd.join(".pi").join("skills"), agent_dir.join("skills")]
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}
