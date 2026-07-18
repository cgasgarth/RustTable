use rusttable_processing::{
    FiniteF32, ImageBuildError, LinearRgb, RasterDimensions, RgbChannel, SourceColorSpace,
    SourceRgb, SourceRgbImage, SrgbChannel, SrgbChannelError, WorkingColorSpace, WorkingRgbImage,
    to_linear_srgb,
};

fn channel(value: f32) -> SrgbChannel {
    SrgbChannel::new(value).expect("test channel is normalized and finite")
}

fn source_pixel(value: f32) -> SourceRgb {
    SourceRgb::new(channel(value), channel(value), channel(value))
}

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("test dimensions are nonzero")
}

fn image(dimensions: RasterDimensions, pixels: Vec<SourceRgb>) -> SourceRgbImage {
    SourceRgbImage::new(dimensions, pixels).expect("test image has matching pixels")
}

#[test]
fn rejects_zero_dimensions() {
    assert!(matches!(
        RasterDimensions::new(0, 1),
        Err(rusttable_processing::RasterDimensionsError::ZeroWidth)
    ));
    assert!(matches!(
        RasterDimensions::new(1, 0),
        Err(rusttable_processing::RasterDimensionsError::ZeroHeight)
    ));
}

#[test]
fn pixel_count_uses_widened_multiplication() {
    let dimensions = dimensions(u32::MAX, u32::MAX);

    assert_eq!(
        dimensions.pixel_count(),
        u64::from(u32::MAX) * u64::from(u32::MAX)
    );
}

#[test]
fn rejects_non_finite_and_out_of_range_srgb_channels() {
    assert_eq!(SrgbChannel::new(f32::NAN), Err(SrgbChannelError::NonFinite));
    assert_eq!(
        SrgbChannel::new(f32::INFINITY),
        Err(SrgbChannelError::NonFinite)
    );
    assert_eq!(
        SrgbChannel::new(f32::NEG_INFINITY),
        Err(SrgbChannelError::NonFinite)
    );
    assert_eq!(SrgbChannel::new(-0.01), Err(SrgbChannelError::BelowZero));
    assert_eq!(SrgbChannel::new(1.01), Err(SrgbChannelError::AboveOne));
}

#[test]
fn rejects_pixel_count_mismatch() {
    let dimensions = dimensions(2, 1);
    let too_short = SourceRgbImage::new(dimensions, vec![source_pixel(0.0)]);
    let too_long = SourceRgbImage::new(dimensions, vec![source_pixel(0.0); 3]);

    assert_eq!(
        too_short,
        Err(ImageBuildError::PixelCountMismatch {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(
        too_long,
        Err(ImageBuildError::PixelCountMismatch {
            expected: 2,
            actual: 3
        })
    );
}

#[test]
fn space_tags_are_fixed_by_image_type() {
    let source = image(dimensions(1, 1), vec![source_pixel(0.5)]);
    let linear = LinearRgb::new(
        FiniteF32::new(0.5).expect("finite"),
        FiniteF32::new(0.5).expect("finite"),
        FiniteF32::new(0.5).expect("finite"),
    );
    let working = WorkingRgbImage::new(dimensions(1, 1), vec![linear]).expect("one pixel");

    assert_eq!(source.space(), SourceColorSpace::Srgb);
    assert_eq!(working.space(), WorkingColorSpace::LinearSrgb);
}

#[test]
fn linear_samples_preserve_extended_range() {
    let pixel = LinearRgb::new(
        FiniteF32::new(-0.5).expect("finite"),
        FiniteF32::new(1.5).expect("finite"),
        FiniteF32::new(0.25).expect("finite"),
    );

    assert_eq!(pixel.red().get().to_bits(), (-0.5f32).to_bits());
    assert_eq!(pixel.green().get().to_bits(), 1.5f32.to_bits());
}

#[test]
fn converts_srgb_reference_values() {
    let source = image(
        dimensions(4, 1),
        vec![
            source_pixel(0.0),
            source_pixel(1.0),
            source_pixel(0.04045),
            source_pixel(0.5),
        ],
    );

    let working = to_linear_srgb(&source);
    let values = working
        .pixels()
        .map(|pixel| pixel.red().get())
        .collect::<Vec<_>>();

    assert!((values[0] - 0.0).abs() < 1e-7);
    assert!((values[1] - 1.0).abs() < 1e-7);
    assert!((values[2] - 0.003_130_8).abs() < 1e-5);
    assert!((values[3] - 0.214_041_14).abs() < 1e-5);
}

#[test]
fn converts_every_u8_srgb_code_value() {
    let pixels = (0..=u8::MAX)
        .map(|value| source_pixel(f32::from(value) / f32::from(u8::MAX)))
        .collect::<Vec<_>>();
    let source = image(dimensions(256, 1), pixels);
    let working = to_linear_srgb(&source);
    let values = working
        .pixels()
        .map(|pixel| pixel.red().get())
        .collect::<Vec<_>>();

    assert_eq!(values.len(), 256);
    assert!(
        values
            .iter()
            .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
    );
    assert!(values.windows(2).all(|window| window[0] <= window[1]));
    assert_eq!(values[0].to_bits(), 0.0f32.to_bits());
    assert_eq!(values[255].to_bits(), 1.0f32.to_bits());
}

#[test]
fn preserves_dimensions_space_and_row_major_order() {
    let source = image(
        dimensions(2, 1),
        vec![
            source_pixel(0.25),
            SourceRgb::new(channel(0.75), channel(0.5), channel(0.25)),
        ],
    );

    let working = to_linear_srgb(&source);

    assert_eq!(working.dimensions(), source.dimensions());
    assert_eq!(working.space(), WorkingColorSpace::LinearSrgb);
    assert!(working.pixels().next().expect("first pixel").red().get() < 0.1);
    assert!(working.pixels().nth(1).expect("second pixel").red().get() > 0.4);
}

#[test]
fn equal_sources_convert_equally() {
    let source = image(dimensions(1, 1), vec![source_pixel(0.5)]);

    assert_eq!(to_linear_srgb(&source), to_linear_srgb(&source));
}

#[test]
fn channel_enum_has_stable_attribution() {
    assert_ne!(RgbChannel::Red, RgbChannel::Green);
    assert_ne!(RgbChannel::Green, RgbChannel::Blue);
}
