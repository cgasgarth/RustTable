use rusttable_processing::operations::flip;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

use flip::{
    FlipConfig, FlipParametersV1, FlipParametersV2, FlipPlan, ORIENTATION_NULL, OrientationBits,
    migrate, migrate_v1, migrate_v1_with_source, migrate_v2,
};
use rusttable_image::{CfaDescriptor, CfaPattern, CfaPhase, ImageDimensions, Orientation, Roi};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
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

fn plan(width: u32, height: u32, orientation: Orientation) -> FlipPlan {
    FlipPlan::new(
        dimensions(width, height),
        FlipConfig::explicit(OrientationBits::from_orientation(orientation)),
        Orientation::Normal,
    )
    .expect("plan")
}

#[test]
fn every_dihedral_mapping_is_bijective_and_has_the_expected_shape() {
    let source = dimensions(3, 2);
    for orientation in [
        Orientation::Normal,
        Orientation::FlipHorizontal,
        Orientation::Rotate180,
        Orientation::FlipVertical,
        Orientation::Transpose,
        Orientation::Rotate90,
        Orientation::Transverse,
        Orientation::Rotate270,
    ] {
        let plan = plan(3, 2, orientation);
        let output = plan.output_dimensions();
        assert_eq!(
            output.width(),
            orientation
                .output_dimensions(ImageDimensions::new(3, 2).expect("image dimensions"))
                .width()
        );
        assert_eq!(
            output.height(),
            orientation
                .output_dimensions(ImageDimensions::new(3, 2).expect("image dimensions"))
                .height()
        );

        let mut seen = Vec::new();
        for y in 0..source.height() {
            for x in 0..source.width() {
                let mapped = plan.forward(x, y).expect("forward coordinate");
                assert!(!seen.contains(&mapped));
                seen.push(mapped);
                assert_eq!(plan.inverse(mapped.0, mapped.1).expect("inverse"), (x, y));
            }
        }
        assert_eq!(seen.len(), 6);
    }
}

#[test]
fn scalar_execution_routes_odd_grid_pixels_without_interpolation() {
    let plan = plan(3, 2, Orientation::Rotate90);
    let output = plan.execute(&pixels(3, 2)).expect("execution");
    let expected = [4.0, 1.0, 5.0, 2.0, 6.0, 3.0];
    let actual: Vec<_> = output
        .pixels()
        .iter()
        .map(|pixel| pixel.red().get())
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(output.dimensions(), dimensions(2, 3));
    assert_eq!(output.receipt().output_orientation(), Orientation::Normal);
}

#[test]
fn automatic_mode_snapshots_source_orientation_in_plan_and_receipt() {
    let config = FlipConfig::automatic();
    let first = FlipPlan::new(dimensions(2, 3), config.clone(), Orientation::Rotate90)
        .expect("automatic plan");
    let second = FlipPlan::new(dimensions(2, 3), config, Orientation::FlipHorizontal)
        .expect("automatic plan");
    assert_eq!(first.resolved_orientation(), Orientation::Rotate90);
    assert_eq!(first.output_dimensions(), dimensions(3, 2));
    assert_ne!(first.identity(), second.identity());
    assert_eq!(
        first
            .execute(&pixels(2, 3))
            .expect("execute")
            .receipt()
            .source_orientation(),
        Orientation::Rotate90
    );
}

#[test]
fn explicit_orientation_overrides_source_metadata() {
    let plan = FlipPlan::new(
        dimensions(2, 3),
        FlipConfig::explicit(OrientationBits::from_orientation(Orientation::FlipVertical)),
        Orientation::Rotate90,
    )
    .expect("explicit plan");
    assert_eq!(plan.resolved_orientation(), Orientation::FlipVertical);
    assert_eq!(plan.output_dimensions(), dimensions(2, 3));
}

#[test]
fn migrations_preserve_darktable_values_and_defaults() {
    let v1 = FlipParametersV1 { orientation: 6 };
    assert_eq!(migrate_v1(v1), FlipParametersV2 { orientation: 6 });
    assert_eq!(
        migrate_v2(FlipParametersV2::default()).expect("automatic"),
        FlipConfig::automatic()
    );
    assert_eq!(
        migrate(1, v1).expect("v1").orientation(),
        OrientationBits::new(6).expect("bits")
    );
    assert_eq!(
        migrate_v1_with_source(v1, Orientation::FlipHorizontal)
            .expect("source-aware v1")
            .orientation,
        7
    );
    assert!(migrate(2, FlipParametersV1 { orientation: 8 }).is_err());
    assert_eq!(ORIENTATION_NULL, -1);
}

#[test]
fn cfa_roi_and_padded_mask_paths_share_the_pixel_mapping() {
    let dimensions = ImageDimensions::new(3, 2).expect("image dimensions");
    let cfa = CfaDescriptor::new(
        CfaPattern::bayer_rggb(),
        CfaPhase::new(1, 0, CfaPattern::bayer_rggb()),
    );
    let plan = plan(3, 2, Orientation::Rotate90);
    let output_cfa = plan.output_cfa(dimensions, cfa);
    assert_eq!(output_cfa.phase().x(), 1);
    assert_eq!(output_cfa.phase().y(), 1);

    let input_roi = Roi::new(1, 0, 2, 2).expect("input ROI");
    let output_roi = plan.output_roi(input_roi).expect("output ROI");
    assert_eq!(output_roi, Roi::new(0, 1, 2, 2).expect("output ROI"));
    assert_eq!(plan.input_roi(output_roi).expect("input ROI"), input_roi);

    let padded = vec![1_u8, 2, 3, 99, 4, 5, 6, 98];
    let routed = plan.execute_plane(&padded, 4).expect("mask routing");
    assert_eq!(routed, vec![4, 1, 5, 2, 6, 3]);
}

#[test]
fn cancellation_and_shape_fail_before_publishing_partial_pixels() {
    let plan = plan(2, 2, Orientation::FlipHorizontal);
    assert!(matches!(
        plan.execute_with_cancel(&pixels(2, 2), || true),
        Err(flip::FlipExecutionError::Cancelled)
    ));
    assert!(matches!(
        plan.execute(&pixels(2, 1)),
        Err(flip::FlipExecutionError::InvalidShape { .. })
    ));
}
