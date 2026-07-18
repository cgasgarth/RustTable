use std::fs;

use rusttable_image::{
    DecodeLimits, DecodedImage, ImageDimensions, ImageInput, ImageOutput, OutputFormat,
    OutputLimits, OutputOptions,
};
use rusttable_image_io::{FileImageInput, FileImageOutput};

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 1).unwrap(),
        vec![255, 0, 0, 128, 0, 255, 0, 64],
    )
    .unwrap()
}

fn destination(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusttable-tiff-output-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    path
}

#[test]
fn tiff_output_is_explicit_classic_rgba8_and_round_trips_exactly() {
    let destination = destination("round-trip.png");
    let source = image();
    let receipt = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap())
        .write_new(&source, &destination, OutputOptions::Tiff)
        .unwrap();

    assert_eq!(receipt.format(), OutputFormat::Tiff);
    let bytes = fs::read(&destination).unwrap();
    assert!(bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*"));
    let decoded = FileImageInput::new(DecodeLimits::new(1_000_000, 2, 1, 2, 8).unwrap())
        .decode_path(&destination)
        .unwrap();
    assert_eq!(decoded.dimensions(), source.dimensions());
    assert_eq!(decoded.pixels(), source.pixels());
    fs::remove_file(destination).unwrap();
}

#[test]
fn tiff_output_uses_options_not_destination_extension() {
    let destination = destination("explicit.jpeg");
    FileImageOutput::new(OutputLimits::new(1_000_000).unwrap())
        .write_new(&image(), &destination, OutputOptions::Tiff)
        .unwrap();
    assert!(fs::read(&destination).unwrap().starts_with(b"II*\0"));
    fs::remove_file(destination).unwrap();
}
