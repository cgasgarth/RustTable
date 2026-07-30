use std::fs;

use rusttable_image::{
    DecodedImage, ImageDimensions, ImageInput, ImageOutput, OutputLimits, OutputOptions,
};
use rusttable_image_io::png::{PngEncodeOptions, PngEncoder};
use rusttable_image_io::{
    FileImageOutput, PngBitDepth, PngColorType, PngDecodeLimits, PngDecodeRequest, PngDecoder,
    PngPixelData,
};

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
fn generic_png_output_routes_the_deterministic_leaf() {
    let first_destination = path("png-route-first.jpg");
    let second_destination = path("png-route-second.jpg");
    let image = image();
    let output = FileImageOutput::new(OutputLimits::new(1_000_000).unwrap());
    output
        .write_new(&image, &first_destination, OutputOptions::Png)
        .unwrap();
    output
        .write_new(&image, &second_destination, OutputOptions::Png)
        .unwrap();

    let first = fs::read(&first_destination).unwrap();
    let second = fs::read(&second_destination).unwrap();
    assert_eq!(first, second);
    let expected = PngEncoder::new()
        .encode(
            &PngPixelData::RgbaU8 {
                dimensions: image.dimensions(),
                samples: image.pixels().to_owned(),
            },
            PngEncodeOptions::new(5, 1_000_000),
        )
        .unwrap();
    assert_eq!(first, expected);

    let header = PngDecoder::new()
        .inspect_bytes(&first, PngDecodeLimits::standard())
        .unwrap();
    assert_eq!(header.color_type, PngColorType::Rgba);
    assert_eq!(header.bit_depth, PngBitDepth::Eight);
    let decoded = PngDecoder::new()
        .decode_bytes(&first, &PngDecodeRequest::new(PngDecodeLimits::standard()))
        .unwrap();
    assert_eq!(
        decoded.pixels,
        Some(PngPixelData::RgbaU8 {
            dimensions: image.dimensions(),
            samples: image.pixels().to_owned(),
        })
    );

    fs::remove_file(first_destination).unwrap();
    fs::remove_file(second_destination).unwrap();
}

#[test]
fn generic_png_output_enforces_the_completed_encoded_size() {
    let image = image();
    let expected = PngEncoder::new()
        .encode(
            &PngPixelData::RgbaU8 {
                dimensions: image.dimensions(),
                samples: image.pixels().to_owned(),
            },
            PngEncodeOptions::default(),
        )
        .expect("encoded PNG");
    let exact_destination = path("png-final-limit-exact");
    let exact_limit = u64::try_from(expected.len()).expect("encoded size");
    let receipt = FileImageOutput::new(OutputLimits::new(exact_limit).unwrap())
        .write_new(&image, &exact_destination, OutputOptions::Png)
        .expect("exact final-size limit");
    assert_eq!(receipt.encoded_byte_length(), exact_limit);
    assert_eq!(fs::read(&exact_destination).unwrap(), expected);
    fs::remove_file(exact_destination).unwrap();

    let rejected_destination = path("png-final-limit-rejected");
    let result = FileImageOutput::new(OutputLimits::new(exact_limit - 1).unwrap()).write_new(
        &image,
        &rejected_destination,
        OutputOptions::Png,
    );
    assert!(matches!(
        result,
        Err(rusttable_image::ImageOutputError::EncodedOutputTooLarge {
            actual,
            limit,
        }) if actual == exact_limit && limit == exact_limit - 1
    ));
    assert!(!rejected_destination.exists());
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
