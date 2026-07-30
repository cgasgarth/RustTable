use rusttable_image::{DecodedImage, ImageDimensions, ImageOutput, OutputLimits, OutputOptions};
use rusttable_image_io::FileImageOutput;
use std::fs;

#[test]
fn existing_destination_is_never_changed() {
    let destination = std::env::temp_dir().join("rusttable-output-existing");
    fs::write(&destination, b"keep this").unwrap();
    let image = DecodedImage::new(ImageDimensions::new(1, 1).unwrap(), vec![1, 2, 3, 255]).unwrap();
    let result = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap()).write_new(
        &image,
        &destination,
        OutputOptions::Png,
    );
    assert!(matches!(
        result,
        Err(rusttable_image::ImageOutputError::DestinationExists { .. })
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"keep this");
    fs::remove_file(destination).unwrap();
}

#[test]
fn existing_tiff_destination_is_never_changed() {
    let destination = std::env::temp_dir().join("rusttable-output-existing-tiff");
    fs::write(&destination, b"keep this TIFF destination").unwrap();
    let image = DecodedImage::new(ImageDimensions::new(1, 1).unwrap(), vec![1, 2, 3, 128]).unwrap();
    let result = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap()).write_new(
        &image,
        &destination,
        OutputOptions::Tiff,
    );
    assert!(matches!(
        result,
        Err(rusttable_image::ImageOutputError::DestinationExists { .. })
    ));
    assert_eq!(
        fs::read(&destination).unwrap(),
        b"keep this TIFF destination"
    );
    fs::remove_file(destination).unwrap();
}
