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

fn canonicalize(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPaths {
    pub repo_dir: PathBuf,
    pub common_git_dir: PathBuf,
    pub head_path: PathBuf,
}

pub fn find_git_paths(cwd: &Path) -> Option<GitPaths> {
    let mut dir = canonicalize(cwd);
    loop {
        let git_path = dir.join(".git");
        if git_path.exists() {
            if git_path.is_file() {
                let content = fs::read_to_string(&git_path).ok()?;
                let content = content.trim();
                if let Some(rest) = content.strip_prefix("gitdir: ") {
                    let git_dir = canonicalize(&dir.join(rest.trim()));
                    let head_path = git_dir.join("HEAD");
                    if !head_path.exists() {
                        return None;
                    }
                    let common_dir_path = git_dir.join("commondir");
                    let common_git_dir = if common_dir_path.exists() {
                        let rel = fs::read_to_string(&common_dir_path).ok()?;
                        canonicalize(&git_dir.join(rel.trim()))
                    } else {
                        git_dir
                    };
                    return Some(GitPaths {
                        repo_dir: dir,
                        common_git_dir,
                        head_path,
                    });
                }
            } else if git_path.is_dir() {
                let head_path = git_path.join("HEAD");
                if !head_path.exists() {
                    return None;
                }
                return Some(GitPaths {
                    repo_dir: dir,
                    common_git_dir: git_path,
                    head_path,
                });
            }
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => return None,
        }
    }
}

pub fn find_shadowed_context_file(cwd: &Path) -> Option<PathBuf> {
    let git_paths = find_git_paths(cwd)?;
    let common_git_dir = canonicalize(&git_paths.common_git_dir);
    let worktree_root = canonicalize(&git_paths.repo_dir);
    let main_repo_root = common_git_dir.parent()?.to_path_buf();
    if !worktree_root.starts_with(&main_repo_root) || worktree_root == main_repo_root {
        return None;
    }
    if canonicalize(&main_repo_root.join(".git")) != common_git_dir {
        return None;
    }
    let worktree_context = load_context_file_from_dir(&worktree_root)?;
    Some(main_repo_root.join(worktree_context.0))
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
    let shadowed = find_shadowed_context_file(cwd).map(|path| canonicalize(&path));
    let mut ancestors = Vec::new();
    let mut dir = cwd.to_path_buf();
    loop {
        if let Some((name, content, path)) = load_context_file_from_dir(&dir) {
            let resolved = canonicalize(&path);
            let is_shadowed = shadowed.as_ref().is_some_and(|shadow| shadow == &resolved);
            if !is_shadowed
                && !seen
                    .iter()
                    .any(|existing| existing == &path || canonicalize(existing) == resolved)
            {
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

    #[test]
    fn shadows_main_repo_context_for_nested_worktree() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        let feat = repo.join("worktrees").join("feat");
        let git_worktree = repo.join(".git").join("worktrees").join("feat");
        fs::create_dir_all(&feat).unwrap();
        fs::create_dir_all(&git_worktree).unwrap();
        fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git_worktree.join("HEAD"), "ref: refs/heads/feat\n").unwrap();
        fs::write(git_worktree.join("commondir"), "../..\n").unwrap();
        fs::write(
            feat.join(".git"),
            format!("gitdir: {}\n", git_worktree.display()),
        )
        .unwrap();
        fs::write(repo.join("AGENTS.md"), "main context").unwrap();
        fs::write(feat.join("AGENTS.md"), "worktree context").unwrap();
        let shadowed = find_shadowed_context_file(&feat).unwrap();
        assert_eq!(
            canonicalize(&shadowed),
            canonicalize(&repo.join("AGENTS.md"))
        );
        let files = load_project_context_files(&feat, None, false);
        let ours: Vec<_> = files
            .iter()
            .filter(|(_, content)| content.contains("context"))
            .map(|(name, content)| (name.as_str(), content.as_str()))
            .collect();
        assert_eq!(ours, vec![("AGENTS.md", "worktree context")]);
    }
}
