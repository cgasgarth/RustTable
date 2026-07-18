use std::fs;

use rusttable_image::{
    DecodedImage, ImageDimensions, ImageInput, ImageOutput, JpegQuality, OutputLimits,
    OutputOptions,
};
use rusttable_image_io::FileImageOutput;

fn path(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!("rusttable-output-{name}"));
    let _ = fs::remove_file(&path);
    path
}

#[test]
fn jpeg_output_is_opaque_and_uses_explicit_quality() {
    let image = DecodedImage::new(
        ImageDimensions::new(2, 1).unwrap(),
        vec![255, 0, 0, 255, 0, 255, 0, 255],
    )
    .unwrap();
    let destination = path("jpeg.no-extension");
    let quality = JpegQuality::new(90).unwrap();
    let receipt = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap())
        .write_new(&image, &destination, OutputOptions::Jpeg { quality })
        .unwrap();
    assert_eq!(receipt.format(), rusttable_image::OutputFormat::Jpeg);
    let input = rusttable_image_io::FileImageInput::new(
        rusttable_image::DecodeLimits::new(1_000_000, 2, 1, 2, 8).unwrap(),
    );
    let decoded = input.decode_path(&destination).unwrap();
    assert_eq!(decoded.dimensions(), image.dimensions());
    assert!(
        decoded
            .pixels()
            .chunks_exact(4)
            .all(|pixel| pixel[3] == 255)
    );
    fs::remove_file(destination).unwrap();
}

#[test]
fn nonopaque_jpeg_is_rejected_before_destination_creation() {
    let image = DecodedImage::new(ImageDimensions::new(1, 1).unwrap(), vec![1, 2, 3, 254]).unwrap();
    let destination = path("jpeg-alpha");
    let result = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap()).write_new(
        &image,
        &destination,
        OutputOptions::Jpeg {
            quality: JpegQuality::new(80).unwrap(),
        },
    );
    assert_eq!(
        result,
        Err(rusttable_image::ImageOutputError::NonOpaqueJpegInput { pixel_index: 0 })
    );
    assert!(!destination.exists());
}
