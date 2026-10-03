//! Bounded raster validation before an asset enters an immutable revision.
use super::error::*;
use image::{ImageFormat, ImageReader, Limits};
use std::io::Cursor;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImageInput {
    pub path: String,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub provenance: String,
    pub rights: String,
}

impl super::store::DesignStore {
    /// Trusted SDK import. Bytes are supplied by the caller, never fetched from a URL.
    /// Publication still requires a revision CAS with the returned asset reference.
    pub fn store_image(
        &self,
        ctx: &super::admission::AuthorizedDesignContext,
        session: &mut davinci_session::JsonlSession,
        input: &ImageInput,
        bytes: &[u8],
    ) -> DesignResult<super::records::Asset> {
        self.prepare(ctx, session, "design_patch", input)?;
        super::types::validate_path(&input.path)?;
        super::types::validate_text(&input.provenance, 4096, "asset provenance")?;
        super::types::validate_text(&input.rights, 4096, "asset rights")?;
        validate_image(bytes, &input.media_type, input.width, input.height)?;
        let source = self.retain(ctx, &input.media_type, bytes)?;
        Ok(super::records::Asset {
            path: input.path.clone(),
            source,
            provenance: input.provenance.clone(),
            rights: input.rights.clone(),
            width: input.width,
            height: input.height,
        })
    }
}

/// Assets affect pixels and must invalidate the same source identity as text edits.
pub fn source_digest(
    sources: &super::types::SourceBundle,
    assets: &[super::records::Asset],
) -> DesignResult<String> {
    super::store::digest(&(sources, assets))
}

pub fn validate_image(bytes: &[u8], media: &str, width: u32, height: u32) -> DesignResult<()> {
    if bytes.len() > 10 * 1024 * 1024
        || width == 0
        || height == 0
        || u64::from(width) * u64::from(height) > 16_000_000
    {
        return Err(DesignError::BudgetExceeded(
            "asset byte or decoded pixel limit".into(),
        ));
    }
    let format = match media {
        "image/png" => ImageFormat::Png,
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::WebP,
        _ => {
            return Err(DesignError::InvalidInput(
                "assets must be PNG, JPEG or WebP; inline SVG belongs in source".into(),
            ))
        }
    };
    let reader = || -> DesignResult<ImageReader<Cursor<&[u8]>>> {
        let mut reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
        if reader.format() != Some(format) {
            return Err(DesignError::InvalidInput(
                "asset media type does not match bytes".into(),
            ));
        }
        let mut limits = Limits::default();
        limits.max_image_width = Some(width);
        limits.max_image_height = Some(height);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        Ok(reader)
    };
    let dimensions = reader()?
        .into_dimensions()
        .map_err(|_| DesignError::InvalidInput("asset dimensions cannot be decoded".into()))?;
    if dimensions != (width, height) {
        return Err(DesignError::InvalidInput(
            "asset dimensions do not match bytes".into(),
        ));
    }
    reader()?.decode().map_err(|_| {
        DesignError::InvalidInput("asset cannot be decoded within its limits".into())
    })?;
    Ok(())
}
