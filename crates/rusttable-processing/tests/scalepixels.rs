#![allow(clippy::cast_precision_loss, clippy::float_cmp)]

use rusttable_processing::operations::{OperationExecutionError, scalepixels};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

use rusttable_image::{
    AlphaMode, ByteOrder, ChannelLayout, ColorEncoding, ImageDescriptor, ImageDimensions,
    ImageView, Orientation, PixelFormat, Roi, SampleType, StorageLayout,
};
use scalepixels::{
    MAX_OUTPUT_DIMENSION, ScalePixelsConfig, ScalePixelsConfigError, ScalePixelsHistory,
    ScalePixelsKernel, ScalePixelsParametersV1, ScalePixelsPlan, ScalePixelsPlanError,
    ScalePixelsPreferences, scalepixels_descriptor,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixel(value: f32) -> LinearRgb {
    let value = FiniteF32::new(value).expect("finite pixel");
    LinearRgb::new(value, value, value)
}

#[test]
fn descriptor_and_v1_payload_match_the_compatibility_contract() {
    let descriptor = scalepixels_descriptor();
    descriptor.validate().expect("valid descriptor");
    assert_eq!(descriptor.id.compatibility_name, "scalepixels");
    assert_eq!(descriptor.id.schema_version, 1);
    assert_eq!(descriptor.parameters[0].id, "pixel_aspect_ratio");

    let config = ScalePixelsConfig::new(1.5).expect("config");
    let parameters = ScalePixelsParametersV1::new(config);
    let decoded = ScalePixelsParametersV1::from_bytes(&parameters.to_bytes()).expect("decode");
    assert_eq!(decoded.config().pixel_aspect_ratio(), 1.5);
    assert_eq!(
        decoded.config().opaque_source(),
        Some(&parameters.to_bytes()[..])
    );
}

#[test]
fn range_validation_and_future_history_are_explicit() {
    assert!(matches!(
        ScalePixelsConfig::new(0.49),
        Err(ScalePixelsConfigError::AspectRatioOutOfRange { .. })
    ));
    assert!(matches!(
        ScalePixelsConfig::new(f32::NAN),
        Err(ScalePixelsConfigError::NonFiniteAspectRatio)
    ));
    let history = ScalePixelsHistory::decode(9, &[1, 2, 3]).expect("opaque history");
    assert_eq!(history.version(), 9);
    assert_eq!(history.payload(), vec![1, 2, 3]);
}

#[test]
fn ratios_resolve_darktable_geometry_and_exact_scales() {
    let vertical = ScalePixelsPlan::new(ScalePixelsConfig::new(0.5).unwrap(), dimensions(5, 3))
        .expect("vertical plan");
    assert_eq!(vertical.output_dimensions(), dimensions(5, 6));
    assert_eq!(vertical.x_scale(), 1.0);
    assert_eq!(vertical.y_scale(), 0.5);

    let horizontal = ScalePixelsPlan::new(ScalePixelsConfig::new(1.5).unwrap(), dimensions(5, 3))
        .expect("horizontal plan");
    assert_eq!(horizontal.output_dimensions(), dimensions(8, 3));
    assert_eq!(horizontal.x_scale(), 0.625);
    assert_eq!(horizontal.y_scale(), 1.0);

    let identity = ScalePixelsPlan::new(ScalePixelsConfig::defaults(), dimensions(1, 1))
        .expect("identity plan");
    assert!(identity.is_identity());
    assert_eq!(identity.output_dimensions(), dimensions(1, 1));
}

#[test]
fn roi_and_point_transforms_use_the_resolved_dimensions() {
    let plan =
        ScalePixelsPlan::new(ScalePixelsConfig::new(1.5).unwrap(), dimensions(5, 3)).expect("plan");
    let output = Roi::new(2, 1, 4, 2).unwrap();
    assert_eq!(plan.roi_in(output).unwrap(), Roi::new(1, 1, 3, 2).unwrap());

    let input = Roi::new(1, 1, 3, 2).unwrap();
    assert_eq!(plan.roi_out(input).unwrap(), Roi::new(1, 1, 6, 2).unwrap());

    let mut points = [2.0, 1.0, 4.0, 2.0];
    let original = points;
    plan.forward_transform(&mut points).unwrap();
    plan.back_transform(&mut points).unwrap();
    assert_eq!(points, original);
}

#[test]
fn scalar_cpu_resampling_supports_all_kernels_and_cancellation() {
    for kernel in [
        ScalePixelsKernel::Nearest,
        ScalePixelsKernel::Bilinear,
        ScalePixelsKernel::Bicubic,
        ScalePixelsKernel::Lanczos,
    ] {
        let preferences = ScalePixelsPreferences::new(kernel, kernel);
        let plan = ScalePixelsPlan::new_with_preferences(
            ScalePixelsConfig::new(2.0).unwrap(),
            dimensions(2, 1),
            preferences,
        )
        .unwrap();
        let input = vec![pixel(0.0), pixel(1.0)];
        let output = plan.execute(&input).expect("resample");
        assert_eq!(output.pixels().len(), 4);
        assert!(
            output
                .pixels()
                .iter()
                .all(|value| value.red().get().is_finite())
        );
    }

    let plan =
        ScalePixelsPlan::new(ScalePixelsConfig::new(0.5).unwrap(), dimensions(2, 2)).unwrap();
    let error = plan
        .execute_with_cancel(&[pixel(0.0); 4], || true)
        .expect_err("cancelled");
    assert!(matches!(error, OperationExecutionError::Cancelled));
}

#[test]
fn mask_resampling_is_global_and_clamped() {
    let plan = ScalePixelsPlan::new_with_preferences(
        ScalePixelsConfig::new(2.0).unwrap(),
        dimensions(2, 1),
        ScalePixelsPreferences::new(ScalePixelsKernel::Nearest, ScalePixelsKernel::Bilinear),
    )
    .unwrap();
    let input_roi = Roi::new(0, 0, 2, 1).unwrap();
    let output_roi = Roi::new(1, 0, 3, 1).unwrap();
    let output = plan
        .execute_mask(&[-1.0, 2.0], input_roi, output_roi)
        .expect("mask");
    assert_eq!(output, vec![0.5, 1.0, 1.0]);
}

#[test]
fn f32_image_execution_accepts_padded_rows_and_preserves_alpha() {
    let format = PixelFormat::new(
        SampleType::F32,
        ChannelLayout::Rgba,
        AlphaMode::Straight,
        ByteOrder::Native,
        StorageLayout::Interleaved,
    )
    .unwrap();
    let source_dimensions = ImageDimensions::new(2, 1).unwrap();
    let descriptor = ImageDescriptor::with_strides(
        source_dimensions,
        format,
        ColorEncoding::LinearSrgbD65,
        None,
        Orientation::Normal,
        &[40],
    )
    .unwrap();
    let mut input = vec![0_u8; descriptor.byte_length()];
    for (index, value) in [0.0_f32, 0.25, 0.5, 1.0, 1.0, 0.75, 0.5, 0.25]
        .into_iter()
        .enumerate()
    {
        input[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    let plan =
        ScalePixelsPlan::new(ScalePixelsConfig::new(2.0).unwrap(), dimensions(2, 1)).unwrap();
    let output = plan.execute_image(&descriptor, &input).expect("image");
    assert_eq!(output.descriptor().dimensions().width(), 4);
    assert_eq!(output.descriptor().format(), format);
    assert_eq!(output.descriptor().planes()[0].row_stride(), 64);
    let view = ImageView::new(output.descriptor(), output.bytes()).unwrap();
    let row = view.row(0, 0).unwrap();
    let alpha = f32::from_ne_bytes(row[12..16].try_into().unwrap());
    assert_eq!(alpha, 1.0);
}

#[test]
fn excessive_output_is_rejected_before_execution() {
    let plan = ScalePixelsPlan::new(
        ScalePixelsConfig::new(2.0).unwrap(),
        dimensions(MAX_OUTPUT_DIMENSION, 1),
    );
    assert!(matches!(plan, Err(ScalePixelsPlanError::OutputTooLarge)));
}

#[test]
fn gpu_dispatch_uses_exact_rois_strides_support_and_workgroups() {
    let plan = ScalePixelsPlan::new_with_preferences(
        ScalePixelsConfig::new(1.5).unwrap(),
        dimensions(5, 3),
        ScalePixelsPreferences::new(ScalePixelsKernel::Lanczos, ScalePixelsKernel::Bicubic),
    )
    .unwrap();
    let dispatch = plan
        .gpu_dispatch(
            Roi::new(0, 0, 5, 3).unwrap(),
            Roi::new(0, 0, 8, 3).unwrap(),
            80,
            128,
        )
        .expect("dispatch");
    assert_eq!(dispatch.image_support(), 3);
    assert_eq!(dispatch.mask_support(), 2);
    assert_eq!(dispatch.workgroups(), (1, 1));
    assert_eq!(dispatch.memory_bytes(), 80 * 3 + 128 * 3);
    assert!(plan.memory_estimate_bytes().unwrap() >= dispatch.memory_bytes());
}
