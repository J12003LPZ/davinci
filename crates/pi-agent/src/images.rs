//! Image blocking and tool-result normalization matching
//! `vendor/pi/packages/coding-agent/src/utils/tool-result-images.ts`.

use base64::Engine;
use image::{imageops::FilterType, DynamicImage, ImageFormat};
use pi_ai::{ChatMessage, MessageContent};

pub const IMAGE_READING_DISABLED: &str = "Image reading is disabled.";
const DEFAULT_MAX_WIDTH: u32 = 2000;
const DEFAULT_MAX_HEIGHT: u32 = 2000;
const DEFAULT_MAX_BYTES: usize = (4.5 * 1024.0 * 1024.0) as usize;

#[derive(Debug, Clone)]
pub struct ProcessedImage {
    pub data: String,
    pub mime_type: String,
    pub hints: Vec<String>,
}

pub fn apply_block_images(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| {
            if message.role != "user" && message.role != "toolResult" {
                return message.clone();
            }
            if !message
                .content
                .iter()
                .any(|block| matches!(block, MessageContent::Image { .. }))
            {
                return message.clone();
            }
            let mut content = Vec::new();
            for block in &message.content {
                match block {
                    MessageContent::Image { .. } => {
                        let duplicate = matches!(
                            content.last(),
                            Some(MessageContent::Text { text }) if text == IMAGE_READING_DISABLED
                        );
                        if !duplicate {
                            content.push(MessageContent::Text {
                                text: IMAGE_READING_DISABLED.into(),
                            });
                        }
                    }
                    other => content.push(other.clone()),
                }
            }
            let mut out = message.clone();
            out.content = content;
            out
        })
        .collect()
}

pub fn convert_to_llm_for_provider(
    messages: &[ChatMessage],
    block_images: bool,
) -> Vec<ChatMessage> {
    let converted = crate::convert_to_llm(messages);
    if block_images {
        apply_block_images(&converted)
    } else {
        converted
    }
}

pub fn parse_rpc_images(values: &[serde_json::Value]) -> Vec<MessageContent> {
    values
        .iter()
        .filter_map(|value| {
            let data = value.get("data")?.as_str()?.to_string();
            let mime_type = value
                .get("mimeType")
                .or_else(|| value.get("mime_type"))
                .and_then(|item| item.as_str())
                .unwrap_or("image/png")
                .to_string();
            Some(MessageContent::Image { data, mime_type })
        })
        .collect()
}

pub fn normalize_tool_result_images(
    content: &[MessageContent],
    auto_resize_images: bool,
) -> Vec<MessageContent> {
    if !content
        .iter()
        .any(|block| matches!(block, MessageContent::Image { .. }))
    {
        return content.to_vec();
    }
    let mut normalized = Vec::new();
    let mut changed = false;
    for block in content {
        let MessageContent::Image { data, mime_type } = block else {
            normalized.push(block.clone());
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) else {
            normalized.push(block.clone());
            continue;
        };
        match process_image_bytes(&bytes, mime_type, auto_resize_images) {
            Ok(processed) => {
                if processed.data == *data
                    && processed.mime_type == *mime_type
                    && processed.hints.is_empty()
                {
                    normalized.push(block.clone());
                    continue;
                }
                normalized.push(MessageContent::Image {
                    data: processed.data,
                    mime_type: processed.mime_type,
                });
                if !processed.hints.is_empty() {
                    normalized.push(MessageContent::Text {
                        text: processed.hints.join("\n"),
                    });
                }
                changed = true;
            }
            Err(_) => normalized.push(block.clone()),
        }
    }
    if changed {
        normalized
    } else {
        content.to_vec()
    }
}

