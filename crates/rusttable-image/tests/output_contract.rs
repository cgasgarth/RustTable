use std::cell::RefCell;
use std::path::{Path, PathBuf};

use rusttable_image::{
    ColorEncoding, DecodedImage, ImageDimensions, ImageOutput, ImageOutputError, JpegQuality,
    OutputFormat, OutputLimits, OutputOptions, OutputReceipt, PixelLayout,
    SUPPORTED_OUTPUT_FORMATS,
};

fn image() -> DecodedImage {
    DecodedImage::new(
        ImageDimensions::new(2, 1).expect("dimensions"),
        vec![255, 0, 0, 255, 0, 255, 0, 255],
    )
    .expect("image")
}

#[test]
fn output_formats_are_stable_and_options_are_closed() {
    assert_eq!(
        SUPPORTED_OUTPUT_FORMATS,
        [
            OutputFormat::Png,
            OutputFormat::Jpeg,
            OutputFormat::JpegXl,
            OutputFormat::Tiff,
            OutputFormat::Webp,
            OutputFormat::Avif,
            OutputFormat::Heif,
            OutputFormat::Heic,
            OutputFormat::Jpeg2000,
            OutputFormat::Jp2,
            OutputFormat::OpenExr,
        ]
    );
    assert_eq!(OutputOptions::Png.format(), OutputFormat::Png);
    let quality = JpegQuality::new(80).expect("quality");
    assert_eq!(OutputOptions::Jpeg { quality }.format(), OutputFormat::Jpeg);
    assert_eq!(OutputOptions::Tiff.format(), OutputFormat::Tiff);
    assert_eq!(quality.get(), 80);
}

#[test]
fn limits_and_quality_are_checked() {
    assert_eq!(
        JpegQuality::new(0),
        Err(rusttable_image::JpegQualityError::OutOfRange { value: 0 })
    );
    assert_eq!(
        JpegQuality::new(101),
        Err(rusttable_image::JpegQualityError::OutOfRange { value: 101 })
    );
    assert_eq!(JpegQuality::new(1).unwrap().get(), 1);
    assert_eq!(JpegQuality::new(100).unwrap().get(), 100);
    assert_eq!(
        OutputLimits::new(0),
        Err(rusttable_image::OutputLimitsError::ZeroEncodedBytes)
    );
    assert_eq!(OutputLimits::new(12).unwrap().max_encoded_bytes(), 12);
}

#[test]
fn receipt_is_checked_and_preserves_only_output_facts() {
    let receipt = OutputReceipt::new(
        PathBuf::from("exports/photo.png"),
        OutputFormat::Png,
        image().dimensions(),
        12,
    )
    .expect("receipt");
    assert_eq!(receipt.destination(), Path::new("exports/photo.png"));
    assert_eq!(receipt.format(), OutputFormat::Png);
    assert_eq!(receipt.dimensions(), ImageDimensions::new(2, 1).unwrap());
    assert_eq!(receipt.encoded_byte_length(), 12);
    assert_eq!(
        OutputReceipt::new(
            PathBuf::from("zero"),
            OutputFormat::Png,
            receipt.dimensions(),
            0
        ),
        Err(rusttable_image::OutputReceiptError::ZeroEncodedBytes)
    );
}

struct RecordingOutput {
    seen: RefCell<Option<(PathBuf, OutputOptions)>>,
}

impl ImageOutput for RecordingOutput {
    fn write_new(
        &self,
        image: &DecodedImage,
        destination: &Path,
        options: OutputOptions,
    ) -> Result<OutputReceipt, ImageOutputError> {
        self.seen
            .borrow_mut()
            .replace((destination.to_owned(), options));
        OutputReceipt::new(
            destination.to_owned(),
            options.format(),
            image.dimensions(),
            1,
        )
        .map_err(|_| ImageOutputError::AllocationFailure)
    }
}

#[test]
fn output_trait_is_object_safe_and_never_uses_extension_for_dispatch() {
    let fake = RecordingOutput {
        seen: RefCell::new(None),
    };
    let output: Box<dyn ImageOutput> = Box::new(fake);
    let quality = JpegQuality::new(90).unwrap();
    let receipt = output
        .write_new(
            &image(),
            Path::new("without-extension"),
            OutputOptions::Jpeg { quality },
        )
        .expect("recording output");
    assert_eq!(receipt.format(), OutputFormat::Jpeg);
    assert_eq!(receipt.encoded_byte_length(), 1);
}

#[test]
fn image_contract_has_explicit_rgba_and_unspecified_color_facts() {
    let value = image();
    assert_eq!(value.layout(), PixelLayout::Rgba8StraightAlpha);
    assert_eq!(value.color_encoding(), ColorEncoding::Unspecified);
}
