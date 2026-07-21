#![allow(clippy::float_cmp)]
#![forbid(unsafe_code)]

use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

pub mod descriptor {
    pub use rusttable_processing::descriptor::*;
}

#[path = "../src/operations/finalscale/mod.rs"]
#[allow(dead_code)]
mod finalscale;

use finalscale::{
    FinalScaleConfig, FinalScaleHistory, FinalScaleKernel, FinalScaleLimits,
    FinalScaleParametersV1, FinalScalePlan, FinalScalePlanError, RenderQuality, RenderSizeRequest,
};
use rusttable_image::Roi;
use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ColorEncoding, ImageDescriptor, ImageDimensions,
    ImageView, Orientation, PixelFormat, SampleType, StorageLayout,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixel(value: f32) -> LinearRgb {
    let value = FiniteF32::new(value).expect("finite pixel");
    LinearRgb::new(value, value, value)
}

#[test]
fn history_payload_is_typed_and_unknown_versions_remain_opaque() {
    let parameters = FinalScaleParametersV1::new([1, 2, 3, 4]);
    assert_eq!(
        FinalScaleParametersV1::from_bytes(&parameters.to_bytes()).unwrap(),
        parameters
    );
    assert_eq!(
        FinalScaleHistory::decode(1, &[1, 2, 3, 4])
            .unwrap()
            .payload(),
        &[1, 2, 3, 4]
    );
    let future = FinalScaleHistory::decode(9, &[5, 6]).unwrap();
    assert_eq!(future.version(), 9);
    assert_eq!(future.payload(), &[5, 6]);
    assert!(FinalScaleParametersV1::from_bytes(&[0; 3]).is_err());
}

#[test]
fn request_modes_resolve_with_one_rounding_rule_and_upscale_policy() {
    let source = dimensions(5, 3);
    let original = FinalScalePlan::new(source, RenderSizeRequest::Original).unwrap();
    assert_eq!(original.output_dimensions(), source);
    let exact = FinalScalePlan::new(source, RenderSizeRequest::exact(4, 2)).unwrap();
    assert_eq!(exact.output_dimensions(), dimensions(4, 2));

    let fit = FinalScalePlan::new(source, RenderSizeRequest::fit_within(8, 8)).unwrap();
    assert_eq!(fit.output_dimensions(), source);
    assert!(fit.upscale_suppressed());

    let long = FinalScalePlan::from_config(
        source,
        FinalScaleConfig::new(RenderSizeRequest::long_edge(8)).with_upscale(true),
    )
    .unwrap();
    assert_eq!(long.output_dimensions(), dimensions(8, 5));

    let short = FinalScalePlan::from_config(
        source,
        FinalScaleConfig::new(RenderSizeRequest::short_edge(6)).with_upscale(true),
    )
    .unwrap();
    assert_eq!(short.output_dimensions(), dimensions(10, 6));

    let megapixels = FinalScalePlan::from_config(
        dimensions(100, 50),
        FinalScaleConfig::new(RenderSizeRequest::megapixels(0.0001).unwrap()).with_upscale(true),
    )
    .unwrap();
    assert_eq!(megapixels.output_dimensions(), dimensions(14, 7));

    let print = FinalScalePlan::from_config(
        source,
        FinalScaleConfig::new(RenderSizeRequest::print(25.4, 12.7, 100.0).unwrap())
            .with_upscale(true),
    )
    .unwrap();
    assert_eq!(print.output_dimensions(), dimensions(100, 50));

    let no_axis_upscale = FinalScalePlan::new(source, RenderSizeRequest::exact(8, 2)).unwrap();
    assert_eq!(no_axis_upscale.output_dimensions(), dimensions(3, 2));
    assert!(no_axis_upscale.upscale_suppressed());
}

#[test]
fn limits_and_pipeline_scale_are_checked_before_publication() {
    let config = FinalScaleConfig::new(RenderSizeRequest::pipeline_scale(2.0).unwrap())
        .with_upscale(true)
        .with_limits(FinalScaleLimits::new(10, 160));
    assert!(matches!(
        FinalScalePlan::from_config(dimensions(4, 4), config),
        Err(FinalScalePlanError::OutputTooLarge { .. })
    ));

    let config = FinalScaleConfig::new(RenderSizeRequest::exact(4, 4))
        .with_limits(FinalScaleLimits::new(100, 15));
    assert!(matches!(
        FinalScalePlan::from_config(dimensions(4, 4), config),
        Err(FinalScalePlanError::OutputTooManyBytes { .. })
    ));
}

