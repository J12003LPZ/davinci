//! `@file` CLI arguments matching TS `cli/file-processor.ts` + `cli/initial-message.ts`.

use std::path::{Path, PathBuf};

use base64::Engine;
use davinci_ai::MessageContent;

use crate::image_convert::resize_image_in_process;

const IMAGE_TYPE_SNIFF_BYTES: usize = 4100;
const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
const NARROW_NO_BREAK_SPACE: char = '\u{202F}';

pub const RPC_FILE_ARGS_ERROR: &str = "Error: @file arguments are not supported in RPC mode";
pub const IMAGE_OMITTED_CONVERT: &str =
    "[Image omitted: could not be converted to a supported inline image format.]";
pub const IMAGE_OMITTED_RESIZE: &str =
    "[Image omitted: could not be resized below the inline image size limit.]";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcessedFiles {
    pub text: String,
    pub images: Vec<MessageContent>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitialMessage {
    pub text: Option<String>,
    pub images: Vec<MessageContent>,
    pub remaining_messages: Vec<String>,
}

pub fn expand_home(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = davinci_session::home_dir() {
            return home.join(rest).display().to_string();
        }
    }
    if path == "~" {
        if let Some(home) = davinci_session::home_dir() {
            return home.display().to_string();
        }
    }
    path.to_string()
}

pub fn resolve_read_path(file_arg: &str, cwd: &Path) -> PathBuf {
    let expanded = expand_home(file_arg.trim_start_matches('@'));
    let path = Path::new(&expanded);
    let resolved = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    if resolved.exists() {
        return resolved;
    }
    let am_pm = macos_screenshot_ampm(&resolved);
    if am_pm != resolved && am_pm.exists() {
        return am_pm;
    }
    let curly = curly_quote_variant(&resolved);
    if curly != resolved && curly.exists() {
        return curly;
    }
    resolved
}

fn macos_screenshot_ampm(path: &Path) -> PathBuf {
    let text = path.display().to_string();
    let replaced = text.replace(" AM.", &format!("{NARROW_NO_BREAK_SPACE}AM."));
    let replaced = replaced.replace(" PM.", &format!("{NARROW_NO_BREAK_SPACE}PM."));
    let replaced = replaced.replace(" am.", &format!("{NARROW_NO_BREAK_SPACE}am."));
    let replaced = replaced.replace(" pm.", &format!("{NARROW_NO_BREAK_SPACE}pm."));
    PathBuf::from(replaced)
}

fn curly_quote_variant(path: &Path) -> PathBuf {
    PathBuf::from(path.display().to_string().replace('\'', "\u{2019}"))
}

pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

pub fn detect_supported_image_mime_type(buffer: &[u8]) -> Option<&'static str> {
    if starts_with(buffer, &[0xff, 0xd8, 0xff]) {
        return if buffer.get(3) == Some(&0xf7) {
            None
        } else {
            Some("image/jpeg")
        };
    }
    if starts_with(buffer, &PNG_SIGNATURE) {
        return if is_png(buffer) && !is_animated_png(buffer) {
            Some("image/png")
        } else {
            None
        };
    }
    if starts_with_ascii(buffer, 0, "GIF") {
        return Some("image/gif");
    }
    if starts_with_ascii(buffer, 0, "RIFF") && starts_with_ascii(buffer, 8, "WEBP") {
        return Some("image/webp");
    }
    if starts_with_ascii(buffer, 0, "BM") && is_bmp(buffer) {
        return Some("image/bmp");
    }
    None
}

fn starts_with(buffer: &[u8], bytes: &[u8]) -> bool {
    buffer.len() >= bytes.len() && buffer[..bytes.len()] == *bytes
}

fn starts_with_ascii(buffer: &[u8], offset: usize, text: &str) -> bool {
    let end = offset + text.len();
    buffer.len() >= end && &buffer[offset..end] == text.as_bytes()
}

fn read_u32_be(buffer: &[u8], offset: usize) -> u32 {
    if offset + 4 > buffer.len() {
        return 0;
    }
    u32::from_be_bytes([
        buffer[offset],
        buffer[offset + 1],
        buffer[offset + 2],
        buffer[offset + 3],
    ])
}

fn read_u32_le(buffer: &[u8], offset: usize) -> u32 {
    if offset + 4 > buffer.len() {
        return 0;
    }
    u32::from_le_bytes([
        buffer[offset],
        buffer[offset + 1],
        buffer[offset + 2],
        buffer[offset + 3],
    ])
}

fn read_u16_le(buffer: &[u8], offset: usize) -> u16 {
    if offset + 2 > buffer.len() {
        return 0;
    }
    u16::from_le_bytes([buffer[offset], buffer[offset + 1]])
}

fn is_png(buffer: &[u8]) -> bool {
    buffer.len() >= 16
        && read_u32_be(buffer, PNG_SIGNATURE.len()) == 13
        && starts_with_ascii(buffer, 12, "IHDR")
}

