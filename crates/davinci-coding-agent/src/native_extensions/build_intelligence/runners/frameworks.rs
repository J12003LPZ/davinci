use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameworkKind {
    Vite,
    NextJs,
}

pub fn detect_framework(dir: &Path) -> Option<FrameworkKind> {
    let vite_candidates = [
        "vite.config.ts",
        "vite.config.js",
        "vite.config.mjs",
        "vite.config.cjs",
    ];
    for c in vite_candidates {
        if dir.join(c).is_file() {
            return Some(FrameworkKind::Vite);
        }
    }

    let next_candidates = [
        "next.config.js",
        "next.config.mjs",
        "next.config.ts",
        "next.config.cjs",
    ];
    for c in next_candidates {
        if dir.join(c).is_file() {
            return Some(FrameworkKind::NextJs);
        }
    }

    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageManager {
    Pnpm,
    Yarn,
    Bun,
    Npm,
}

impl PackageManager {
    pub fn as_str(&self) -> &'static str {
        match self {
            PackageManager::Pnpm => "pnpm",
            PackageManager::Yarn => "yarn",
            PackageManager::Bun => "bun",
            PackageManager::Npm => "npm",
        }
    }

    pub fn run_script_argv(&self, script: &str) -> Vec<String> {
        match self {
            PackageManager::Pnpm => vec!["pnpm".to_string(), "run".to_string(), script.to_string()],
            PackageManager::Yarn => vec!["yarn".to_string(), "run".to_string(), script.to_string()],
            PackageManager::Bun => vec!["bun".to_string(), "run".to_string(), script.to_string()],
            PackageManager::Npm => vec!["npm".to_string(), "run".to_string(), script.to_string()],
        }
    }
}

pub fn detect_package_manager(root: &Path) -> PackageManager {
    if root.join("pnpm-lock.yaml").is_file() || root.join("pnpm-workspace.yaml").is_file() {
        PackageManager::Pnpm
    } else if root.join("bun.lockb").is_file() || root.join("bunfig.toml").is_file() {
        PackageManager::Bun
    } else if root.join("yarn.lock").is_file() {
        PackageManager::Yarn
    } else {
        PackageManager::Npm
    }
}
