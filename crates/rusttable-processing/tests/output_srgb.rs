use rusttable_processing::{
    EncodedSrgbOutput, FiniteF32, GamutClipReport, LinearRgb, RasterDimensions, RgbChannel,
    SourceRgb, SourceRgbImage, SrgbChannel, WorkingRgbImage, encode_linear_srgb,
    encode_working_to_srgb, to_linear_srgb,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("test dimensions are nonzero")
}

fn scalar(value: f32) -> FiniteF32 {
    FiniteF32::new(value).expect("test scalar is finite")
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(scalar(red), scalar(green), scalar(blue))
}

fn working(pixels: Vec<LinearRgb>) -> WorkingRgbImage {
    let width = u32::try_from(pixels.len()).expect("test image fits in u32");
    WorkingRgbImage::new(dimensions(width, 1), pixels).expect("test image matches dimensions")
}

fn channel(value: f32) -> SrgbChannel {
    SrgbChannel::new(value).expect("test channel is normalized")
}

fn source_pixel(red: f32, green: f32, blue: f32) -> SourceRgb {
    SourceRgb::new(channel(red), channel(green), channel(blue))
}

fn channel_bits(output: &EncodedSrgbOutput, index: usize, channel: RgbChannel) -> u32 {
    output
        .image()
        .pixel(index)
        .expect("test pixel exists")
        .channel(channel)
        .get()
        .to_bits()
}

#[test]
fn black_and_white_encode_exactly() {
    let output = encode_linear_srgb(&working(vec![pixel(0.0, 0.0, 0.0), pixel(1.0, 1.0, 1.0)]));

    for pixel_index in 0..2 {
        for channel in [RgbChannel::Red, RgbChannel::Green, RgbChannel::Blue] {
            let expected = if pixel_index == 0 { 0.0 } else { 1.0 };
            assert_eq!(
                channel_bits(&output, pixel_index, channel),
                expected_f32(expected)
            );
        }
    }
    assert_eq!(output.clipping(), GamutClipReport::default());
}

fn expected_f32(value: f32) -> u32 {
    value.to_bits()
}

#[test]
fn uses_both_standard_transfer_branches() {
    let threshold = 0.003_130_8_f32;
    let output = encode_linear_srgb(&working(vec![
        pixel(threshold, threshold, threshold),
        pixel(
            threshold + 0.000_001,
            threshold + 0.000_001,
            threshold + 0.000_001,
        ),
    ]));
    let low_expected = (12.92 * threshold).to_bits();
    let high_expected = (1.055 * (threshold + 0.000_001).powf(1.0 / 2.4) - 0.055).to_bits();

    assert_eq!(channel_bits(&output, 0, RgbChannel::Red), low_expected);
    assert_eq!(channel_bits(&output, 1, RgbChannel::Red), high_expected);
}

#[test]
fn colorimetric_srgb_conversion_preserves_neutral_luminance_and_transfer() {
    let linear_mid_grey = 0.18;
    let expected_srgb = 0.461_356_12_f32;
    let input = working(vec![pixel(
        linear_mid_grey,
        linear_mid_grey,
        linear_mid_grey,
    )]);

    let fallback = encode_working_to_srgb(&input);
    let direct = encode_linear_srgb(&input);
    let output = fallback.image().pixel(0).expect("fallback pixel exists");
    let luminance = 0.2126_f32.mul_add(
        output.red().get(),
        0.7152_f32.mul_add(output.green().get(), 0.0722 * output.blue().get()),
    );

    assert_eq!(fallback, direct);
    assert!((output.red().get() - expected_srgb).abs() < 0.000_001);
    assert!((luminance - expected_srgb).abs() < 0.000_001);
}

#[test]
fn source_to_linear_to_output_round_trip() {
    let source = SourceRgbImage::new(
        dimensions(4, 1),
        vec![
            source_pixel(0.0, 0.1, 0.25),
            source_pixel(0.5, 0.75, 1.0),
            source_pixel(0.04045, 0.2, 0.8),
            source_pixel(0.9, 0.01, 0.33),
        ],
    )
    .expect("test source matches dimensions");
    let output = encode_linear_srgb(&to_linear_srgb(&source));

    for (index, source_pixel) in source.pixels().enumerate() {
        let encoded = output.image().pixel(index).expect("output pixel exists");
        for channel in [RgbChannel::Red, RgbChannel::Green, RgbChannel::Blue] {
            assert!(
                (encoded.channel(channel).get() - source_pixel.channel(channel).get()).abs()
                    < 0.000_01
            );
        }
    }
}

#[test]
fn reports_every_clipped_channel_by_direction() {
    let output = encode_linear_srgb(&working(vec![
        pixel(-1.0, -2.0, 0.0),
        pixel(2.0, 3.0, 4.0),
        pixel(-5.0, 0.0, 6.0),
    ]));
    let clipping = output.clipping();

    assert_eq!(clipping.below_zero().red(), 2);
    assert_eq!(clipping.below_zero().green(), 1);
    assert_eq!(clipping.below_zero().blue(), 0);
    assert_eq!(clipping.above_one().red(), 1);
    assert_eq!(clipping.above_one().green(), 1);
    assert_eq!(clipping.above_one().blue(), 2);
}

#[test]
fn clipping_happens_only_at_output_boundary() {
    let input = working(vec![pixel(-0.5, 1.5, -0.0)]);
    let before = input.clone();
    let output = encode_linear_srgb(&input);

    assert_eq!(input, before);
    assert_eq!(output.image().dimensions(), input.dimensions());
    assert_eq!(output.image().pixel_slice().len(), 1);
    assert_eq!(
        input
            .pixel(0)
            .expect("input pixel exists")
            .red()
            .get()
            .to_bits(),
        (-0.5f32).to_bits()
    );
    assert_eq!(output.clipping().below_zero().blue(), 0);
}

#[test]
fn equal_working_images_have_equal_output_and_reports() {
    let left = encode_linear_srgb(&working(vec![pixel(0.1, 0.2, 0.3), pixel(-1.0, 2.0, 0.0)]));
    let right = encode_linear_srgb(&working(vec![pixel(0.1, 0.2, 0.3), pixel(-1.0, 2.0, 0.0)]));

    assert_eq!(left, right);
}
