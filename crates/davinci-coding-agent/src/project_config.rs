//! Where project configuration lives: `.davinci/<name>` first, then the
//! legacy `.pi/<name>`, decided per file. The trust check and every loader
//! use this module so they cannot disagree about what a project ships.

use std::path::{Path, PathBuf};

use crate::settings::{CONFIG_DIR_NAME, LEGACY_CONFIG_DIR_NAME};

pub fn candidates(cwd: &Path, name: &str) -> [PathBuf; 2] {
    [
        cwd.join(CONFIG_DIR_NAME).join(name),
        cwd.join(LEGACY_CONFIG_DIR_NAME).join(name),
    ]
}

/// The one file a single-file loader (settings, hooks, mcp) should read.
pub fn resolve(cwd: &Path, name: &str) -> Option<PathBuf> {
    candidates(cwd, name).into_iter().find(|path| path.exists())
}

/// Every existing location. Directory resources (skills, prompts, agents,
/// extensions) are merged from both, `.davinci` first.
pub fn all(cwd: &Path, name: &str) -> Vec<PathBuf> {
    candidates(cwd, name)
        .into_iter()
        .filter(|path| path.exists())
        .collect()
}

pub fn any_exists(cwd: &Path, names: &[&str]) -> bool {
    names
        .iter()
        .any(|name| candidates(cwd, name).iter().any(|path| path.exists()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn prefers_davinci_then_falls_back_per_file() {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        fs::create_dir_all(cwd.join(".davinci")).unwrap();
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("hooks.json"), "{}").unwrap();
        // .davinci exists but has no hooks.json: the .pi file is still found.
        assert_eq!(
            resolve(cwd, "hooks.json"),
            Some(cwd.join(".pi").join("hooks.json"))
        );
        fs::write(cwd.join(".davinci").join("hooks.json"), "{}").unwrap();
        assert_eq!(
            resolve(cwd, "hooks.json"),
            Some(cwd.join(".davinci").join("hooks.json"))
        );
    }

    #[test]
    fn all_returns_both_directories_in_order() {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        fs::create_dir_all(cwd.join(".davinci").join("skills")).unwrap();
        fs::create_dir_all(cwd.join(".pi").join("skills")).unwrap();
        assert_eq!(
            all(cwd, "skills"),
            vec![
                cwd.join(".davinci").join("skills"),
                cwd.join(".pi").join("skills")
            ]
        );
    }

    #[test]
    fn any_exists_checks_both_directories() {
        let dir = tempdir().unwrap();
        let cwd = dir.path();
        fs::create_dir_all(cwd.join(".davinci")).unwrap();
        fs::write(cwd.join(".davinci").join("README"), "").unwrap();
        assert!(!any_exists(cwd, &["mcp.json", "hooks.json"]));
        fs::create_dir_all(cwd.join(".pi")).unwrap();
        fs::write(cwd.join(".pi").join("mcp.json"), "{}").unwrap();
        assert!(any_exists(cwd, &["mcp.json", "hooks.json"]));
    }
}
