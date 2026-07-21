use rusttable_processing::operations::rotatepixels;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

use rotatepixels::{
    ROTATEPIXELS_PARAMETER_BYTES, ROTATEPIXELS_WGSL, RotatePixelsConfig, RotatePixelsInterpolation,
    RotatePixelsParametersV1, RotatePixelsPlan, RotatePixelsPlanError, decode_history,
    history_policy, rotatepixels_descriptor,
};
use rusttable_image::Roi;

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn config(rx: u32, ry: u32, angle: f32) -> RotatePixelsConfig {
    RotatePixelsConfig::new(RotatePixelsParametersV1::new(rx, ry, angle)).expect("config")
}

#[allow(clippy::cast_precision_loss)]
fn pixels(width: u32, height: u32) -> Vec<LinearRgb> {
    (0..height)
        .flat_map(|y| {
            (0..width).map(move |x| {
                let value = (y * width + x + 1) as f32;
                LinearRgb::new(
                    FiniteF32::new(value).expect("red"),
                    FiniteF32::new(value + 100.0).expect("green"),
                    FiniteF32::new(value + 200.0).expect("blue"),
                )
            })
        })
        .collect()
}

#[allow(clippy::cast_precision_loss)]
fn mask_values() -> Vec<f32> {
    (0..256).map(|value| value as f32 / 255.0).collect()
}

#[test]
fn dto_matches_darktable_version_one_and_unknown_history_is_opaque() {
    let parameters = RotatePixelsParametersV1::new(12, 34, -45.0);
    let bytes = parameters.to_bytes();
    assert_eq!(bytes.len(), ROTATEPIXELS_PARAMETER_BYTES);
    assert_eq!(
        RotatePixelsParametersV1::from_bytes(&bytes).expect("decode"),
        parameters
    );
    let opaque = decode_history(9, &[1, 2, 3]).expect("opaque history");
    assert!(
        matches!(opaque, rotatepixels::RotatePixelsHistoryParameters::Opaque { version: 9, ref bytes } if bytes == &[1, 2, 3])
    );
    assert!(RotatePixelsParametersV1::from_bytes(&[0; 8]).is_err());
}

#[test]
fn zero_center_is_disabled_but_valid_center_zero_angle_keeps_geometry() {
    let disabled = RotatePixelsPlan::new(
        dimensions(8, 8),
        config(0, 0, 33.0),
        RotatePixelsInterpolation::Lanczos,
    )
    .expect("disabled plan");
    assert!(!disabled.is_enabled());
    assert_eq!(disabled.output_dimensions(), dimensions(8, 8));

    let active = RotatePixelsPlan::new(
        dimensions(100, 100),
        config(1, 50, 0.0),
        RotatePixelsInterpolation::Bilinear,
    )
    .expect("zero-angle plan");
    assert!(active.is_enabled());
    assert_eq!(active.output_dimensions(), dimensions(68, 68));
}

#[test]
fn matrix_round_trip_and_point_batches_are_inverse() {
    let plan = RotatePixelsPlan::new(
        dimensions(100, 100),
        config(20, 50, -45.0),
        RotatePixelsInterpolation::Bilinear,
    )
    .expect("plan");
    let point = [21.0, 50.0];
    let rotated = plan.forward_point(point).expect("forward");
    assert!((rotated[0] - 0.707_106_77).abs() < 1.0e-5);
    assert!((rotated[1] - 0.707_106_77).abs() < 1.0e-5);
    let restored = plan.back_point(rotated).expect("back");
    assert!((restored[0] - point[0]).abs() < 1.0e-5);
    assert!((restored[1] - point[1]).abs() < 1.0e-5);

    let mut points = [21.0, 50.0, 20.0, 51.0];
    plan.forward_transform(&mut points).expect("batch forward");
    plan.back_transform(&mut points).expect("batch back");
    assert!(
        points
            .iter()
            .zip([21.0_f32, 50.0, 20.0, 51.0])
            .all(|(actual, expected)| (*actual - expected).abs() < 1.0e-5)
    );
}