fn is_animated_png(buffer: &[u8]) -> bool {
    let mut offset = PNG_SIGNATURE.len();
    while offset + 8 <= buffer.len() {
        let chunk_length = read_u32_be(buffer, offset) as usize;
        let chunk_type = offset + 4;
        if starts_with_ascii(buffer, chunk_type, "acTL") {
            return true;
        }
        if starts_with_ascii(buffer, chunk_type, "IDAT") {
            return false;
        }
        let next = offset
            .saturating_add(8)
            .saturating_add(chunk_length)
            .saturating_add(4);
        if next <= offset || next > buffer.len() {
            return false;
        }
        offset = next;
    }
    false
}

fn is_bmp(buffer: &[u8]) -> bool {
    if buffer.len() < 26 {
        return false;
    }
    let declared_file_size = read_u32_le(buffer, 2);
    let pixel_data_offset = read_u32_le(buffer, 10);
    let dib_header_size = read_u32_le(buffer, 14);
    if declared_file_size != 0 && declared_file_size < 26 {
        return false;
    }
    if pixel_data_offset < 14 + dib_header_size {
        return false;
    }
    if declared_file_size != 0 && pixel_data_offset >= declared_file_size {
        return false;
    }
    let (color_planes, bits_per_pixel) = if dib_header_size == 12 {
        (read_u16_le(buffer, 22), read_u16_le(buffer, 24))
    } else if (40..=124).contains(&dib_header_size) {
        if buffer.len() < 30 {
            return false;
        }
        (read_u16_le(buffer, 26), read_u16_le(buffer, 28))
    } else {
        return false;
    };
    color_planes == 1 && matches!(bits_per_pixel, 1 | 4 | 8 | 16 | 24 | 32)
}

fn format_dimension_note(
    original_width: u32,
    original_height: u32,
    width: u32,
    height: u32,
    was_resized: bool,
) -> Option<String> {
    if !was_resized || width == 0 {
        return None;
    }
    let scale = original_width as f64 / width as f64;
    Some(format!(
        "[Image: original {original_width}x{original_height}, displayed at {width}x{height}. Multiply coordinates by {scale:.2} to map to original image.]"
    ))
}

fn conversion_hint(from: &str, to: &str) -> Option<String> {
    if from == to {
        None
    } else {
        Some(format!("[Image converted from {from} to {to}.]"))
    }
}

fn process_image(
    bytes: &[u8],
    mime_type: &str,
    auto_resize: bool,
) -> Result<(String, String, Vec<String>), String> {
    let (normalized, converted_from) = if matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
    ) {
        let mime = if mime_type == "image/jpg" {
            "image/jpeg"
        } else {
            mime_type
        };
        (bytes.to_vec(), mime.to_string())
    } else if let Some(png) = crate::image_convert::convert_image_bytes_to_png(bytes) {
        (png, "image/png".into())
    } else {
        return Err(IMAGE_OMITTED_CONVERT.into());
    };
    let source_mime = if mime_type == "image/jpg" {
        "image/jpeg"
    } else {
        mime_type
    };
    if !auto_resize {
        let mut hints = Vec::new();
        if let Some(hint) = conversion_hint(source_mime, &converted_from) {
            hints.push(hint);
        }
        return Ok((
            base64::engine::general_purpose::STANDARD.encode(&normalized),
            converted_from,
            hints,
        ));
    }
    let resized = resize_image_in_process(&normalized, &converted_from)
        .ok_or_else(|| IMAGE_OMITTED_RESIZE.to_string())?;
    let mut hints = Vec::new();
    if let Some(hint) = conversion_hint(source_mime, &resized.mime_type) {
        hints.push(hint);
    }
    if let Some(note) = format_dimension_note(
        resized.original_width,
        resized.original_height,
        resized.width,
        resized.height,
        resized.was_resized,
    ) {
        hints.push(note);
    }
    Ok((
        base64::engine::general_purpose::STANDARD.encode(&resized.bytes),
        resized.mime_type,
        hints,
    ))
}

