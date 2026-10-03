//! Shared agent directory selection; does not read or migrate credentials.
use std::path::PathBuf;

pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// TS `os.homedir()`: libuv reads `USERPROFILE` on Windows and `HOME` on
/// POSIX. `HOME` stays as a Windows fallback for MSYS/Git Bash shells.
pub fn home_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn default_agent_dir() -> PathBuf {
    if let Ok(dir) =
        std::env::var("DAVINCI_CODING_AGENT_DIR").or_else(|_| std::env::var("PI_CODING_AGENT_DIR"))
    {
        return expand_tilde(&dir);
    }
    let home = home_dir().unwrap_or_else(|| PathBuf::from("."));
    let davinci_dir = home.join(".davinci").join("agent");
    if davinci_dir.exists() {
        return davinci_dir;
    }
    let pi_dir = home.join(".pi").join("agent");
    if pi_dir.exists() {
        return pi_dir;
    }
    davinci_dir
}