pub fn process_image_bytes(
    bytes: &[u8],
    mime_type: &str,
    auto_resize_images: bool,
) -> Result<ProcessedImage, String> {
    let base = mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase();
    let (bytes, mime, converted_from) = match base.as_str() {
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp" => {
            let mime = if base == "image/jpg" {
                "image/jpeg"
            } else {
                base.as_str()
            };
            (bytes.to_vec(), mime.to_string(), None)
        }
        other => {
            let image = image::load_from_memory(bytes).map_err(|_| {
                "[Image omitted: could not be converted to a supported inline image format.]"
                    .to_string()
            })?;
            let mut out = Vec::new();
            image
                .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
                .map_err(|_| {
                    "[Image omitted: could not be converted to a supported inline image format.]"
                        .to_string()
                })?;
            (out, "image/png".into(), Some(other.to_string()))
        }
    };
    let mut hints = Vec::new();
    if let Some(from) = converted_from {
        if from != mime {
            hints.push(format!("[Image converted from {from} to {mime}.]"));
        }
    }
    if !auto_resize_images {
        return Ok(ProcessedImage {
            data: base64::engine::general_purpose::STANDARD.encode(&bytes),
            mime_type: mime,
            hints,
        });
    }
    let Some(resized) = resize_inline(&bytes, &mime) else {
        return Err(
            "[Image omitted: could not be resized below the inline image size limit.]".into(),
        );
    };
    if resized.was_resized {
        let scale = resized.original_width as f64 / resized.width.max(1) as f64;
        hints.push(format!(
            "[Image: original {}x{}, displayed at {}x{}. Multiply coordinates by {:.2} to map to original image.]",
            resized.original_width, resized.original_height, resized.width, resized.height, scale
        ));
    }
    Ok(ProcessedImage {
        data: base64::engine::general_purpose::STANDARD.encode(resized.bytes),
        mime_type: resized.mime_type,
        hints,
    })
}

struct ResizedInline {
    bytes: Vec<u8>,
    mime_type: String,
    width: u32,
    height: u32,
    original_width: u32,
    original_height: u32,
    was_resized: bool,
}

fn encoded_base64_len(bytes: &[u8]) -> usize {
    bytes.len().div_ceil(3) * 4
}

fn resize_inline(input: &[u8], mime_type: &str) -> Option<ResizedInline> {
    let image = image::load_from_memory(input).ok()?;
    let original_w = image.width();
    let original_h = image.height();
    if original_w <= DEFAULT_MAX_WIDTH
        && original_h <= DEFAULT_MAX_HEIGHT
        && encoded_base64_len(input) < DEFAULT_MAX_BYTES
    {
        return Some(ResizedInline {
            bytes: input.to_vec(),
            mime_type: mime_type.to_string(),
            width: original_w,
            height: original_h,
            original_width: original_w,
            original_height: original_h,
            was_resized: false,
        });
    }
    let mut target_w = original_w;
    let mut target_h = original_h;
    if target_w > DEFAULT_MAX_WIDTH {
        target_h = ((target_h as u64 * DEFAULT_MAX_WIDTH as u64) / target_w as u64).max(1) as u32;
        target_w = DEFAULT_MAX_WIDTH;
    }
    if target_h > DEFAULT_MAX_HEIGHT {
        target_w = ((target_w as u64 * DEFAULT_MAX_HEIGHT as u64) / target_h as u64).max(1) as u32;
        target_h = DEFAULT_MAX_HEIGHT;
    }
    let resized = image.resize_exact(target_w, target_h, FilterType::Lanczos3);
    let mut out = Vec::new();
    DynamicImage::ImageRgba8(resized.to_rgba8())
        .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
        .ok()?;
    if encoded_base64_len(&out) >= DEFAULT_MAX_BYTES {
        out.clear();
        DynamicImage::ImageRgb8(resized.to_rgb8())
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Jpeg)
            .ok()?;
        return Some(ResizedInline {
            bytes: out,
            mime_type: "image/jpeg".into(),
            width: target_w,
            height: target_h,
            original_width: original_w,
            original_height: original_h,
            was_resized: true,
        });
    }
    Some(ResizedInline {
        bytes: out,
        mime_type: "image/png".into(),
        width: target_w,
        height: target_h,
        original_width: original_w,
        original_height: original_h,
        was_resized: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_images_replaces_and_dedupes_placeholders() {
        let messages = vec![ChatMessage {
            role: "user".into(),
            content: vec![
                MessageContent::Image {
                    data: "abc".into(),
                    mime_type: "image/png".into(),
                },
                MessageContent::Image {
                    data: "def".into(),
                    mime_type: "image/png".into(),
                },
                MessageContent::Text {
                    text: "caption".into(),
                },
            ],
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        }];
        let blocked = apply_block_images(&messages);
        assert_eq!(
            blocked[0].content,
            vec![
                MessageContent::Text {
                    text: IMAGE_READING_DISABLED.into()
                },
                MessageContent::Text {
                    text: "caption".into()
                },
            ]
        );
    }

    #[test]
    fn normalize_keeps_non_images_and_passthrough_failures() {
        let content = vec![MessageContent::Text { text: "ok".into() }];
        assert_eq!(normalize_tool_result_images(&content, true), content);
    }
}
