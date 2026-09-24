use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedSource {
    Local(String),
    Npm { spec: String, name: String },
    Git(String),
}

pub fn parse_package_source(source: &str) -> ParsedSource {
    if let Some(spec) = source.strip_prefix("npm:") {
        let spec = spec.trim();
        let name = spec
            .split_once('@')
            .filter(|(head, _)| !head.is_empty() || spec.starts_with('@'))
            .map(|(head, _)| {
                if spec.starts_with('@') {
                    let rest = spec.trim_start_matches('@');
                    rest.split_once('@')
                        .map(|(pkg, _)| format!("@{pkg}"))
                        .unwrap_or_else(|| spec.to_string())
                } else {
                    head.to_string()
                }
            })
            .unwrap_or_else(|| spec.to_string());
        return ParsedSource::Npm {
            spec: spec.to_string(),
            name,
        };
    }
    if source.starts_with("git:")
        || source.starts_with("git@")
        || source.ends_with(".git")
        || source.contains("github.com")
        || source.starts_with("https://")
        || source.starts_with("ssh://")
    {
        return ParsedSource::Git(source.to_string());
    }
    ParsedSource::Local(source.to_string())
}

pub fn parse_git_source(source: &str) -> (String, Option<String>) {
    let raw = source.strip_prefix("git:").unwrap_or(source);
    if raw.starts_with("git@") {
        if let Some(idx) = raw.rfind('@') {
            if idx > 3 {
                return (raw[..idx].to_string(), Some(raw[idx + 1..].to_string()));
            }
        }
        return (raw.to_string(), None);
    }
    if let Some(idx) = raw.rfind('@') {
        let spec = &raw[idx + 1..];
        if !spec.contains('/') && !spec.contains(':') {
            return (raw[..idx].to_string(), Some(spec.to_string()));
        }
    }
    (raw.to_string(), None)
}

pub fn npm_install_args(manager: &str, specs: &[String], install_root: &Path) -> Vec<String> {
    let mut args = vec!["install".into()];
    args.extend(specs.iter().cloned());
    match manager {
        "pnpm" => {
            args.push("--prefix".into());
            args.push(install_root.display().to_string());
            args.push("--config.auto-install-peers=false".into());
            args.push("--config.strict-peer-dependencies=false".into());
            args.push("--config.strict-dep-builds=false".into());
        }
        "bun" => {
            args.push("--cwd".into());
            args.push(install_root.display().to_string());
            args.push("--omit=peer".into());
        }
        _ => {
            args.push("--prefix".into());
            args.push(install_root.display().to_string());
            args.push("--legacy-peer-deps".into());
        }
    }
    args
}

pub fn npm_install_root(agent_dir: &Path, local: bool, cwd: &Path) -> PathBuf {
    if local {
        cwd.join(".pi").join("npm")
    } else {
        agent_dir.join("npm")
    }
}

pub fn git_install_root(agent_dir: &Path, local: bool, cwd: &Path) -> PathBuf {
    if local {
        cwd.join(".pi").join("git")
    } else {
        agent_dir.join("git")
    }
}


pub fn git_checkout_path(
    agent_dir: &Path,
    local: bool,
    cwd: &Path,
    spec: &str,
) -> Result<PathBuf, String> {
    let (url, _) = parse_git_source(spec);
    // Inspect the raw path before URL or filesystem normalization can erase
    // parent components. A checkout may subsequently be recursively replaced.
    let raw = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("ssh://"))
        .unwrap_or(&url);
    let (host, path) = if let Some(scp) = raw.strip_prefix("git@") {
        if !url.contains("://") {
            scp.split_once(':')
        } else {
            raw.split_once('/')
        }
    } else {
        raw.split_once('/')
    }
    .ok_or("Invalid Git checkout path")?;
    if host.is_empty() || path.contains(':') {
        return Err("Invalid Git checkout path".into());
    }
    let host_path = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("ssh://")
        .trim_start_matches("git@")
        .replace(':', "/");
    let relative = host_path.trim_end_matches(".git");
    for part in relative.split('/') {
        let device = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "<>:\"\\|?*".contains(c))
            || matches!(
                device.as_str(),
                "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
            )
            || ["COM", "LPT"].iter().any(|prefix| {
                device.strip_prefix(prefix).is_some_and(|number| {
                    matches!(number, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
                })
            })
        {
            return Err("Invalid Git checkout path".into());
        }
    }
    let root = git_install_root(agent_dir, local, cwd);
    let destination = root.join(relative);
    if davinci_agent::check_path_boundary(&root, &destination) != (false, false) {
        return Err("Git checkout escapes the package directory".into());
    }
    Ok(destination)
}


pub fn expand_local(path: &str, cwd: &Path) -> PathBuf {
    let expanded = match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"))
    {
        Some(rest) => davinci_session::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(path)),
        None => PathBuf::from(path),
    };
    if expanded.is_absolute() {
        expanded
    } else {
        cwd.join(expanded)
    }
}

pub fn installed_root(source: &str, agent_dir: &Path, cwd: &Path) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = match parse_package_source(source) {
        ParsedSource::Local(path) => vec![expand_local(&path, cwd)],
        ParsedSource::Npm { name, .. } => [false, true]
            .into_iter()
            .map(|local| npm_install_root(agent_dir, local, cwd).join("node_modules").join(&name))
            .collect(),
        ParsedSource::Git(_) => [false, true]
            .into_iter()
            .filter_map(|local| git_checkout_path(agent_dir, local, cwd, source).ok())
            .collect(),
    };
    candidates.into_iter().find(|path| path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_root_finds_global_then_project_installs() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("project");
        let global = agent
            .join("npm")
            .join("node_modules")
            .join("@scope")
            .join("tool");
        std::fs::create_dir_all(&global).unwrap();
        assert_eq!(
            installed_root("npm:@scope/tool@1.2.3", &agent, &cwd),
            Some(global)
        );
        let local = cwd
            .join(".pi")
            .join("npm")
            .join("node_modules")
            .join("other");
        std::fs::create_dir_all(&local).unwrap();
        assert_eq!(installed_root("npm:other", &agent, &cwd), Some(local));
        assert_eq!(installed_root("npm:missing", &agent, &cwd), None);
    }

    #[test]
    fn local_sources_resolve_to_their_path() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            installed_root(
                &dir.path().display().to_string(),
                dir.path(),
                dir.path()
            ),
            Some(dir.path().to_path_buf())
        );
    }
}
