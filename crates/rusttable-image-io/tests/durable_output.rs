use std::fs;
use std::path::PathBuf;

use rusttable_image::{
    DecodedImage, DurableImageOutput, DurableOutputTag, ImageDimensions, OutputLimits,
    OutputOptions,
};
use rusttable_image_io::FileImageOutput;

fn destination(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rusttable-durable-output-{name}-{}",
        std::process::id()
    ))
}

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(1, 1).expect("dimensions"),
        vec![255, 0, 0, 255],
    )
    .expect("image")
}

#[test]
fn durable_png_publishes_by_options_not_destination_extension() {
    let destination = destination("png.jpg");
    let output = FileImageOutput::new(OutputLimits::new(4096).expect("limit"));
    let receipt = output
        .write_new_durable(&image(), &destination, OutputOptions::Png)
        .expect("directory sync should be supported");
    assert_eq!(
        receipt.durability(),
        DurableOutputTag::FileAndParentDirectorySynchronized
    );
    assert_eq!(receipt.format(), rusttable_image::OutputFormat::Png);
    assert_eq!(receipt.dimensions(), ImageDimensions::new(1, 1).unwrap());
    assert!(receipt.encoded_byte_length() > 0);
    assert!(
        fs::read(&destination)
            .expect("output reads")
            .starts_with(b"\x89PNG")
    );
    fs::remove_file(destination).expect("output removes");
}

#[test]
fn durable_jpeg_publishes_with_explicit_options() {
    let destination = destination("jpeg.png");
    let output = FileImageOutput::new(OutputLimits::new(4096).expect("limit"));
    let receipt = output
        .write_new_durable(
            &image(),
            &destination,
            OutputOptions::Jpeg {
                quality: rusttable_image::JpegQuality::new(90).unwrap(),
            },
        )
        .expect("directory sync should be supported");
    assert_eq!(receipt.format(), rusttable_image::OutputFormat::Jpeg);
    assert!(
        fs::read(&destination)
            .expect("output reads")
            .starts_with(&[0xff, 0xd8, 0xff])
    );
    fs::remove_file(destination).expect("output removes");
}

#[test]
fn durable_tiff_publishes_one_classic_image() {
    let destination = destination("tiff.jpg");
    let output = FileImageOutput::new(OutputLimits::new(4096).expect("limit"));
    let receipt = output
        .write_new_durable(&image(), &destination, OutputOptions::Tiff)
        .expect("directory sync should be supported");
    assert_eq!(receipt.format(), rusttable_image::OutputFormat::Tiff);
    assert!(
        fs::read(&destination)
            .expect("output reads")
            .starts_with(b"II*\0")
    );
    fs::remove_file(destination).expect("output removes");
}
