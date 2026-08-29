//! TypeScript `packages/tui/src/components/image.ts`.

use std::cell::RefCell;

use crate::component::Component;
use crate::diff::truncate_to_width;
use crate::terminal_image::{
    allocate_image_id, get_capabilities, get_cell_dimensions, get_image_dimensions, image_fallback,
    parse_kitty_image_header, render_image, ImageDimensions, ImageProtocol, ImageRenderOptions,
};

#[derive(Debug, Clone)]
pub struct Image {
    base64_data: String,
    mime_type: String,
    dimensions: ImageDimensions,
    fallback_color: fn(&str) -> String,
    max_width_cells: Option<u32>,
    max_height_cells: Option<u32>,
    filename: Option<String>,
    image_id: RefCell<Option<u32>>,
    cached_lines: RefCell<Option<(usize, Vec<String>)>>,
}

pub struct ImageOptions {
    pub max_width_cells: Option<u32>,
    pub max_height_cells: Option<u32>,
    pub filename: Option<String>,
    pub image_id: Option<u32>,
}

impl Default for ImageOptions {
    fn default() -> Self {
        Self {
            max_width_cells: None,
            max_height_cells: None,
            filename: None,
            image_id: None,
        }
    }
}

impl Image {
    pub fn new(
        base64_data: impl Into<String>,
        mime_type: impl Into<String>,
        fallback_color: fn(&str) -> String,
        options: ImageOptions,
        dimensions: Option<ImageDimensions>,
    ) -> Self {
        let ImageOptions {
            max_width_cells,
            max_height_cells,
            filename,
            image_id,
        } = options;
        let base64_data = base64_data.into();
        let mime_type = mime_type.into();
        let dimensions = dimensions
            .or_else(|| get_image_dimensions(&base64_data, &mime_type))
            .unwrap_or(ImageDimensions {
                width_px: 800,
                height_px: 600,
            });
        Self {
            base64_data,
            mime_type,
            dimensions,
            fallback_color,
            max_width_cells,
            max_height_cells,
            filename,
            image_id: RefCell::new(image_id),
            cached_lines: RefCell::new(None),
        }
    }

    pub fn image_id(&self) -> Option<u32> {
        *self.image_id.borrow()
    }

    pub fn invalidate(&self) {
        self.cached_lines.replace(None);
    }
}

impl Component for Image {
    fn render(&self, width: usize) -> Vec<String> {
        if let Some((cached_width, lines)) = self.cached_lines.borrow().as_ref() {
            if *cached_width == width {
                return lines.clone();
            }
        }
        let max_width =
            1.max((width.saturating_sub(2) as u32).min(self.max_width_cells.unwrap_or(60)));
        let cell = get_cell_dimensions();
        let default_max_height = 1.max((max_width * cell.width_px).div_ceil(cell.height_px));
        let max_height = self.max_height_cells.unwrap_or(default_max_height);
        let caps = get_capabilities();
        let mut image_id = *self.image_id.borrow();
        let lines = if let Some(protocol) = caps.images {
            if protocol == ImageProtocol::Kitty && image_id.is_none() {
                image_id = Some(allocate_image_id());
            }
            match render_image(
                &self.base64_data,
                self.dimensions,
                ImageRenderOptions {
                    max_width_cells: Some(max_width),
                    max_height_cells: Some(max_height),
                    image_id,
                    move_cursor: Some(false),
                    ..ImageRenderOptions::default()
                },
            ) {
                Some(result) => {
                    if result.image_id.is_some() {
                        image_id = result.image_id;
                    }
                    if protocol == ImageProtocol::Kitty {
                        let mut lines = vec![result.sequence];
                        for _ in 0..result.rows.saturating_sub(1) {
                            lines.push(String::new());
                        }
                        lines
                    } else {
                        let mut lines = Vec::new();
                        for _ in 0..result.rows.saturating_sub(1) {
                            lines.push(String::new());
                        }
                        let row_offset = result.rows.saturating_sub(1);
                        let move_up = if row_offset > 0 {
                            format!("\x1b[{row_offset}A")
                        } else {
                            String::new()
                        };
                        lines.push(format!("{}{}", move_up, result.sequence));
                        lines
                    }
                }
                None => self.fallback_lines(width),
            }
        } else {
            self.fallback_lines(width)
        };
        if image_id.is_none() {
            if let Some(line) = lines.first() {
                if let Some((ids, _)) = parse_kitty_image_header(line) {
                    image_id = ids.first().copied();
                }
            }
        }
        *self.image_id.borrow_mut() = image_id;
        self.cached_lines.replace(Some((width, lines.clone())));
        lines
    }
}

