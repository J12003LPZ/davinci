//! TS `packages/coding-agent/src/modes/interactive/components/external-editor.ts`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

pub struct ExternalEditor {
    pub command: String,
    pub temp_dir: PathBuf,
    pub file_path: PathBuf,
}

impl ExternalEditor {
    pub fn new(command: Option<&str>, initial: &str) -> Result<Self, String> {
        let command = command
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .or_else(|| std::env::var("VISUAL").ok().filter(|s| !s.is_empty()))
            .or_else(|| std::env::var("EDITOR").ok().filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "vi".to_string());
        let temp_dir = std::env::temp_dir().join(format!("pi-editor-{}", std::process::id()));
        fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;
        let file_path = temp_dir.join("prompt.md");
        fs::write(&file_path, initial).map_err(|e| e.to_string())?;
        Ok(Self {
            command,
            temp_dir,
            file_path,
        })
    }

    pub fn launch_message(&self) -> String {
        format!(
            "Launching external editor: {}\nPi will resume when the editor exits.\n",
            self.command
        )
    }

    pub fn edit(&self) -> Result<String, String> {
        if std::env::var("PI_EXTERNAL_EDITOR_DRY_RUN").is_ok() {
            let extra = std::env::var("PI_EXTERNAL_EDITOR_CONTENT").unwrap_or_default();
            let mut text = fs::read_to_string(&self.file_path).unwrap_or_default();
            if !extra.is_empty() {
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&extra);
            }
            return Ok(normalize(&text));
        }
        let mut parts = self.command.split_whitespace();
        let program = parts
            .next()
            .ok_or_else(|| "empty editor command".to_string())?;
        let mut cmd = Command::new(program);
        for arg in parts {
            cmd.arg(arg);
        }
        cmd.arg(&self.file_path);
        cmd.status().map_err(|e| e.to_string())?;
        let text = fs::read_to_string(&self.file_path).map_err(|e| e.to_string())?;
        Ok(normalize(&text))
    }
}

impl Drop for ExternalEditor {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file_path);
        let _ = fs::remove_dir(&self.temp_dir);
    }
}

fn normalize(text: &str) -> String {
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
    stripped.trim_end_matches('\n').to_string()
}

