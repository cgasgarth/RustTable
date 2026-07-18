use std::path::Path;

use rusttable_image::{
    ColorEncoding, DecodeLimits, DecodeLimitsError, DecodedImage, DecodedImageError,
    ImageDimensions, ImageInput, ImageInputError, ImageProbe, InputFormat, PixelLayout,
};

#[test]
fn dimensions_are_checked_and_report_rgba_size() {
    let dimensions = ImageDimensions::new(2, 3).expect("positive dimensions should construct");

    assert_eq!(dimensions.width(), 2);
    assert_eq!(dimensions.height(), 3);
    assert_eq!(dimensions.pixel_count(), Ok(6));
    assert_eq!(dimensions.decoded_byte_count(), Ok(24));
    assert_eq!(
        ImageDimensions::new(0, 1),
        Err(rusttable_image::ImageDimensionsError::ZeroWidth)
    );
    assert_eq!(
        ImageDimensions::new(1, 0),
        Err(rusttable_image::ImageDimensionsError::ZeroHeight)
    );
}

#[test]
fn supported_input_formats_have_stable_jpeg_png_tiff_order() {
    assert_eq!(
        rusttable_image::SUPPORTED_INPUT_FORMATS,
        [
            rusttable_image::InputFormat::Jpeg,
            rusttable_image::InputFormat::Png,
            rusttable_image::InputFormat::Tiff,
        ]
    );
}

#[test]
fn limits_require_nonzero_representable_bounds_and_keep_caps_independent() {
    assert_eq!(
        DecodeLimits::new(0, 2, 2, 4, 16),
        Err(DecodeLimitsError::ZeroLimit)
    );
    assert_eq!(
        DecodeLimits::new(100, 2, 2, 5, 20),
        Err(DecodeLimitsError::InconsistentPixelCount)
    );
    assert_eq!(
        DecodeLimits::new(100, 2, 2, 4, 17),
        Err(DecodeLimitsError::InconsistentDecodedBytes)
    );
    let max_pixels = u64::from(u32::MAX) * u64::from(u32::MAX);
    assert_eq!(
        DecodeLimits::new(100, u32::MAX, u32::MAX, max_pixels, u64::MAX),
        Err(DecodeLimitsError::ArithmeticOverflow)
    );

    let limits = DecodeLimits::new(100, 2, 2, 2, 4).expect("stricter caps are valid");
    assert_eq!(limits.max_pixel_count(), 2);
    assert_eq!(limits.max_decoded_bytes(), 4);
}

#[test]
fn decoded_image_exposes_immutable_checked_rgba8_data() {
    let dimensions = ImageDimensions::new(2, 1).expect("positive dimensions should construct");
    let pixels = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let image = DecodedImage::new(dimensions, pixels.clone()).expect("matching bytes should work");

    assert_eq!(image.dimensions(), dimensions);
    assert_eq!(image.layout(), PixelLayout::Rgba8StraightAlpha);
    assert_eq!(image.color_encoding(), ColorEncoding::Unspecified);
    assert_eq!(image.pixels(), pixels.as_slice());
    assert_eq!(
        DecodedImage::new(dimensions, vec![0; 4]),
        Err(DecodedImageError::ByteLengthMismatch {
            expected: 8,
            actual: 4,
        })
    );
}

#[test]
fn decoded_image_preserves_explicit_display_p3_tag() {
    let dimensions = ImageDimensions::new(1, 1).expect("positive dimensions should construct");
    let image = DecodedImage::new_with_color_encoding(
        dimensions,
        vec![1, 2, 3, 4],
        ColorEncoding::DisplayP3,
    )
    .expect("matching bytes should work");

    assert_eq!(image.color_encoding(), ColorEncoding::DisplayP3);
    assert_eq!(image.dimensions(), dimensions);
    assert_eq!(image.layout(), PixelLayout::Rgba8StraightAlpha);
}

struct FakeInput {
    probe: ImageProbe,
    image: DecodedImage,
}

impl ImageInput for FakeInput {
    fn probe_bytes(&self, _bytes: &[u8]) -> Result<ImageProbe, ImageInputError> {
        Ok(self.probe)
    }

    fn decode_bytes(&self, _bytes: &[u8]) -> Result<DecodedImage, ImageInputError> {
        Ok(self.image.clone())
    }

    fn probe_path(&self, _path: &Path) -> Result<ImageProbe, ImageInputError> {
        Ok(self.probe)
    }

    fn decode_path(&self, _path: &Path) -> Result<DecodedImage, ImageInputError> {
        Ok(self.image.clone())
    }
}

#[test]
fn image_input_is_object_safe() {
    let dimensions = ImageDimensions::new(1, 1).expect("positive dimensions should construct");
    let probe = ImageProbe::new(InputFormat::Png, dimensions);
    let image =
        DecodedImage::new(dimensions, vec![9, 8, 7, 6]).expect("matching bytes should work");
    let input: Box<dyn ImageInput> = Box::new(FakeInput { probe, image });

    assert_eq!(input.probe_path(Path::new("ignored")), Ok(probe));
    assert_eq!(
        input.decode_path(Path::new("ignored")).unwrap().pixels(),
        &[9, 8, 7, 6]
    );
}