#[test]
fn coefficients_are_normalized_and_roi_transforms_round_trip() {
    let plan = FinalScalePlan::from_config(
        dimensions(4, 2),
        FinalScaleConfig::new(RenderSizeRequest::exact(7, 5))
            .with_quality(RenderQuality::export(FinalScaleKernel::Lanczos))
            .with_upscale(true),
    )
    .unwrap();
    for axis in [plan.coefficients_x(), plan.coefficients_y()] {
        for taps in axis.iter() {
            let sum: f32 = taps.iter().map(|tap| tap.weight()).sum();
            assert!((sum - 1.0).abs() < 1.0e-5);
        }
    }
    let output = Roi::new(2, 1, 3, 2).unwrap();
    let input = plan.modify_roi_in(output).unwrap();
    assert!(input.right() <= 4 && input.bottom() <= 2);
    let mapped = plan.modify_roi_out(input).unwrap();
    assert!(mapped.x() <= output.x() && mapped.right() >= output.right());

    let mut points = [1.5, 0.5, 3.0, 1.0];
    let original = points;
    plan.forward_transform(&mut points).unwrap();
    plan.back_transform(&mut points).unwrap();
    for (actual, expected) in points.into_iter().zip(original) {
        assert!((actual - expected).abs() < 1.0e-5);
    }
}

#[test]
fn cpu_resampling_supports_kernels_tiles_masks_and_cancellation() {
    for kernel in [
        FinalScaleKernel::Nearest,
        FinalScaleKernel::Bilinear,
        FinalScaleKernel::Bicubic,
        FinalScaleKernel::Lanczos,
    ] {
        let plan = FinalScalePlan::from_config(
            dimensions(2, 2),
            FinalScaleConfig::new(RenderSizeRequest::exact(4, 3))
                .with_quality(RenderQuality::export(kernel))
                .with_upscale(true),
        )
        .unwrap();
        let input = vec![pixel(0.0), pixel(1.0), pixel(2.0), pixel(3.0)];
        let output = plan.execute(&input).unwrap();
        assert_eq!(output.pixels().len(), 12);
        assert!(
            output
                .pixels()
                .iter()
                .all(|value| value.red().get().is_finite())
        );

        let mask = plan
            .execute_mask(
                &[0.0, 1.0, 1.0, 0.0],
                Roi::new(0, 0, 2, 2).unwrap(),
                Roi::new(0, 0, 4, 3).unwrap(),
            )
            .unwrap();
        assert_eq!(mask.len(), 12);
        assert!(mask.iter().all(|value| (0.0..=1.0).contains(value)));
        assert!(matches!(
            plan.execute_with_cancel(&input, || true),
            Err(finalscale::FinalScaleExecutionError::Cancelled)
        ));
    }
}

#[test]
fn image_contract_preserves_f32_rgba_metadata_and_padded_input() {
    let format = PixelFormat::new(
        SampleType::F32,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .unwrap();
    let input_dimensions = ImageDimensions::new(2, 1).unwrap();
    let descriptor = ImageDescriptor::with_strides(
        input_dimensions,
        format,
        ColorEncoding::LinearSrgbD65,
        None,
        Orientation::Normal,
        &[40],
    )
    .unwrap();
    let mut bytes = vec![0; descriptor.byte_length()];
    for (index, value) in [0.0_f32, 0.25, 0.5, 1.0, 1.0, 0.75, 0.5, 0.25]
        .into_iter()
        .enumerate()
    {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    let plan = FinalScalePlan::from_config(
        dimensions(2, 1),
        FinalScaleConfig::new(RenderSizeRequest::exact(4, 1))
            .with_quality(RenderQuality::export(FinalScaleKernel::Bilinear))
            .with_upscale(true),
    )
    .unwrap();
    let output = plan.execute_image(&descriptor, &bytes).unwrap();
    assert_eq!(output.descriptor().format(), format);
    assert_eq!(output.descriptor().dimensions().width(), 4);
    assert_eq!(output.descriptor().planes()[0].row_stride(), 64);
    let view = ImageView::new(output.descriptor(), output.bytes()).unwrap();
    assert_eq!(view.row(0, 0).unwrap().len(), 64);
    assert_eq!(output.identity(), plan.identity());
}
