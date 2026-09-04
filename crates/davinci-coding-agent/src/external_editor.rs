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
        return fs::read(path).ok().map(normalize_clipboard_image);
    }
    if std::env::var("PI_CLIPBOARD_DRY_RUN").is_ok() {
        return None;
    }
    if std::env::var("TERMUX_VERSION").is_ok() {
        return None;
    }
    if is_wayland() || is_wsl() {
        if let Some(image) = wl_paste_image() {
            return Some(normalize_clipboard_image(image));
        }
        if let Some(image) = xclip_image() {
            return Some(normalize_clipboard_image(image));
        }
    }
    if is_wsl() {
        if let Some(image) = powershell_image() {
            return Some(normalize_clipboard_image(image));
        }
    }
    if !is_wayland() {
        if let Some(image) = xclip_image() {
            return Some(normalize_clipboard_image(image));
        }
    }
    if let Some(image) = command_stdout("pngpaste", &["-"], 3000) {
        if !image.is_empty() {
            return Some(normalize_clipboard_image(image));
        }
    }
    None
}

fn normalize_clipboard_image(bytes: Vec<u8>) -> Vec<u8> {
    if let Some(png) = crate::image_convert::convert_image_bytes_to_png(&bytes) {
        return png;
    }
    if bytes.starts_with(b"BM") {
        if let Some(png) = bmp_to_png(&bytes) {
            return png;
        }
    }
    bytes
}

fn bmp_to_png(bmp: &[u8]) -> Option<Vec<u8>> {
    if bmp.len() < 54 || &bmp[0..2] != b"BM" {
        return None;
    }
    let data_offset = u32::from_le_bytes(bmp[10..14].try_into().ok()?) as usize;
    let width = i32::from_le_bytes(bmp[18..22].try_into().ok()?) as usize;
    let height_signed = i32::from_le_bytes(bmp[22..26].try_into().ok()?);
    let bottom_up = height_signed > 0;
    let height = height_signed.unsigned_abs() as usize;
    let bpp = u16::from_le_bytes(bmp[28..30].try_into().ok()?);
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }
    let bytes_per_pixel = match bpp {
        24 => 3,
        32 => 4,
        _ => return None,
    };
    let row_stride = (width * bytes_per_pixel + 3) & !3;
    let mut rgba = vec![0u8; width * height * 4];
    for y in 0..height {
        let src_y = if bottom_up { height - 1 - y } else { y };
        let src = data_offset.checked_add(src_y * row_stride)?;
        for x in 0..width {
            let px = src + x * bytes_per_pixel;
            if px + 2 >= bmp.len() {
                return None;
            }
            let dest = (y * width + x) * 4;
            rgba[dest] = bmp[px + 2];
            rgba[dest + 1] = bmp[px + 1];
            rgba[dest + 2] = bmp[px];
            rgba[dest + 3] = if bytes_per_pixel == 4 {
                *bmp.get(px + 3).unwrap_or(&255)
            } else {
                255
            };
        }
    }
    Some(encode_png(width as u32, height as u32, &rgba))
}

fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((width as usize) * 4 + 1) * height as usize);
    for row in rgba.chunks(width as usize * 4) {
        raw.push(0);
        raw.extend_from_slice(row);
    }
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut png = Vec::from([137, 80, 78, 71, 13, 10, 26, 10]);
    write_png_chunk(&mut png, b"IHDR", &ihdr);
    write_png_chunk(&mut png, b"IDAT", &zlib_store(&raw));
    write_png_chunk(&mut png, b"IEND", &[]);
    png
}

fn write_png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = 0xffff_ffffu32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ mask;
        }
    }
    out.extend_from_slice(&(crc ^ 0xffff_ffff).to_be_bytes());
}

fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut offset = 0;
    while offset < data.len() {
        let end = (offset + 65535).min(data.len());
        let chunk = &data[offset..end];
        let last = end == data.len();
        out.push(if last { 1 } else { 0 });
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
        offset = end;
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for byte in data {
        a = (a + u32::from(*byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
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
    fn clipboard_bmp_converts_to_png() {
        let dir = tempfile::tempdir().expect("temp");
        let path = dir.path().join("clip.bmp");
        std::fs::write(&path, one_pixel_bmp()).expect("write bmp");
        std::env::set_var("PI_CLIPBOARD_IMAGE", path.display().to_string());
        let png = clipboard_image_png().expect("png");
        assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(png.len() > 8);
        std::env::remove_var("PI_CLIPBOARD_IMAGE");
    }

    fn one_pixel_bmp() -> Vec<u8> {
        let mut bmp = vec![0u8; 58];
        bmp[0..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
        bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
        bmp[18..22].copy_from_slice(&1i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
        bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
        bmp[54] = 0;
        bmp[55] = 128;
        bmp[56] = 255;
        bmp
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