#[test]
fn compatibility_bounds_round_down_to_even_and_include_kernel_support() {
    let source_dimensions = dimensions(100, 100);
    let nearest = RotatePixelsPlan::new(
        source_dimensions,
        config(1, 50, -45.0),
        RotatePixelsInterpolation::Nearest,
    )
    .expect("nearest");
    let lanczos = RotatePixelsPlan::new(
        source_dimensions,
        config(1, 50, -45.0),
        RotatePixelsInterpolation::Lanczos,
    )
    .expect("lanczos");
    assert_eq!(nearest.output_dimensions(), dimensions(68, 68));
    assert_eq!(lanczos.output_dimensions(), dimensions(62, 62));
    assert_eq!(
        nearest.source_roi(),
        Roi::new(0, 0, 100, 100).expect("source ROI")
    );
    assert_eq!(
        nearest.output_roi(),
        Roi::new(0, 0, 68, 68).expect("output ROI")
    );
    assert!(
        lanczos
            .modify_roi_in(lanczos.output_roi())
            .expect("input ROI")
            .width()
            >= 60
    );
}

#[test]
fn every_registered_kernel_executes_images_and_masks_deterministically() {
    for interpolation in RotatePixelsInterpolation::all() {
        let plan = RotatePixelsPlan::new(dimensions(16, 16), config(1, 8, 17.0), interpolation)
            .expect("plan");
        let input = pixels(16, 16);
        let first = plan.execute(&input).expect("image execution");
        let second = plan.execute(&input).expect("image execution");
        assert_eq!(first, second);
        assert_eq!(
            first.pixels().len() as u64,
            plan.output_dimensions().pixel_count()
        );
        let routed = plan
            .execute_plane(&mask_values(), 16)
            .expect("mask execution");
        assert_eq!(routed.len() as u64, plan.output_dimensions().pixel_count());
    }
}

#[test]
fn roi_transforms_use_checked_bounds_and_global_origins() {
    let plan = RotatePixelsPlan::new(
        dimensions(100, 100),
        config(1, 50, -45.0),
        RotatePixelsInterpolation::Bicubic,
    )
    .expect("plan");
    let requested = Roi::new(5, 7, 20, 20).expect("ROI");
    let output = plan.modify_roi_out(requested).expect("output ROI");
    assert_eq!(output.x(), 5);
    assert_eq!(output.y(), 7);
    assert_eq!(output.width() % 2, 0);
    assert_eq!(output.height() % 2, 0);
    assert!(plan.modify_roi_in(output).expect("input ROI").right() <= 100);
    assert!(plan.modify_roi_in(output).expect("input ROI").bottom() <= 100);
}

#[test]
fn cancellation_and_invalid_inputs_publish_nothing() {
    let plan = RotatePixelsPlan::new(
        dimensions(16, 16),
        config(1, 8, 17.0),
        RotatePixelsInterpolation::Bilinear,
    )
    .expect("plan");
    let input = pixels(16, 16);
    let error = plan
        .execute_with_cancel(&input, || true)
        .expect_err("cancelled");
    assert!(matches!(
        error,
        rotatepixels::RotatePixelsExecutionError::Cancelled
    ));
    assert!(matches!(
        plan.execute_interleaved(&[0.0; 4], 4, 64),
        Err(rotatepixels::RotatePixelsExecutionError::InvalidShape { .. })
    ));
    assert!(matches!(
        RotatePixelsPlan::new(
            dimensions(4, 4),
            config(4, 1, 0.0),
            RotatePixelsInterpolation::Nearest
        ),
        Err(RotatePixelsPlanError::CenterOutsideSource)
    ));
}

#[test]
fn hidden_history_policy_descriptor_cache_and_wgpu_contract_are_explicit() {
    let policy = history_policy();
    assert!(policy.hidden && policy.one_instance && policy.unsafe_copy);
    assert!(!policy.copy_to_user_stack && !policy.ordinary_controls);
    let descriptor = rotatepixels_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(descriptor.ui.is_none());
    let plan = RotatePixelsPlan::new(
        dimensions(16, 16),
        config(1, 8, 17.0),
        RotatePixelsInterpolation::Bilinear,
    )
    .expect("plan");
    let other = RotatePixelsPlan::new(
        dimensions(16, 16),
        config(1, 8, 17.0),
        RotatePixelsInterpolation::Lanczos,
    )
    .expect("plan");
    assert_ne!(plan.identity(), other.identity());
    assert!(ROTATEPIXELS_WGSL.contains("reflect_index"));
    assert!(ROTATEPIXELS_WGSL.contains("rotatepixels_lanczos"));
    assert_eq!(
        rotatepixels::wgpu_passes(),
        ["rotatepixels.image", "rotatepixels.mask"]
    );
}
