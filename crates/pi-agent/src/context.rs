use std::fs;
use std::path::{Path, PathBuf};

pub const CONTEXT_FILES: &[&str] = &[
    "AGENTS.override.md",
    "AGENTS.md",
    "AGENTS.MD",
    "CLAUDE.md",
    "CLAUDE.MD",
];

pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

pub fn resolve_prompt_input(input: &str) -> String {
    let path = Path::new(input);
    if path.exists() && path.is_file() {
        fs::read_to_string(path)
            .map(|raw| strip_bom(&raw).to_string())
            .unwrap_or_else(|_| input.to_string())
    } else {
        input.to_string()
    }
}

fn load_context_file_from_dir(dir: &Path) -> Option<(String, String, PathBuf)> {
    for name in CONTEXT_FILES {
        let path = dir.join(name);
        if !path.is_file() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&path) {
            return Some(((*name).to_string(), strip_bom(&content).to_string(), path));
        }
    }
    None
}

pub fn load_context_files(cwd: &Path, disabled: bool) -> Vec<(String, String)> {
    load_project_context_files(cwd, None, disabled)
}

pub fn load_project_context_files(
    cwd: &Path,
    agent_dir: Option<&Path>,
    disabled: bool,
) -> Vec<(String, String)> {
    if disabled {
        return Vec::new();
    }
    let mut files = Vec::new();
    let mut seen = Vec::new();
    if let Some(agent) = agent_dir {
        if let Some((name, content, path)) = load_context_file_from_dir(agent) {
            seen.push(path);
            files.push((name, content));
        }
    }
    let mut ancestors = Vec::new();
    let mut dir = cwd.to_path_buf();
    loop {
        if let Some((name, content, path)) = load_context_file_from_dir(&dir) {
            if !seen.iter().any(|existing| existing == &path) {
                seen.push(path);
                ancestors.push((name, content));
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }
    }
    ancestors.reverse();
    files.extend(ancestors);
    files
}

pub fn render_system_prompt(
    base: Option<&str>,
    append: &[String],
    context: &[(String, String)],
) -> String {
    let mut prompt = base.map(resolve_prompt_input).unwrap_or_else(|| {
        "You are a coding assistant with read, bash, edit, and write tools.".into()
    });
    for extra in append {
        prompt.push('\n');
        prompt.push_str(&resolve_prompt_input(extra));
    }
    for (name, content) in context {
        prompt.push_str(&format!("\n\n# {name}\n{content}"));
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn loads_agents_md() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "use rustfmt").unwrap();
        let files = load_context_files(dir.path(), false);
        assert_eq!(files[0].0, "AGENTS.md");
        assert!(load_context_files(dir.path(), true).is_empty());
    }

    #[test]
    fn prefers_override_and_layers_ancestors_like_typescript() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("svc");
        fs::create_dir_all(&nested).unwrap();
        fs::write(dir.path().join("AGENTS.md"), "root").unwrap();
        fs::write(nested.join("AGENTS.override.md"), "override").unwrap();
        let files = load_project_context_files(&nested, Some(dir.path()), false);
        let names: Vec<_> = files
            .iter()
            .filter(|(_, content)| content == "root" || content == "override")
            .map(|(name, content)| (name.as_str(), content.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![("AGENTS.md", "root"), ("AGENTS.override.md", "override")]
        );
        let from_file = dir.path().join("prompt.md");
        fs::write(&from_file, "\u{feff}file prompt").unwrap();
        assert_eq!(
            resolve_prompt_input(&from_file.to_string_lossy()),
            "file prompt"
        );
        assert_eq!(
            render_system_prompt(Some(&from_file.to_string_lossy()), &[], &[]),
            "file prompt"
        );
    }
}
