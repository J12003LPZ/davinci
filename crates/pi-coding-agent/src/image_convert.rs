//! Image conversion matching TS `image-convert.ts` + `exif-orientation.ts`.

use image::{imageops::FilterType, DynamicImage, ImageEncoder, ImageFormat, RgbaImage};
use std::io::Cursor;

pub struct ResizedImage {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub was_resized: bool,
}

const DEFAULT_MAX_WIDTH: u32 = 2000;
const DEFAULT_MAX_HEIGHT: u32 = 2000;
const DEFAULT_MAX_BYTES: usize = (4.5 * 1024.0 * 1024.0) as usize;

pub fn convert_image_bytes_to_png(bytes: &[u8]) -> Option<Vec<u8>> {
    let image = image::load_from_memory(bytes).ok()?;
    let rgba = image.to_rgba8();
    if rgba.width() == 0 || rgba.height() == 0 || rgba.width() > 4096 || rgba.height() > 4096 {
        return None;
    }
    let oriented = apply_exif_orientation(rgba, get_exif_orientation(bytes));
    encode_dynamic(DynamicImage::ImageRgba8(oriented), ImageFormat::Png)
}

fn encode_dynamic(image: DynamicImage, format: ImageFormat) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    image.write_to(&mut Cursor::new(&mut out), format).ok()?;
    Some(out)
}

fn encoded_base64_len(bytes: &[u8]) -> usize {
    bytes.len().div_ceil(3) * 4
}

/// TS `resizeImageInProcess`: Lanczos3 fit within 2000×2000 and 4.5MB base64.
pub fn resize_image_in_process(input: &[u8], mime_type: &str) -> Option<ResizedImage> {
    let image = image::load_from_memory(input).ok()?;
    let original_width = image.width();
    let original_height = image.height();
    let input_b64 = encoded_base64_len(input);
    if original_width <= DEFAULT_MAX_WIDTH
        && original_height <= DEFAULT_MAX_HEIGHT
        && input_b64 < DEFAULT_MAX_BYTES
    {
        return Some(ResizedImage {
            bytes: input.to_vec(),
            mime_type: mime_type.to_string(),
            original_width,
            original_height,
            width: original_width,
            height: original_height,
            was_resized: false,
        });
    }
    let mut target_width = original_width;
    let mut target_height = original_height;
    if target_width > DEFAULT_MAX_WIDTH {
        target_height =
            ((target_height as u64 * DEFAULT_MAX_WIDTH as u64) / target_width as u64).max(1) as u32;
        target_width = DEFAULT_MAX_WIDTH;
    }
    if target_height > DEFAULT_MAX_HEIGHT {
        target_width = ((target_width as u64 * DEFAULT_MAX_HEIGHT as u64) / target_height as u64)
            .max(1) as u32;
        target_height = DEFAULT_MAX_HEIGHT;
    }
    let qualities = [80u8, 85, 70, 55, 40];
    loop {
        let resized = image.resize_exact(target_width, target_height, FilterType::Lanczos3);
        let mut candidates = Vec::new();
        if let Some(png) = encode_dynamic(resized.clone(), ImageFormat::Png) {
            candidates.push(("image/png", png));
        }
        let rgb = DynamicImage::ImageRgb8(resized.to_rgb8());
        for quality in qualities {
            let mut jpeg = Vec::new();
            let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, quality);
            if encoder
                .write_image(
                    rgb.as_rgb8()?.as_raw(),
                    target_width,
                    target_height,
                    image::ExtendedColorType::Rgb8,
                )
                .is_ok()
            {
                candidates.push(("image/jpeg", jpeg));
            }
        }
        if let Some((mime, bytes)) = candidates
            .into_iter()
            .filter(|(_, bytes)| encoded_base64_len(bytes) < DEFAULT_MAX_BYTES)
            .min_by_key(|(_, bytes)| bytes.len())
        {
            return Some(ResizedImage {
                bytes,
                mime_type: mime.into(),
                original_width,
                original_height,
                width: target_width,
                height: target_height,
                was_resized: true,
            });
        }
        if target_width == 1 && target_height == 1 {
            return None;
        }
        let next_w = if target_width == 1 {
            1
        } else {
            (target_width * 3 / 4).max(1)
        };
        let next_h = if target_height == 1 {
            1
        } else {
            (target_height * 3 / 4).max(1)
        };
        if next_w == target_width && next_h == target_height {
            return None;
        }
        target_width = next_w;
        target_height = next_h;
    }
}

