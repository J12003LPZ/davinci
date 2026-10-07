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
        let parts = split_command_line(&self.command)?;
        let (program, args) = parts
            .split_first()
            .ok_or_else(|| "empty editor command".to_string())?;
        let mut cmd = Command::new(davinci_sys::process::resolve_program(program));
        cmd.args(args).arg(&self.file_path);
        let status = cmd.status().map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(format!(
                "editor exited with {status}; the edit was discarded"
            ));
        }
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

pub fn split_command_line(line: &str) -> Result<Vec<String>, String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;

    while let Some(ch) = chars.next() {
        match quote {
            Some('\'') => {
                if ch == '\'' {
                    quote = None;
                } else {
                    current.push(ch);
                }
            }
            Some('"') => {
                if ch == '"' {
                    quote = None;
                } else if ch == '\\' && chars.peek() == Some(&'"') {
                    current.push(chars.next().unwrap());
                } else {
                    current.push(ch);
                }
            }
            None => match ch {
                '\'' | '"' => quote = Some(ch),
                '\\' => {
                    // Preserve ordinary Windows path separators (for example
                    // C:\\Tools\\vim.exe). Only consume the backslash as an
                    // escape when it actually quotes a shell-like separator.
                    match chars.peek().copied() {
                        Some(next)
                            if next == '\\'
                                || next == '\''
                                || next == '"'
                                || next.is_whitespace() =>
                        {
                            current.push(chars.next().unwrap());
                        }
                        _ => current.push('\\'),
                    }
                }
                ch if ch.is_whitespace() => {
                    if !current.is_empty() {
                        parts.push(std::mem::take(&mut current));
                    }
                }
                _ => current.push(ch),
            },
            _ => unreachable!(),
        }
    }
    if quote.is_some() {
        return Err("unterminated quote in editor command".into());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    Ok(parts)
}

fn normalize(text: &str) -> String {
    let stripped = text.strip_prefix('\u{feff}').unwrap_or(text);
    stripped
        .replace("\r\n", "\n")
        .trim_end_matches('\n')
        .to_string()
}

pub fn clipboard_text() -> Option<String> {
    if let Ok(text) = std::env::var("PI_CLIPBOARD_TEXT") {
        return Some(text);
    }
    if std::env::var("PI_CLIPBOARD_DRY_RUN").is_ok() {
        return None;
    }
    if cfg!(windows) {
        return windows_clipboard_text();
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
    if cfg!(windows) {
        return windows_clipboard_image().map(normalize_clipboard_image);
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
    let out = command_stdout(
        "powershell.exe",
        &["-NoProfile", "-STA", "-Command", CLIPBOARD_IMAGE_SCRIPT],
        5000,
    )?;
    decode_clipboard_script_output(&out)
}

/// Windows PowerShell, from WSL: the clipboard image as base64 PNG on stdout
/// (a bitmap, or else the first image file in a copied file list), or
/// nothing. No path goes into the script, so nothing needs quoting.
const CLIPBOARD_IMAGE_SCRIPT: &str = concat!(
    "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; ",
    "$img = [System.Windows.Forms.Clipboard]::GetImage(); ",
    "if (-not $img) { foreach ($f in [System.Windows.Forms.Clipboard]::GetFileDropList()) { ",
    r"if ($f -match '\.(png|jpe?g|gif|bmp|webp|tiff?)$') { ",
    "try { $img = [System.Drawing.Image]::FromFile($f) } catch { }; break } } }; ",
    "if ($img) { $ms = New-Object System.IO.MemoryStream; ",
    "$img.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $img.Dispose(); ",
    "[Convert]::ToBase64String($ms.ToArray()) }",
);

fn decode_clipboard_script_output(out: &[u8]) -> Option<Vec<u8>> {
    let text = String::from_utf8_lossy(out);
    let encoded = text.trim();
    if encoded.is_empty() {
        return None;
    }
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
        .ok()
        .filter(|data| !data.is_empty())
}

/// Native Windows: the clipboard read in-process, as Claude Code does — a
/// bitmap (a screenshot), or else the first image file in a copied file list
/// (an image copied in Explorer). No helper process, so alt+v is instant.
#[cfg(windows)]
fn windows_clipboard_image() -> Option<Vec<u8>> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    if let Ok(image) = clipboard.get_image() {
        return rgba_png(
            image.width as u32,
            image.height as u32,
            image.bytes.into_owned(),
        );
    }
    let files = clipboard.get().file_list().ok()?;
    let path = files.iter().find(|path| is_image_file(path))?;
    copied_image_png(path)
}

/// A copied image file, decoded and re-encoded as PNG so what is sent is
/// what it is labelled; one that will not decode, or is past the size an
/// upload can take, is not attached.
#[cfg_attr(not(windows), allow(dead_code))]
fn copied_image_png(path: &std::path::Path) -> Option<Vec<u8>> {
    const MAX_BYTES: u64 = 50 * 1024 * 1024;
    if fs::metadata(path).ok()?.len() > MAX_BYTES {
        return None;
    }
    let image = image::open(path).ok()?.to_rgba8();
    let (width, height) = image.dimensions();
    rgba_png(width, height, image.into_raw())
}

#[cfg(not(windows))]
fn windows_clipboard_image() -> Option<Vec<u8>> {
    None
}

#[cfg(windows)]
fn windows_clipboard_text() -> Option<String> {
    arboard::Clipboard::new()
        .ok()?
        .get_text()
        .ok()
        .filter(|text| !text.is_empty())
}

#[cfg(not(windows))]
fn windows_clipboard_text() -> Option<String> {
    None
}

/// Raw RGBA pixels, as the clipboard hands them over, encoded as PNG.
#[cfg_attr(not(windows), allow(dead_code))]
fn rgba_png(width: u32, height: u32, rgba: Vec<u8>) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut png = std::io::Cursor::new(Vec::new());
    image.write_to(&mut png, image::ImageFormat::Png).ok()?;
    Some(png.into_inner())
}

