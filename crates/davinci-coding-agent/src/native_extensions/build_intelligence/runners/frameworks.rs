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

    /// Arguments (without the program) that run `script` for `packages`, or
    /// for the root when none are selected. `None` when the manager has no
    /// workspace-targeted form for the selection.
    pub fn run_script_argv(&self, script: &str, packages: &[String]) -> Option<Vec<String>> {
        let run = |argv: &mut Vec<String>| {
            argv.push("run".to_string());
            argv.push(script.to_string());
        };
        let mut argv = Vec::new();
        if packages.is_empty() {
            run(&mut argv);
            return Some(argv);
        }
        match self {
            PackageManager::Pnpm => {
                for pkg in packages {
                    argv.push(format!("--filter={pkg}..."));
                }
                run(&mut argv);
            }
            PackageManager::Npm => {
                run(&mut argv);
                for pkg in packages {
                    argv.push(format!("--workspace={pkg}"));
                }
            }
            PackageManager::Bun => {
                for pkg in packages {
                    argv.push("--filter".to_string());
                    argv.push(pkg.clone());
                }
                run(&mut argv);
            }
            PackageManager::Yarn => {
                if let [only] = packages {
                    argv.extend(["workspace".to_string(), only.clone()]);
                } else {
                    argv.extend(["workspaces".to_string(), "foreach".to_string()]);
                    for pkg in packages {
                        argv.push("--include".to_string());
                        argv.push(pkg.clone());
                    }
                }
                run(&mut argv);
            }
        }
        Some(argv)
    }
}

/// The manager named by the root manifest's Corepack `packageManager` field.
fn declared_package_manager(root: &Path) -> Option<PackageManager> {
    let content = std::fs::read_to_string(root.join("package.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&content).ok()?;
    let declared = manifest.get("packageManager")?.as_str()?;
    match declared.split('@').next()? {
        "pnpm" => Some(PackageManager::Pnpm),
        "yarn" => Some(PackageManager::Yarn),
        "bun" => Some(PackageManager::Bun),
        "npm" => Some(PackageManager::Npm),
        _ => None,
    }
}

pub fn detect_package_manager(root: &Path) -> PackageManager {
    if let Some(declared) = declared_package_manager(root) {
        return declared;
    }
    if root.join("pnpm-lock.yaml").is_file() || root.join("pnpm-workspace.yaml").is_file() {
        PackageManager::Pnpm
    } else if root.join("bun.lockb").is_file()
        || root.join("bun.lock").is_file()
        || root.join("bunfig.toml").is_file()
    {
        PackageManager::Bun
    } else if root.join("yarn.lock").is_file() {
        PackageManager::Yarn
    } else {
        PackageManager::Npm
    }
}
