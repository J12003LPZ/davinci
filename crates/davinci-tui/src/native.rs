//! Native TUI helper resolution matching `native-module-path.ts` and `native-modifiers.ts`.

use std::path::{Path, PathBuf};

pub const TUI_PACKAGE_NAME: &str = "@earendil-works/pi-tui";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModifierKey {
    Shift,
    Command,
    Control,
    Option,
}

impl ModifierKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shift => "shift",
            Self::Command => "command",
            Self::Control => "control",
            Self::Option => "option",
        }
    }

    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "shift" => Some(Self::Shift),
            "command" | "super" | "win" => Some(Self::Command),
            "control" => Some(Self::Control),
            "option" | "alt" => Some(Self::Option),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeModuleCandidateOptions {
    pub module_dir: PathBuf,
    pub exec_path: PathBuf,
    pub package_entry: Option<PathBuf>,
}

/// TS `getNativeModuleCandidates`.
pub fn get_native_module_candidates(
    native_path: &str,
    options: NativeModuleCandidateOptions,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(package_entry) = options.package_entry {
        if let Some(parent) = package_entry.parent() {
            candidates.push(parent.join("..").join(native_path));
        }
    }
    candidates.push(options.module_dir.join("..").join(native_path));
    candidates.push(options.module_dir.join(native_path));
    if let Some(exec_dir) = options.exec_path.parent() {
        candidates.push(exec_dir.join(native_path));
    }
    let mut unique = Vec::new();
    for candidate in candidates {
        let normalized = normalize_path(&candidate);
        if !unique.contains(&normalized) {
            unique.push(normalized);
        }
    }
    unique
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

pub fn native_helper_path(platform: &str, arch: &str) -> Option<String> {
    if arch != "x64" && arch != "arm64" {
        return None;
    }
    match platform {
        "darwin" => Some(format!(
            "native/darwin/prebuilds/darwin-{arch}/darwin-modifiers.node"
        )),
        "win32" | "windows" => Some(format!(
            "native/win32/prebuilds/win32-{arch}/win32-console-mode.node"
        )),
        _ => None,
    }
}

/// TS `isNativeModifierPressed` with fixture `PI_TUI_NATIVE_MODIFIER_{SHIFT,COMMAND,CONTROL,OPTION}`.
pub fn is_native_modifier_pressed(key: ModifierKey) -> bool {
    let env_name = match key {
        ModifierKey::Shift => "PI_TUI_NATIVE_MODIFIER_SHIFT",
        ModifierKey::Command => "PI_TUI_NATIVE_MODIFIER_COMMAND",
        ModifierKey::Control => "PI_TUI_NATIVE_MODIFIER_CONTROL",
        ModifierKey::Option => "PI_TUI_NATIVE_MODIFIER_OPTION",
    };
    matches!(
        std::env::var(env_name).as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

pub fn enable_virtual_terminal_input() -> bool {
    if cfg!(windows) {
        matches!(
            std::env::var("PI_TUI_ENABLE_VT_INPUT").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        )
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_module_candidates_match_ts() {
        let native_path = "native/win32/prebuilds/win32-arm64/win32-console-mode.node";
        let package_root = PathBuf::from("/virtual/node_modules/@earendil-works/pi-tui");
        let bundled = PathBuf::from("/virtual/pi-coding-agent/dist/bundle/chunks");
        let candidates = get_native_module_candidates(
            native_path,
            NativeModuleCandidateOptions {
                module_dir: bundled.clone(),
                exec_path: PathBuf::from("/virtual/node/node.exe"),
                package_entry: Some(package_root.join("dist/index.js")),
            },
        );
        assert_eq!(candidates[0], package_root.join(native_path));
        assert!(candidates.contains(&normalize_path(&bundled.join("..").join(native_path))));

        let bundled = PathBuf::from("/virtual/pi/bundle/chunks");
        let exec = PathBuf::from("/virtual/pi/pi.exe");
        let native_path = "native/darwin/prebuilds/darwin-arm64/darwin-modifiers.node";
        let candidates = get_native_module_candidates(
            native_path,
            NativeModuleCandidateOptions {
                module_dir: bundled.clone(),
                exec_path: exec.clone(),
                package_entry: None,
            },
        );
        assert_eq!(
            candidates,
            [
                normalize_path(&bundled.join("..").join(native_path)),
                bundled.join(native_path),
                exec.parent().unwrap().join(native_path),
            ]
        );
        assert_eq!(
            native_helper_path("darwin", "arm64").as_deref(),
            Some("native/darwin/prebuilds/darwin-arm64/darwin-modifiers.node")
        );
        assert_eq!(
            native_helper_path("win32", "x64").as_deref(),
            Some("native/win32/prebuilds/win32-x64/win32-console-mode.node")
        );
        assert_eq!(native_helper_path("linux", "x64"), None);
        assert_eq!(ModifierKey::parse("alt"), Some(ModifierKey::Option));
        assert_eq!(ModifierKey::parse("win"), Some(ModifierKey::Command));
    }
}