pub fn get_exif_orientation(bytes: &[u8]) -> u8 {
    let tiff = if bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] == 0xd8 {
        find_jpeg_tiff_offset(bytes)
    } else if is_webp(bytes) {
        find_webp_tiff_offset(bytes)
    } else {
        None
    };
    tiff.map(|offset| read_orientation_from_tiff(bytes, offset))
        .unwrap_or(1)
}

fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && bytes.starts_with(b"RIFF") && bytes[8..12] == *b"WEBP"
}

fn has_exif_header(bytes: &[u8], offset: usize) -> bool {
    bytes.get(offset..offset + 6) == Some(b"Exif\0\0")
}

fn find_jpeg_tiff_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 2usize;
    while offset + 1 < bytes.len() {
        if bytes[offset] != 0xff {
            return None;
        }
        let marker = bytes[offset + 1];
        if marker == 0xff {
            offset += 1;
            continue;
        }
        if marker == 0xe1 {
            let segment_start = offset.checked_add(4)?;
            if !has_exif_header(bytes, segment_start) {
                return None;
            }
            return Some(segment_start + 6);
        }
        if offset + 4 > bytes.len() {
            return None;
        }
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        offset += 2 + length;
    }
    None
}

fn find_webp_tiff_offset(bytes: &[u8]) -> Option<usize> {
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let chunk_id = bytes.get(offset..offset + 4)?;
        let chunk_size =
            u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
        let data_start = offset + 8;
        if chunk_id == b"EXIF" {
            if data_start + chunk_size > bytes.len() {
                return None;
            }
            let tiff_start = if chunk_size >= 6 && has_exif_header(bytes, data_start) {
                data_start + 6
            } else {
                data_start
            };
            return Some(tiff_start);
        }
        offset = data_start + chunk_size + (chunk_size % 2);
    }
    None
}

