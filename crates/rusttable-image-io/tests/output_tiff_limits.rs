use std::fs;

use rusttable_image::{DecodedImage, ImageDimensions, ImageOutput, OutputLimits, OutputOptions};
use rusttable_image_io::FileImageOutput;

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 1).unwrap(),
        vec![255, 0, 0, 128, 0, 255, 0, 64],
    )
    .unwrap()
}

fn destination(name: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rusttable-tiff-limit-{name}-{}",
        std::process::id()
    ));
    let _ = fs::remove_file(&path);
    path
}

fn encoded_length() -> u64 {
    let path = destination("measure");
    FileImageOutput::new(OutputLimits::new(1_000_000).unwrap())
        .write_new(&image(), &path, OutputOptions::Tiff)
        .unwrap();
    let length = fs::metadata(&path).unwrap().len();
    fs::remove_file(path).unwrap();
    length
}

#[test]
fn exact_tiff_limit_succeeds_and_first_byte_beyond_fails_before_publication() {
    let length = encoded_length();
    let exact_destination = destination("exact");
    FileImageOutput::new(OutputLimits::new(length).unwrap())
        .write_new(&image(), &exact_destination, OutputOptions::Tiff)
        .unwrap();
    assert_eq!(fs::metadata(&exact_destination).unwrap().len(), length);
    fs::remove_file(exact_destination).unwrap();

    let rejected_destination = destination("rejected");
    let result = FileImageOutput::new(OutputLimits::new(length - 1).unwrap()).write_new(
        &image(),
        &rejected_destination,
        OutputOptions::Tiff,
    );
    assert!(matches!(
        result,
        Err(rusttable_image::ImageOutputError::EncodedOutputTooLarge {
            limit,
            actual
        }) if limit == length - 1 && actual == length
    ));
    assert!(!rejected_destination.exists());
}