#[cfg_attr(not(windows), allow(dead_code))]
fn is_image_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| {
            matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff"
            )
        })
}

#[cfg(test)]
mod clipboard_script_tests {
    use super::*;

    #[test]
    fn the_wsl_script_takes_no_path_and_accepts_copied_image_files() {
        // Nothing interpolated: no quoting to get wrong, whatever TEMP holds.
        assert!(!CLIPBOARD_IMAGE_SCRIPT.contains("$path"));
        assert!(CLIPBOARD_IMAGE_SCRIPT.contains("GetImage()"));
        assert!(CLIPBOARD_IMAGE_SCRIPT.contains("GetFileDropList()"));
        assert!(CLIPBOARD_IMAGE_SCRIPT.contains("ToBase64String"));
        assert!(CLIPBOARD_IMAGE_SCRIPT.contains(r"'\.(png|jpe?g|gif|bmp|webp|tiff?)$'"));
    }

    #[test]
    fn script_output_decodes_to_the_image_or_nothing() {
        assert_eq!(
            decode_clipboard_script_output(b"iVBORw0K\r\n"),
            Some(vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a])
        );
        assert_eq!(decode_clipboard_script_output(b"  \r\n"), None);
        assert_eq!(decode_clipboard_script_output(b"not base64!"), None);
    }

    #[test]
    fn clipboard_pixels_become_a_png() {
        let png = rgba_png(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let decoded = image::load_from_memory(&png).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
        // A buffer that does not match the size is refused, not misread.
        assert!(rgba_png(2, 2, vec![0; 8]).is_none());
    }

    #[test]
    fn a_copied_image_file_is_sent_as_png_and_a_broken_one_not_at_all() {
        let dir = tempfile::tempdir().unwrap();
        let bmp = dir.path().join("shot.bmp");
        image::RgbaImage::from_raw(1, 1, vec![1, 2, 3, 255])
            .unwrap()
            .save(&bmp)
            .unwrap();
        let png = copied_image_png(&bmp).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let fake = dir.path().join("fake.png");
        fs::write(&fake, b"not an image").unwrap();
        assert!(copied_image_png(&fake).is_none());
        assert!(copied_image_png(&dir.path().join("missing.png")).is_none());
    }

    #[test]
    fn copied_files_count_only_when_they_are_images() {
        assert!(is_image_file(std::path::Path::new(
            r"C:\shots\Screen Shot.PNG"
        )));
        assert!(is_image_file(std::path::Path::new("a/b.jpeg")));
        assert!(!is_image_file(std::path::Path::new("notes.txt")));
        assert!(!is_image_file(std::path::Path::new("png")));
    }
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
    fn editor_command_lines_split_like_a_shell() {
        assert_eq!(
            split_command_line("code --wait").unwrap(),
            ["code", "--wait"]
        );
        assert_eq!(
            split_command_line(r#""C:\Program Files\Sublime\subl.exe" -w"#).unwrap(),
            [r"C:\Program Files\Sublime\subl.exe", "-w"]
        );
        assert_eq!(
            split_command_line("vim -c 'set tw=72'").unwrap(),
            ["vim", "-c", "set tw=72"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn non_zero_editor_exit_cancels_the_edit() {
        let editor = ExternalEditor::new(Some("false"), "draft").unwrap();
        assert!(editor.edit().is_err());
    }

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