fn read_orientation_from_tiff(bytes: &[u8], tiff_start: usize) -> u8 {
    if tiff_start + 8 > bytes.len() {
        return 1;
    }
    let le = u16::from_be_bytes([bytes[tiff_start], bytes[tiff_start + 1]]) == 0x4949
        || (bytes[tiff_start] == b'I' && bytes[tiff_start + 1] == b'I');
    let read16 = |pos: usize| -> Option<u16> {
        let b = bytes.get(pos..pos + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let read32 = |pos: usize| -> Option<u32> {
        let b = bytes.get(pos..pos + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd_offset = match read32(tiff_start + 4) {
        Some(value) => value as usize,
        None => return 1,
    };
    let ifd_start = tiff_start + ifd_offset;
    let entry_count = match read16(ifd_start) {
        Some(value) => value as usize,
        None => return 1,
    };
    for index in 0..entry_count {
        let entry_pos = ifd_start + 2 + index * 12;
        if read16(entry_pos) == Some(0x0112) {
            let value = read16(entry_pos + 8).unwrap_or(1);
            return if (1..=8).contains(&value) {
                value as u8
            } else {
                1
            };
        }
    }
    1
}

fn apply_exif_orientation(image: RgbaImage, orientation: u8) -> RgbaImage {
    if orientation == 1 {
        return image;
    }
    let w = image.width() as usize;
    let h = image.height() as usize;
    let src = image.into_raw();
    let mut current = (src, w, h);
    match orientation {
        2 => current = flip_h(current.0, current.1, current.2),
        3 => {
            current = flip_h(current.0, current.1, current.2);
            current = flip_v(current.0, current.1, current.2);
        }
        4 => current = flip_v(current.0, current.1, current.2),
        5 => {
            current = rotate90_ts6(current.0, current.1, current.2);
            current = flip_h(current.0, current.1, current.2);
        }
        6 => current = rotate90_ts6(current.0, current.1, current.2),
        7 => {
            current = rotate90_ts8(current.0, current.1, current.2);
            current = flip_h(current.0, current.1, current.2);
        }
        8 => current = rotate90_ts8(current.0, current.1, current.2),
        _ => {}
    }
    RgbaImage::from_raw(current.1 as u32, current.2 as u32, current.0)
        .unwrap_or_else(|| RgbaImage::new(w as u32, h as u32))
}

fn flip_h(src: Vec<u8>, w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let mut dest = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let src_idx = (y * w + x) * 4;
            let dst_idx = (y * w + (w - 1 - x)) * 4;
            dest[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    (dest, w, h)
}

fn flip_v(src: Vec<u8>, w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let mut dest = vec![0u8; src.len()];
    for y in 0..h {
        for x in 0..w {
            let src_idx = (y * w + x) * 4;
            let dst_idx = ((h - 1 - y) * w + x) * 4;
            dest[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    (dest, w, h)
}

/// TS orientation 6: destIndex (x, y, _w, h) => x * h + (h - 1 - y), dest is h×w.
fn rotate90_ts6(src: Vec<u8>, w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let dest_w = h;
    let dest_h = w;
    let mut dest = vec![0u8; dest_w * dest_h * 4];
    for y in 0..h {
        for x in 0..w {
            let src_idx = (y * w + x) * 4;
            let dest_index = x * h + (h - 1 - y);
            let dst_idx = dest_index * 4;
            dest[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    (dest, dest_w, dest_h)
}

/// TS orientation 8: destIndex (x, y, w, h) => (w - 1 - x) * h + y, dest is h×w.
fn rotate90_ts8(src: Vec<u8>, w: usize, h: usize) -> (Vec<u8>, usize, usize) {
    let dest_w = h;
    let dest_h = w;
    let mut dest = vec![0u8; dest_w * dest_h * 4];
    for y in 0..h {
        for x in 0..w {
            let src_idx = (y * w + x) * 4;
            let dest_index = (w - 1 - x) * h + y;
            let dst_idx = dest_index * 4;
            dest[dst_idx..dst_idx + 4].copy_from_slice(&src[src_idx..src_idx + 4]);
        }
    }
    (dest, dest_w, dest_h)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgba};

    fn solid_png(width: u32, height: u32, color: [u8; 4]) -> Vec<u8> {
        let img = ImageBuffer::from_pixel(width, height, Rgba(color));
        let mut out = Vec::new();
        DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .expect("png");
        out
    }

    fn encode(format: ImageFormat, width: u32, height: u32) -> Vec<u8> {
        let img = ImageBuffer::from_pixel(width, height, Rgba([10, 20, 30, 255]));
        let mut out = Vec::new();
        let dynamic = DynamicImage::ImageRgba8(img);
        let dynamic = if matches!(format, ImageFormat::Jpeg) {
            DynamicImage::ImageRgb8(dynamic.to_rgb8())
        } else {
            dynamic
        };
        dynamic
            .write_to(&mut Cursor::new(&mut out), format)
            .expect("encode");
        out
    }

    fn jpeg_with_orientation(orientation: u8) -> Vec<u8> {
        let jpeg = encode(ImageFormat::Jpeg, 2, 1);
        let mut tiff = vec![b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0];
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&(orientation as u16).to_le_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend_from_slice(&tiff);
        let mut out = vec![0xff, 0xd8, 0xff, 0xe1];
        let len = (app1.len() + 2) as u16;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&app1);
        out.extend_from_slice(&jpeg[2..]);
        out
    }

    #[test]
    fn converts_jpeg_gif_webp_and_png() {
        for format in [ImageFormat::Jpeg, ImageFormat::Gif, ImageFormat::WebP] {
            let bytes = encode(format, 2, 2);
            let png = convert_image_bytes_to_png(&bytes).expect("convert");
            assert_eq!(&png[..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        }
        let png = solid_png(1, 1, [1, 2, 3, 255]);
        assert!(convert_image_bytes_to_png(&png).is_some());
    }

    #[test]
    fn reads_jpeg_exif_orientation_like_ts() {
        let bytes = jpeg_with_orientation(6);
        assert_eq!(get_exif_orientation(&bytes), 6);
        assert_eq!(get_exif_orientation(&[0xff, 0xd8, 0xff, 0xd9]), 1);
    }

    #[test]
    fn resize_fits_large_images() {
        let big = encode(ImageFormat::Png, 80, 40);
        let resized = resize_image_in_process(&big, "image/png").expect("resize");
        assert!(!resized.was_resized);
        assert_eq!(resized.width, 80);
        assert_eq!(resized.original_width, 80);
        assert_eq!(resized.original_height, 40);
        assert_eq!(resized.mime_type, "image/png");
        let huge = ImageBuffer::from_pixel(2001, 10, Rgba([1, 2, 3, 255]));
        let mut png = Vec::new();
        DynamicImage::ImageRgba8(huge)
            .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
            .expect("png");
        let resized = resize_image_in_process(&png, "image/png").expect("resize huge");
        assert!(resized.was_resized);
        assert!(resized.width <= 2000);
        assert!(resized.height >= 1);
    }

    #[test]
    fn orientation_6_rotates_dimensions() {
        let src = ImageBuffer::from_pixel(3, 1, Rgba([255, 0, 0, 255]));
        let rotated = apply_exif_orientation(src, 6);
        assert_eq!((rotated.width(), rotated.height()), (1, 3));
    }
}