impl Image {
    fn fallback_lines(&self, width: usize) -> Vec<String> {
        let fallback = image_fallback(
            &self.mime_type,
            Some(self.dimensions),
            self.filename.as_deref(),
        );
        vec![truncate_to_width(
            &(self.fallback_color)(&fallback),
            width,
            "...",
        )]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::visible_width;
    use crate::terminal_image::{
        reset_capabilities_cache, set_capabilities, set_cell_dimensions, CellDimensions,
        TerminalCapabilities,
    };

    fn identity(value: &str) -> String {
        value.to_string()
    }

    fn yellow(value: &str) -> String {
        format!("\x1b[33m{value}\x1b[0m")
    }

    #[test]
    fn image_square_box_and_padding_match_typescript() {
        let _lock = crate::terminal_image::capabilities_lock();
        set_capabilities(TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        });
        set_cell_dimensions(CellDimensions {
            width_px: 10,
            height_px: 20,
        });
        let image = Image::new(
            "AAAA",
            "image/png",
            identity,
            ImageOptions {
                max_width_cells: Some(10),
                ..ImageOptions::default()
            },
            Some(ImageDimensions {
                width_px: 10,
                height_px: 100,
            }),
        );
        let lines = image.render(12);
        assert_eq!(lines.len(), 5);
        assert!(lines[0].contains(",c=1,r=5"));
        set_cell_dimensions(CellDimensions {
            width_px: 10,
            height_px: 10,
        });
        let padded = Image::new(
            "AAAA",
            "image/png",
            identity,
            ImageOptions {
                max_width_cells: Some(2),
                ..ImageOptions::default()
            },
            Some(ImageDimensions {
                width_px: 20,
                height_px: 20,
            }),
        );
        let lines = padded.render(4);
        let image_id = padded.image_id().expect("id");
        assert!(lines[0].starts_with("\x1b_G"));
        assert!(lines[0].contains(",C=1,"));
        assert!(lines[0].contains(&format!(",i={image_id}")));
        assert!(lines[0].ends_with("\x1b\\"));
        assert_eq!(lines[1..], [""]);
        reset_capabilities_cache();
        set_cell_dimensions(CellDimensions {
            width_px: 9,
            height_px: 18,
        });
    }

    #[test]
    fn image_fallback_truncates_and_shortens_home() {
        let _lock = crate::terminal_image::capabilities_lock();
        set_capabilities(TerminalCapabilities {
            images: None,
            true_color: false,
            hyperlinks: false,
        });
        let home = std::env::var("HOME").unwrap_or_else(|_| "/home/user".into());
        let long_path = format!(
            "{home}/images/{}.png",
            "generated-image-with-a-very-long-absolute-path".repeat(4)
        );
        let image = Image::new(
            "AAAA",
            "image/png",
            yellow,
            ImageOptions {
                filename: Some(long_path),
                ..ImageOptions::default()
            },
            Some(ImageDimensions {
                width_px: 1280,
                height_px: 720,
            }),
        );
        let lines = image.render(40);
        assert_eq!(lines.len(), 1);
        assert!(visible_width(&lines[0]) <= 40);
        assert!(lines[0].contains("..."));
        assert!(lines[0].contains('~'));
        reset_capabilities_cache();
    }
}
