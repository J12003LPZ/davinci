//! Footer formatting matching TS `modes/interactive/components/footer.ts`
//! and git resolution from `core/footer-data-provider.ts`.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitPaths {
    pub repo_dir: PathBuf,
    pub common_git_dir: PathBuf,
    pub head_path: PathBuf,
}

/// Walk up from `cwd` to find `.git` (directory or worktree file).
pub fn find_git_paths(cwd: &Path) -> Option<GitPaths> {
    let mut dir = cwd.to_path_buf();
    loop {
        let git_path = dir.join(".git");
        if git_path.exists() {
            if git_path.is_file() {
                let content = std::fs::read_to_string(&git_path).ok()?.trim().to_string();
                if let Some(rest) = content.strip_prefix("gitdir: ") {
                    let git_dir = dir.join(rest.trim());
                    let head_path = git_dir.join("HEAD");
                    if !head_path.exists() {
                        return None;
                    }
                    let common_dir_path = git_dir.join("commondir");
                    let common_git_dir = if common_dir_path.exists() {
                        git_dir.join(std::fs::read_to_string(common_dir_path).ok()?.trim())
                    } else {
                        git_dir.clone()
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
        if !dir.pop() {
            return None;
        }
    }
}

/// Current git branch, `None` if not in a repo, `"detached"` if detached HEAD.
pub fn resolve_git_branch(cwd: &Path) -> Option<String> {
    let paths = find_git_paths(cwd)?;
    let content = std::fs::read_to_string(&paths.head_path)
        .ok()?
        .trim()
        .to_string();
    if let Some(branch) = content.strip_prefix("ref: refs/heads/") {
        if branch == ".invalid" {
            return Some("detached".into());
        }
        return Some(branch.to_string());
    }
    Some("detached".into())
}

/// TS `formatCwdForFooter`.
pub fn format_cwd_for_footer(cwd: &str, home: Option<&str>) -> String {
    let Some(home) = home.filter(|value| !value.is_empty()) else {
        return cwd.to_string();
    };
    let cwd_path = Path::new(cwd);
    let home_path = Path::new(home);
    if cwd_path == home_path {
        return "~".into();
    }
    if let Ok(relative) = cwd_path.strip_prefix(home_path) {
        let rel = relative.to_string_lossy().replace('\\', "/");
        if rel.is_empty() {
            return "~".into();
        }
        return format!("~/{rel}");
    }
    cwd.to_string()
}

pub fn format_pwd_line(
    cwd: &str,
    home: Option<&str>,
    branch: Option<&str>,
    session_name: Option<&str>,
) -> String {
    let mut pwd = format_cwd_for_footer(cwd, home);
    if let Some(branch) = branch.filter(|value| !value.is_empty()) {
        pwd = format!("{pwd} ({branch})");
    }
    if let Some(name) = session_name.filter(|value| !value.is_empty()) {
        pwd = format!("{pwd} • {name}");
    }
    pwd
}

pub fn truncate_to_width(text: &str, width: usize, ellipsis: &str) -> String {
    crate::ansi::truncate_to_width(text, width, ellipsis, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn format_cwd_matches_ts_home_rules() {
        assert_eq!(
            format_cwd_for_footer("/home/user2", Some("/home/user")),
            "/home/user2"
        );
        assert_eq!(format_cwd_for_footer("/home/user", Some("/home/user")), "~");
        assert_eq!(
            format_cwd_for_footer("/home/user/project", Some("/home/user")),
            "~/project"
        );
        assert_eq!(
            format_pwd_line(
                "/home/user/project",
                Some("/home/user"),
                Some("main"),
                Some("work")
            ),
            "~/project (main) • work"
        );
    }

    #[test]
    fn find_git_paths_reads_head_and_worktree() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git").join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        assert_eq!(resolve_git_branch(&repo).as_deref(), Some("feature"));
        std::fs::create_dir_all(repo.join("nested")).unwrap();
        assert_eq!(
            resolve_git_branch(&repo.join("nested")).as_deref(),
            Some("feature")
        );

        std::fs::write(repo.join(".git").join("HEAD"), "abc123\n").unwrap();
        assert_eq!(resolve_git_branch(&repo).as_deref(), Some("detached"));

        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let gitdir = dir.path().join("gitdir");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::write(gitdir.join("HEAD"), "ref: refs/heads/worktree\n").unwrap();
        std::fs::write(work.join(".git"), format!("gitdir: {}\n", gitdir.display())).unwrap();
        assert_eq!(resolve_git_branch(&work).as_deref(), Some("worktree"));
    }
}
