#![allow(clippy::cast_precision_loss, clippy::default_trait_access)]

use rusttable_image::Roi;
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

pub mod descriptor {
    pub use rusttable_processing::descriptor::*;
}

#[path = "../src/operations/perspective/mod.rs"]
pub mod perspective;

use perspective::{
    AnalysisStatus, AutoMethod, BoundaryMode, FitAxis, Interpolation, LineSegment, LuminanceFrame,
    PerspectiveConfig, PerspectiveConfigError, PerspectiveParametersV1, PerspectiveParametersV5,
    PerspectivePlan, Point, analyze_lines, decode_history, detect_lines,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(value).expect("red"),
        FiniteF32::new(value + 1.0).expect("green"),
        FiniteF32::new(value + 2.0).expect("blue"),
    )
}

#[test]
fn default_plan_is_checked_identity_and_preserves_pixels() {
    let plan = PerspectivePlan::new(
        PerspectiveConfig::default(),
        dimensions(4, 3),
        Interpolation::Nearest,
    )
    .expect("plan");
    assert!(plan.is_identity());
    assert_eq!(plan.output_dimensions(), dimensions(4, 3));
    assert_eq!(plan.boundary_mode(), BoundaryMode::Reflect);
    let input: Vec<_> = (0..12).map(|value| pixel(value as f32)).collect();
    let output = plan.execute(&input).expect("execute");
    assert_eq!(output.pixels(), input.as_slice());
}

#[test]
fn manual_camera_transform_round_trips_points_and_expands_roi() {
    let config = PerspectiveConfig::default().with_method(AutoMethod::None, FitAxis::NONE);
    let plan =
        PerspectivePlan::new(config, dimensions(100, 80), Interpolation::Bilinear).expect("plan");
    let point = Point::new(17.25, 23.5);
    let mapped = plan.forward_point(point).expect("forward");
    let restored = plan.back_point(mapped).expect("back");
    assert!((restored.x() - point.x()).abs() < 1.0e-8);
    assert!((restored.y() - point.y()).abs() < 1.0e-8);
    let roi = plan
        .input_roi(Roi::new(4, 5, 20, 20).expect("ROI"))
        .expect("input ROI");
    assert!(roi.right() <= 100 && roi.bottom() <= 80);
}

#[test]
fn line_analysis_is_stable_and_builds_a_rectification() {
    let lines = [
        LineSegment::new(Point::new(20.0, 90.0), Point::new(44.0, 10.0), 1.0, 1.0)
            .expect("vertical"),
        LineSegment::new(Point::new(70.0, 90.0), Point::new(53.0, 10.0), 1.0, 1.0)
            .expect("vertical"),
        LineSegment::new(Point::new(10.0, 25.0), Point::new(90.0, 49.0), 1.0, 1.0)
            .expect("horizontal"),
        LineSegment::new(Point::new(10.0, 70.0), Point::new(90.0, 53.0), 1.0, 1.0)
            .expect("horizontal"),
    ];
    let first = analyze_lines(&lines, Default::default());
    let second = analyze_lines(&lines, Default::default());
    assert_eq!(first, second);
    assert_eq!(first.status(), AnalysisStatus::Ready);
    assert!(first.correction().is_some());
    assert!(first.confidence() > 0.0);
}

#[test]
fn luminance_detector_finds_axis_aligned_structure_deterministically() {
    let dims = dimensions(32, 24);
    let pixels = (0..24)
        .flat_map(|y| (0..32).map(move |x| if x == 8 || y == 12 { 1.0 } else { 0.0 }))
        .collect();
    let frame = LuminanceFrame::new(dims, pixels).expect("frame");
    let first = detect_lines(&frame, Default::default()).expect("analysis");
    let second = detect_lines(&frame, Default::default()).expect("analysis");
    assert_eq!(first, second);
    assert!(
        first
            .lines()
            .iter()
            .any(|line| line.kind() == perspective::LineKind::Vertical)
    );
    assert!(
        first
            .lines()
            .iter()
            .any(|line| line.kind() == perspective::LineKind::Horizontal)
    );
}

#[test]
fn insufficient_auto_analysis_does_not_publish_a_plan() {
    let line =
        LineSegment::new(Point::new(1.0, 1.0), Point::new(20.0, 2.0), 1.0, 1.0).expect("line");
    let analysis = analyze_lines(&[line], Default::default());
    assert_eq!(analysis.status(), AnalysisStatus::InsufficientLines);
    let result = PerspectivePlan::from_analysis(
        PerspectiveConfig::default(),
        dimensions(32, 32),
        &analysis,
        Interpolation::Lanczos3,
    );
    assert!(result.is_err());
}

#[test]
fn image_and_mask_use_the_same_inverse_sampling_and_cancellation_publishes_nothing() {
    let plan = PerspectivePlan::new(
        PerspectiveConfig::default(),
        dimensions(8, 8),
        Interpolation::Bilinear,
    )
    .expect("plan");
    let input: Vec<_> = (0..64).map(|value| pixel(value as f32)).collect();
    assert!(matches!(
        plan.execute_with_cancel(&input, || true),
        Err(perspective::PerspectiveExecutionError::Cancelled)
    ));
    let mask: Vec<_> = (0..64).map(|value| value as f32 / 63.0).collect();
    let output = plan.execute_plane(&mask, || false).expect("mask");
    assert_eq!(output, mask);
}

#[test]
fn history_v1_migrates_and_unknown_versions_are_opaque() {
    let value = PerspectiveParametersV1::new(2.0, -0.25, 0.5, 0);
    assert!(matches!(
        decode_history(1, &value.to_bytes()).expect("decode"),
        perspective::PerspectiveHistory::V1(_)
    ));
    assert!(
        matches!(decode_history(99, &[1, 2, 3]).expect("opaque"), perspective::PerspectiveHistory::Opaque { version: 99, ref bytes } if bytes == &[1, 2, 3])
    );
    let config = PerspectiveConfig::from_parameters(PerspectiveParametersV5 {
        rotation: 181.0,
        ..Default::default()
    });
    assert!(matches!(
        config,
        Err(PerspectiveConfigError::OutOfRange {
            field: "rotation",
            ..
        })
    ));
    let current = PerspectiveParametersV5::default();
    assert!(matches!(
        decode_history(5, &current.to_bytes()).expect("current history"),
        perspective::PerspectiveHistory::V5(_)
    ));
}

#[test]
fn descriptor_matches_perspective_schema_and_validates() {
    let descriptor = perspective::perspective_descriptor();
    descriptor.validate().expect("descriptor");
    assert_eq!(
        descriptor.parameters.len(),
        perspective::ASHIFT_DESCRIPTOR_PARAMETER_COUNT
    );
    assert_eq!(descriptor.id.compatibility_name, "ashift");
    assert_eq!(descriptor.id.parameter_version, 5);
    assert_eq!(descriptor.migration.source_versions, [1, 2, 3, 4, 5]);
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::FULL_IMAGE)
    );
    assert!(
        descriptor
            .flags
            .contains(rusttable_processing::descriptor::OperationFlags::ANALYSIS)
    );
    assert_eq!(
        descriptor
            .parameters
            .iter()
            .filter(|parameter| parameter.id.starts_with("last_drawn_line_"))
            .count(),
        perspective::ASHIFT_DESCRIPTOR_LINE_PARAMETER_COUNT
    );
}
