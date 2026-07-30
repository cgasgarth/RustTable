use std::fs;

use rusttable_image::{
    DecodedImage, ImageDimensions, ImageInput, ImageOutput, OutputLimits, OutputOptions,
};
use rusttable_image_io::FileImageOutput;

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 1).unwrap(),
        vec![255, 0, 0, 255, 0, 255, 0, 255],
    )
    .unwrap()
}
fn path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("rusttable-output-{name}"));
    let _ = fs::remove_file(&path);
    path
}

#[test]
fn png_output_uses_options_and_preserves_rgba_samples() {
    let destination = path("png.jpg");
    let receipt = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap())
        .write_new(&image(), &destination, OutputOptions::Png)
        .unwrap();
    assert_eq!(receipt.format(), rusttable_image::OutputFormat::Png);
    let input = rusttable_image_io::FileImageInput::new(
        rusttable_image::DecodeLimits::new(1_000_000, 2, 1, 2, 8).unwrap(),
    );
    assert_eq!(
        input.decode_path(&destination).unwrap().pixels(),
        image().pixels()
    );
    fs::remove_file(destination).unwrap();
}

#[test]
fn too_small_png_limit_creates_no_destination() {
    let destination = path("png-limit");
    let result = FileImageOutput::new(OutputLimits::new(1).unwrap()).write_new(
        &image(),
        &destination,
        OutputOptions::Png,
    );
    assert!(matches!(
        result,
        Err(rusttable_image::ImageOutputError::EncodedOutputTooLarge { .. })
    ));
    assert!(!destination.exists());
}