/// Process `@file` arguments into text wrappers and image attachments.
pub fn process_file_arguments(
    file_args: &[String],
    cwd: &Path,
    auto_resize_images: bool,
) -> Result<ProcessedFiles, String> {
    let mut text = String::new();
    let mut images = Vec::new();
    for file_arg in file_args {
        let absolute = resolve_read_path(file_arg, cwd);
        if !absolute.exists() {
            return Err(format!("Error: File not found: {}", absolute.display()));
        }
        let metadata = std::fs::metadata(&absolute)
            .map_err(|err| format!("Error: Could not read file {}: {err}", absolute.display()))?;
        if metadata.len() == 0 {
            continue;
        }
        let bytes = std::fs::read(&absolute)
            .map_err(|err| format!("Error: Could not read file {}: {err}", absolute.display()))?;
        let sniff_len = bytes.len().min(IMAGE_TYPE_SNIFF_BYTES);
        if let Some(mime) = detect_supported_image_mime_type(&bytes[..sniff_len]) {
            match process_image(&bytes, mime, auto_resize_images) {
                Ok((data, mime_type, hints)) => {
                    images.push(MessageContent::Image { data, mime_type });
                    if hints.is_empty() {
                        text.push_str(&format!("<file name=\"{}\"></file>\n", absolute.display()));
                    } else {
                        text.push_str(&format!(
                            "<file name=\"{}\">{}</file>\n",
                            absolute.display(),
                            hints.join("\n")
                        ));
                    }
                }
                Err(message) => {
                    text.push_str(&format!(
                        "<file name=\"{}\">{message}</file>\n",
                        absolute.display()
                    ));
                }
            }
        } else {
            let content = strip_bom(std::str::from_utf8(&bytes).map_err(|err| {
                format!("Error: Could not read file {}: {err}", absolute.display())
            })?);
            text.push_str(&format!(
                "<file name=\"{}\">\n{content}\n</file>\n",
                absolute.display()
            ));
        }
    }
    Ok(ProcessedFiles { text, images })
}

/// Combine stdin, `@file` text, and the first CLI message (TS `buildInitialMessage`).
pub fn build_initial_message(
    messages: &[String],
    file_text: Option<&str>,
    file_images: &[MessageContent],
    stdin_content: Option<&str>,
) -> InitialMessage {
    let mut parts = Vec::new();
    if let Some(stdin) = stdin_content {
        parts.push(stdin.to_string());
    }
    if let Some(file_text) = file_text.filter(|text| !text.is_empty()) {
        parts.push(file_text.to_string());
    }
    let mut remaining = messages.to_vec();
    if !remaining.is_empty() {
        parts.push(remaining.remove(0));
    }
    InitialMessage {
        text: if parts.is_empty() {
            None
        } else {
            Some(parts.join(""))
        },
        images: file_images.to_vec(),
        remaining_messages: remaining,
    }
}

pub fn prepare_initial_message(
    messages: &[String],
    file_args: &[String],
    stdin_content: Option<&str>,
    cwd: &Path,
    auto_resize_images: bool,
) -> Result<InitialMessage, String> {
    if file_args.is_empty() {
        return Ok(build_initial_message(messages, None, &[], stdin_content));
    }
    let processed = process_file_arguments(file_args, cwd, auto_resize_images)?;
    Ok(build_initial_message(
        messages,
        Some(&processed.text),
        &processed.images,
        stdin_content,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn tiny_png() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(1, 1, image::Rgb([255, 0, 0]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
            .expect("png");
        out
    }

    #[test]
    fn wraps_text_files_and_skips_empty() {
        let dir = tempfile::tempdir().unwrap();
        let text_path = dir.path().join("notes.txt");
        fs::write(&text_path, "\u{feff}hello\nworld").unwrap();
        fs::write(dir.path().join("empty.txt"), "").unwrap();
        let processed = process_file_arguments(
            &[
                text_path.display().to_string(),
                dir.path().join("empty.txt").display().to_string(),
            ],
            dir.path(),
            true,
        )
        .unwrap();
        assert!(processed.text.contains(&format!(
            "<file name=\"{}\">\nhello\nworld\n</file>",
            text_path.display()
        )));
        assert!(!processed.text.contains("empty.txt"));
        assert!(processed.images.is_empty());
    }

    #[test]
    fn missing_file_uses_ts_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("gone.txt");
        let err =
            process_file_arguments(&[missing.display().to_string()], dir.path(), true).unwrap_err();
        assert_eq!(err, format!("Error: File not found: {}", missing.display()));
    }

    #[test]
    fn images_become_attachments_with_file_wrapper() {
        let dir = tempfile::tempdir().unwrap();
        let png_path = dir.path().join("dot.png");
        let mut file = fs::File::create(&png_path).unwrap();
        file.write_all(&tiny_png()).unwrap();
        let processed =
            process_file_arguments(&[png_path.display().to_string()], dir.path(), true).unwrap();
        assert_eq!(processed.images.len(), 1);
        assert!(matches!(
            &processed.images[0],
            MessageContent::Image { mime_type, .. } if mime_type == "image/png"
        ));
        assert!(processed
            .text
            .contains(&format!("<file name=\"{}\"></file>", png_path.display())));
    }

    #[test]
    fn build_initial_message_joins_without_separators_and_shifts_first() {
        let built = build_initial_message(
            &["one".into(), "two".into()],
            Some("<file name=\"a\">x</file>\n"),
            &[],
            Some("stdin"),
        );
        assert_eq!(
            built.text.as_deref(),
            Some("stdin<file name=\"a\">x</file>\none")
        );
        assert_eq!(built.remaining_messages, ["two"]);
    }

    #[test]
    fn detects_jpeg_and_rejects_jpeg_f7() {
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff, 0xe0]),
            Some("image/jpeg")
        );
        assert_eq!(
            detect_supported_image_mime_type(&[0xff, 0xd8, 0xff, 0xf7]),
            None
        );
    }
}
