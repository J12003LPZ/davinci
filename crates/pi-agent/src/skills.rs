use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

#[derive(Debug, Clone)]
pub struct Skill {
    pub name: String,
    pub path: PathBuf,
    pub description: String,
    pub body: String,
    pub disable_model_invocation: bool,
}

impl Skill {
    pub fn base_dir(&self) -> PathBuf {
        self.path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.path.clone())
    }
}

pub fn load_skills(paths: &[PathBuf], disabled: bool) -> Vec<Skill> {
    if disabled {
        return Vec::new();
    }
    let mut skills = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for path in paths {
        collect_skill(path, &mut skills, &mut seen);
    }
    skills
}

fn collect_skill(
    path: &Path,
    skills: &mut Vec<Skill>,
    seen: &mut std::collections::BTreeSet<PathBuf>,
) {
    if path.is_file() {
        if let Some(skill) = load_skill_from_file(path) {
            let key = canonicalize_or_self(&skill.path);
            if seen.insert(key) {
                skills.push(skill);
            }
        }
        return;
    }
    if path.is_dir() {
        for entry in WalkDir::new(path).max_depth(8).into_iter().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Some(skill) = load_skill_from_file(p) {
                    let key = canonicalize_or_self(&skill.path);
                    if seen.insert(key) {
                        skills.push(skill);
                    }
                }
            }
        }
    }
}

fn canonicalize_or_self(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn load_skill_from_file(path: &Path) -> Option<Skill> {
    let raw = fs::read_to_string(path).ok()?;
    let is_declared = path.file_name().and_then(|n| n.to_str()) == Some("SKILL.md");
    let (frontmatter, body) = parse_simple_frontmatter(&raw);
    let description = frontmatter.get("description").cloned().unwrap_or_default();
    if !is_declared && description.trim().is_empty() {
        return None;
    }
    let parent_dir_name = path
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();
    let name = frontmatter
        .get("name")
        .cloned()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or(parent_dir_name);
    let disable_model_invocation = frontmatter
        .get("disable-model-invocation")
        .map(|v| v == "true" || v == "yes" || v == "1")
        .unwrap_or(false);
    Some(Skill {
        name,
        path: path.to_path_buf(),
        description,
        body,
        disable_model_invocation,
    })
}

pub fn format_skills_for_prompt(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|skill| !skill.disable_model_invocation)
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    let mut lines = vec![
        String::new(),
        String::new(),
        "The following skills provide specialized instructions for specific tasks.".into(),
        "Use the read tool to load a skill's file when the task matches its description.".into(),
        "When a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.".into(),
        String::new(),
        "<available_skills>".into(),
    ];
    for skill in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!(
            "    <description>{}</description>",
            escape_xml(&skill.description)
        ));
        lines.push(format!(
            "    <location>{}</location>",
            escape_xml(&skill.path.display().to_string())
        ));
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn parse_simple_frontmatter(content: &str) -> (BTreeMap<String, String>, String) {
    let normalized = content.replace("\r\n", "\n").replace('\r', "\n");
    let normalized = normalized.strip_prefix('\u{feff}').unwrap_or(&normalized);
    if !normalized.starts_with("---") {
        return (BTreeMap::new(), normalized.to_string());
    }
    let Some(end) = normalized.find("\n---") else {
        return (BTreeMap::new(), normalized.to_string());
    };
    let yaml = &normalized[3..end];
    let yaml = yaml.strip_prefix('\n').unwrap_or(yaml);
    let body = normalized[end + 4..].trim().to_string();
    let mut map = BTreeMap::new();
    for line in yaml.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let mut value = value.trim();
        if (value.starts_with('"') && value.ends_with('"') && value.len() >= 2)
            || (value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2)
        {
            value = &value[1..value.len() - 1];
        }
        map.insert(key.trim().to_string(), value.to_string());
    }
    (map, body)
}

pub fn discover_default_skill_dirs(cwd: &Path, agent_dir: &Path) -> Vec<PathBuf> {
    [cwd.join(".pi").join("skills"), agent_dir.join("skills")]
        .into_iter()
        .filter(|p| p.exists())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_md_uses_parent_dir_name_and_frontmatter() {
        let dir = tempfile::tempdir().unwrap();
        let skill_dir = dir.path().join("review");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\ndescription: Review a diff\n---\nRead the patch.\n",
        )
        .unwrap();
        let skills = load_skills(&[skill_dir.clone()], false);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "review");
        assert_eq!(skills[0].description, "Review a diff");
        assert_eq!(skills[0].body, "Read the patch.");
        assert!(!skills[0].disable_model_invocation);
        let prompt = format_skills_for_prompt(&skills);
        assert!(prompt.contains("<name>review</name>"));
        assert!(prompt.contains("<available_skills>"));
        assert!(!prompt.contains("Read the patch"));
    }

    #[test]
    fn loose_md_requires_description() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.md"), "# Notes\n\nhello\n").unwrap();
        assert!(load_skills(&[dir.path().to_path_buf()], false).is_empty());
        std::fs::write(
            dir.path().join("named.md"),
            "---\nname: named\ndescription: A named skill\n---\nBody\n",
        )
        .unwrap();
        let skills = load_skills(&[dir.path().to_path_buf()], false);
        assert_eq!(skills[0].name, "named");
        assert_eq!(skills[0].description, "A named skill");
    }
}