pub fn clipboard_text() -> Option<String> {
    if let Ok(text) = std::env::var("PI_CLIPBOARD_TEXT") {
        return Some(text);
    }
    if std::env::var("PI_CLIPBOARD_DRY_RUN").is_ok() {
        return None;
    }
    if is_wayland() {
        if let Some(text) = command_stdout("wl-paste", &["--no-newline", "--type", "text"], 1000) {
            let text = String::from_utf8_lossy(&text).into_owned();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    if let Some(text) = command_stdout("pbpaste", &[], 1000) {
        let text = String::from_utf8_lossy(&text).into_owned();
        if !text.is_empty() {
            return Some(text);
        }
    }
    if let Some(text) = command_stdout("xclip", &["-selection", "clipboard", "-o"], 1000) {
        let text = String::from_utf8_lossy(&text).into_owned();
        if !text.is_empty() {
            return Some(text);
        }
    }
    if let Some(text) = command_stdout("xsel", &["--clipboard", "--output"], 1000) {
        let text = String::from_utf8_lossy(&text).into_owned();
        if !text.is_empty() {
            return Some(text);
        }
    }
    None
}

pub fn clipboard_image_png() -> Option<Vec<u8>> {
    if let Ok(path) = std::env::var("PI_CLIPBOARD_IMAGE") {
        return fs::read(path).ok();
    }
    if std::env::var("PI_CLIPBOARD_DRY_RUN").is_ok() {
        return None;
    }
    if std::env::var("TERMUX_VERSION").is_ok() {
        return None;
    }
    if is_wayland() || is_wsl() {
        if let Some(image) = wl_paste_image() {
            return Some(image);
        }
        if let Some(image) = xclip_image() {
            return Some(image);
        }
    }
    if is_wsl() {
        if let Some(image) = powershell_image() {
            return Some(image);
        }
    }
    if !is_wayland() {
        if let Some(image) = xclip_image() {
            return Some(image);
        }
    }
    if let Some(image) = command_stdout("pngpaste", &["-"], 3000) {
        if !image.is_empty() {
            return Some(image);
        }
    }
    None
}

fn is_wayland() -> bool {
    std::env::var("WAYLAND_DISPLAY")
        .ok()
        .filter(|value| !value.is_empty())
        .is_some()
        || std::env::var("XDG_SESSION_TYPE").ok().as_deref() == Some("wayland")
}

fn is_wsl() -> bool {
    if std::env::var("WSL_DISTRO_NAME").is_ok() || std::env::var("WSLENV").is_ok() {
        return true;
    }
    std::fs::read_to_string("/proc/version")
        .map(|text| {
            let lower = text.to_ascii_lowercase();
            lower.contains("microsoft") || lower.contains("wsl")
        })
        .unwrap_or(false)
}

fn wl_paste_image() -> Option<Vec<u8>> {
    let list = command_stdout("wl-paste", &["--list-types"], 1000)?;
    let types = String::from_utf8_lossy(&list);
    let selected = select_image_mime(types.lines())?;
    let data = command_stdout("wl-paste", &["--type", &selected, "--no-newline"], 3000)?;
    if data.is_empty() {
        None
    } else {
        Some(data)
    }
}

fn xclip_image() -> Option<Vec<u8>> {
    let targets = command_stdout(
        "xclip",
        &["-selection", "clipboard", "-t", "TARGETS", "-o"],
        1000,
    )
    .unwrap_or_default();
    let listed = String::from_utf8_lossy(&targets);
    let preferred = select_image_mime(listed.lines());
    let mut try_types = Vec::new();
    if let Some(preferred) = preferred {
        try_types.push(preferred);
    }
    try_types.extend(
        ["image/png", "image/jpeg", "image/webp", "image/gif"]
            .into_iter()
            .map(str::to_string),
    );
    for mime in try_types {
        if let Some(data) = command_stdout(
            "xclip",
            &["-selection", "clipboard", "-t", &mime, "-o"],
            3000,
        ) {
            if !data.is_empty() {
                return Some(data);
            }
        }
    }
    None
}

fn powershell_image() -> Option<Vec<u8>> {
    let tmp = std::env::temp_dir().join(format!("pi-wsl-clip-{}.png", std::process::id()));
    let win = command_stdout("wslpath", &["-w", &tmp.display().to_string()], 1000)?;
    let win = String::from_utf8_lossy(&win).trim().replace('\'', "''");
    if win.is_empty() {
        return None;
    }
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $path = '{win}'; $img = [System.Windows.Forms.Clipboard]::GetImage(); if ($img) {{ $img.Save($path, [System.Drawing.Imaging.ImageFormat]::Png); Write-Output 'ok' }} else {{ Write-Output 'empty' }}"
    );
    let out = command_stdout("powershell.exe", &["-NoProfile", "-Command", &script], 5000)?;
    let ok = String::from_utf8_lossy(&out).trim() == "ok";
    let bytes = if ok { fs::read(&tmp).ok() } else { None };
    let _ = fs::remove_file(&tmp);
    bytes.filter(|data| !data.is_empty())
}

fn select_image_mime<'a>(types: impl Iterator<Item = &'a str>) -> Option<String> {
    let normalized: Vec<String> = types
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.to_ascii_lowercase())
        .collect();
    for preferred in ["image/png", "image/jpeg", "image/webp", "image/gif"] {
        if let Some(found) = normalized.iter().find(|item| item.starts_with(preferred)) {
            return Some(found.clone());
        }
    }
    normalized
        .into_iter()
        .find(|item| item.starts_with("image/"))
}

fn command_stdout(program: &str, args: &[&str], timeout_ms: u64) -> Option<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let program = program.to_string();
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_string()).collect();
    std::thread::spawn(move || {
        let output = std::process::Command::new(program).args(args).output();
        let _ = tx.send(output);
    });
    match rx.recv_timeout(std::time::Duration::from_millis(timeout_ms)) {
        Ok(Ok(output)) if output.status.success() => Some(output.stdout),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_appends_fixture_and_strips_bom() {
        std::env::set_var("PI_EXTERNAL_EDITOR_DRY_RUN", "1");
        std::env::set_var("PI_EXTERNAL_EDITOR_CONTENT", "from editor");
        let editor = ExternalEditor::new(Some("code --wait"), "draft").expect("editor");
        assert!(editor
            .launch_message()
            .contains("Launching external editor: code --wait"));
        let out = editor.edit().expect("edit");
        assert_eq!(out, "draft\nfrom editor");
        std::env::remove_var("PI_EXTERNAL_EDITOR_DRY_RUN");
        std::env::remove_var("PI_EXTERNAL_EDITOR_CONTENT");
    }

    #[test]
    fn clipboard_fixtures_win_over_live_tools() {
        std::env::set_var("PI_CLIPBOARD_DRY_RUN", "1");
        std::env::set_var("PI_CLIPBOARD_TEXT", "fixture text");
        assert_eq!(clipboard_text().as_deref(), Some("fixture text"));
        std::env::remove_var("PI_CLIPBOARD_TEXT");
        assert!(clipboard_text().is_none());
        std::env::remove_var("PI_CLIPBOARD_DRY_RUN");
    }
}
