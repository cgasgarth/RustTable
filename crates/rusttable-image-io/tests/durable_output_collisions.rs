use std::fs;

use rusttable_image::{
    DecodedImage, DurableImageOutput, DurableImageOutputError, ImageDimensions, ImageOutputError,
    OutputLimits, OutputOptions,
};
use rusttable_image_io::FileImageOutput;

#[test]
fn existing_destination_is_unchanged_and_is_reported_before_publication() {
    let destination = std::env::temp_dir().join(format!(
        "rusttable-durable-collision-{}",
        std::process::id()
    ));
    fs::write(&destination, b"original").expect("destination writes");
    let image = DecodedImage::new(ImageDimensions::new(1, 1).unwrap(), vec![1, 2, 3, 255]).unwrap();
    let output = FileImageOutput::new(OutputLimits::new(4096).unwrap());
    let error = output
        .write_new_durable(&image, &destination, OutputOptions::Png)
        .unwrap_err();
    assert!(matches!(
        error,
        DurableImageOutputError::BeforePublication {
            source: ImageOutputError::DestinationExists { .. }
        }
    ));
    assert_eq!(fs::read(&destination).unwrap(), b"original");
    fs::remove_file(destination).expect("destination removes");
}
