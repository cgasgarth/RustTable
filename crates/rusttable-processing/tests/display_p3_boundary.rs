use rusttable_processing::{
    DisplayP3Channel, DisplayP3ChannelError, DisplayP3Rgb, DisplayP3RgbImage, FiniteF32,
    RasterDimensions, SourceColorSpace, WorkingColorSpace, to_linear_srgb_from_display_p3,
};

fn channel(value: f32) -> DisplayP3Channel {
    DisplayP3Channel::new(value).unwrap()
}

#[test]
fn display_p3_channel_has_closed_normalized_boundaries() {
    assert!(channel(0.0).get().abs() < f32::EPSILON);
    assert!((channel(1.0).get() - 1.0).abs() < f32::EPSILON);
    assert_eq!(
        DisplayP3Channel::new(f32::NAN),
        Err(DisplayP3ChannelError::NonFinite)
    );
    assert_eq!(
        DisplayP3Channel::new(-0.01),
        Err(DisplayP3ChannelError::BelowZero)
    );
    assert_eq!(
        DisplayP3Channel::new(1.01),
        Err(DisplayP3ChannelError::AboveOne)
    );
    assert_eq!(channel(0.25), channel(0.25));
}

#[test]
fn display_p3_image_preserves_rows_and_converts_without_clipping() {
    let dimensions = RasterDimensions::new(2, 1).unwrap();
    let source = DisplayP3RgbImage::new(
        dimensions,
        vec![
            DisplayP3Rgb::new(channel(1.0), channel(0.0), channel(0.0)),
            DisplayP3Rgb::new(channel(0.5), channel(0.5), channel(0.5)),
        ],
    )
    .unwrap();
    let working = to_linear_srgb_from_display_p3(&source);

    assert_eq!(source.space(), SourceColorSpace::DisplayP3);
    assert_eq!(working.space(), WorkingColorSpace::LinearSrgb);
    assert_eq!(working.dimensions(), dimensions);
    assert_eq!(working, to_linear_srgb_from_display_p3(&source));
    let primary = working.pixel(0).unwrap();
    assert!((primary.red().get() - 1.224_940_2).abs() < 0.000_001);
    assert!((primary.green().get() + 0.042_056_955).abs() < 0.000_001);
    assert!((primary.blue().get() + 0.019_637_555).abs() < 0.000_001);
    assert!((working.pixel(1).unwrap().red().get() - 0.214_041_14).abs() < 0.000_001);
}

#[test]
fn display_p3_image_rejects_wrong_pixel_count() {
    let dimensions = RasterDimensions::new(2, 1).unwrap();
    let pixel = DisplayP3Rgb::new(channel(0.0), channel(0.0), channel(0.0));
    assert!(DisplayP3RgbImage::new(dimensions, vec![pixel]).is_err());
}

#[test]
fn display_p3_linear_values_remain_finite() {
    let source = DisplayP3RgbImage::new(
        RasterDimensions::new(1, 1).unwrap(),
        vec![DisplayP3Rgb::new(channel(1.0), channel(1.0), channel(1.0))],
    )
    .unwrap();
    let working = to_linear_srgb_from_display_p3(&source);
    let pixel = working.pixel(0).unwrap();
    assert!(FiniteF32::new(pixel.red().get()).is_ok());
    assert!(FiniteF32::new(pixel.green().get()).is_ok());
    assert!(FiniteF32::new(pixel.blue().get()).is_ok());
}
