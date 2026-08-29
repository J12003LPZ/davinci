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
    None
}

pub fn clipboard_image_png() -> Option<Vec<u8>> {
    if let Ok(path) = std::env::var("PI_CLIPBOARD_IMAGE") {
        return fs::read(path).ok();
    }
    None
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
}
