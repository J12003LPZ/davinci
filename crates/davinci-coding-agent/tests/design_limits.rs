use davinci_coding_agent::design::{records::Viewport, types::DesignLimits};

#[test]
fn viewport_and_default_resource_limits_are_bounded_before_capture() {
    for viewport in Viewport::defaults() {
        viewport.validate().unwrap();
    }
    for (width, height) in [
        (0, 900),
        (128, 0),
        (4097, 128),
        (128, 4097),
        (4096, 4096),
        (u32::MAX, u32::MAX),
    ] {
        assert!(Viewport { width, height }.validate().is_err());
    }
    let limits = DesignLimits::default();
    assert_eq!(limits.max_source_bytes, 2 * 1024 * 1024);
    assert_eq!(limits.max_render_bytes, 50 * 1024 * 1024);
    assert_eq!(limits.max_artifact_bytes, 100 * 1024 * 1024);
    assert_eq!(limits.max_task_bytes, 500 * 1024 * 1024);
}

#[test]
fn asset_dimensions_come_from_decoded_bytes_not_declared_metadata() {
    use davinci_coding_agent::design::assets::validate_image;
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::new(2, 3))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    validate_image(bytes.get_ref(), "image/png", 2, 3).unwrap();
    assert!(validate_image(bytes.get_ref(), "image/png", 3, 2).is_err());
    assert!(validate_image(bytes.get_ref(), "image/jpeg", 2, 3).is_err());
    assert!(validate_image(b"not an image", "image/png", 2, 3).is_err());
    assert!(validate_image(bytes.get_ref(), "image/png", u32::MAX, u32::MAX).is_err());
    let mut corrupt = bytes.into_inner();
    corrupt.truncate(corrupt.len() / 2);
    assert!(validate_image(&corrupt, "image/png", 2, 3).is_err());
}
