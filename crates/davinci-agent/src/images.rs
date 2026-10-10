//! Image blocking and tool-result normalization matching
//! `vendor/pi/packages/coding-agent/src/utils/tool-result-images.ts`.

use base64::Engine;
use davinci_ai::{ChatMessage, MessageContent};
use image::{imageops::FilterType, DynamicImage, ImageFormat};

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

/// Pixel budget checked against the header before decoding, so a
/// decompression bomb is rejected without allocating it.
const MAX_DECODE_PIXELS: u64 = 100_000_000;
/// JPEG qualities tried best first; the first one that fits the byte limit wins.
const JPEG_QUALITIES: [u8; 5] = [85, 80, 70, 55, 40];

fn fits_inline(bytes: &[u8]) -> bool {
    encoded_base64_len(bytes) < DEFAULT_MAX_BYTES
}

/// True when any pixel is not fully opaque.
fn has_transparency(image: &DynamicImage) -> bool {
    image.color().has_alpha() && image.to_rgba8().pixels().any(|pixel| pixel[3] < u8::MAX)
}

fn encode_image(image: &DynamicImage, format: ImageFormat) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut out), format)
        .ok()?;
    Some(out)
}

/// Highest-quality encoding that fits the inline limit: lossless PNG first,
/// then JPEG from the best quality down. Images with real transparency stay
/// PNG, because JPEG would flatten the alpha channel; the caller shrinks them
/// instead.
fn encode_within_limit(
    resized: &DynamicImage,
    preserve_alpha: bool,
) -> Option<(&'static str, Vec<u8>)> {
    let rgb = DynamicImage::ImageRgb8(resized.to_rgb8());
    let png_source = if preserve_alpha { resized } else { &rgb };
    if let Some(png) = encode_image(png_source, ImageFormat::Png).filter(|png| fits_inline(png)) {
        return Some(("image/png", png));
    }
    if preserve_alpha {
        return None;
    }
    for quality in JPEG_QUALITIES {
        let mut jpeg = Vec::new();
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality);
        if rgb.write_with_encoder(encoder).is_ok() && fits_inline(&jpeg) {
            return Some(("image/jpeg", jpeg));
        }
    }
    None
}

fn resize_inline(input: &[u8], mime_type: &str) -> Option<ResizedInline> {
    let (header_w, header_h) = image::ImageReader::new(std::io::Cursor::new(input))
        .with_guessed_format()
        .ok()?
        .into_dimensions()
        .ok()?;
    if header_w as u64 * header_h as u64 > MAX_DECODE_PIXELS {
        return None;
    }
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
    let preserve_alpha = has_transparency(&image);
    loop {
        let resized = image.resize_exact(target_w, target_h, FilterType::Lanczos3);
        if let Some((mime, bytes)) = encode_within_limit(&resized, preserve_alpha) {
            return Some(ResizedInline {
                bytes,
                mime_type: mime.into(),
                width: target_w,
                height: target_h,
                original_width: original_w,
                original_height: original_h,
                was_resized: true,
            });
        }
        if target_w == 1 && target_h == 1 {
            return None;
        }
        target_w = (target_w * 3 / 4).max(1);
        target_h = (target_h * 3 / 4).max(1);
    }
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
            ..ChatMessage::default()
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

    fn noise(width: u32, height: u32, alpha: u8) -> DynamicImage {
        let mut state = 0x2545_f491u32;
        let img = image::ImageBuffer::from_fn(width, height, |_, _| {
            let mut next = || {
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                (state >> 8) as u8
            };
            image::Rgba([next(), next(), next(), alpha])
        });
        DynamicImage::ImageRgba8(img)
    }

    fn png_bytes(image: &DynamicImage) -> Vec<u8> {
        let mut out = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut out), ImageFormat::Png)
            .expect("png");
        out
    }

    #[test]
    fn opaque_image_gets_the_best_jpeg_quality_that_fits() {
        let image = noise(1100, 1100, 255);
        assert!(!fits_inline(&png_bytes(&DynamicImage::ImageRgb8(
            image.to_rgb8()
        ))));
        let (mime, bytes) = encode_within_limit(&image, false).expect("fits as jpeg");
        assert_eq!(mime, "image/jpeg");
        let mut best = Vec::new();
        DynamicImage::ImageRgb8(image.to_rgb8())
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut best, 85,
            ))
            .unwrap();
        assert!(fits_inline(&best), "fixture must fit at q85");
        assert_eq!(bytes, best);
    }

    #[test]
    fn transparent_image_is_never_flattened_to_jpeg() {
        let image = noise(1500, 1500, 128);
        assert!(has_transparency(&image));
        assert!(encode_within_limit(&image, true).is_none());
        let small = noise(40, 40, 128);
        let (mime, bytes) = encode_within_limit(&small, true).expect("small png fits");
        assert_eq!(mime, "image/png");
        let decoded = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert!(decoded.pixels().all(|pixel| pixel[3] == 128));
    }

    #[test]
    fn opaque_rgba_is_not_treated_as_transparent() {
        assert!(!has_transparency(&noise(8, 8, 255)));
        assert!(!has_transparency(&DynamicImage::ImageRgb8(
            noise(8, 8, 255).to_rgb8()
        )));
    }

    #[test]
    fn oversized_resize_keeps_transparency_and_fits() {
        let png = png_bytes(&noise(2400, 2400, 128));
        let resized = resize_inline(&png, "image/png").expect("resized");
        assert!(resized.was_resized);
        assert_eq!(resized.mime_type, "image/png");
        let bytes = &resized.bytes;
        assert!(fits_inline(bytes));
        let decoded = image::load_from_memory(bytes).unwrap().to_rgba8();
        assert!(decoded.pixels().any(|pixel| pixel[3] < 255));
    }

    #[test]
    fn pixel_bomb_is_rejected_from_the_header() {
        let bomb = image::ImageBuffer::<image::Luma<u8>, _>::from_pixel(
            10_001,
            10_000,
            image::Luma([7u8]),
        );
        let mut png = Vec::new();
        DynamicImage::ImageLuma8(bomb)
            .write_to(&mut std::io::Cursor::new(&mut png), ImageFormat::Png)
            .expect("png");
        assert!(resize_inline(&png, "image/png").is_none());
    }
}
