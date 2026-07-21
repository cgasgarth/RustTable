pub use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};
pub mod descriptor {
    pub use rusttable_processing::descriptor::*;
}

#[path = "../src/operations/lenscorrection/mod.rs"]
pub mod lenscorrection;

use lenscorrection::{
    CorrectionFlags, LENSFUN_DATABASE_COMMIT, LensCorrectionConfig,
    LensCorrectionHistoryParameters, LensCorrectionMethod, LensCorrectionMode,
    LensCorrectionParametersV1, LensCorrectionPlan, LensfunSnapshot, decode_history,
    encode_history, lenscorrection_descriptor,
};
use rusttable_image::Roi;

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn canon_parameters() -> LensCorrectionParametersV1 {
    LensCorrectionParametersV1::new("EOS 5D Mark II", "Canon EF 24-70mm f/2.8L USM", 24.0, 2.8)
        .expect("parameters")
}

fn sony_parameters() -> LensCorrectionParametersV1 {
    LensCorrectionParametersV1::new("Alpha 7 III", "FE 24-105mm f/4 G OSS", 50.0, 4.0)
        .expect("parameters")
}

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(value).expect("red"),
        FiniteF32::new(value + 0.1).expect("green"),
        FiniteF32::new(value + 0.2).expect("blue"),
    )
}

fn points_differ(left: [f32; 2], right: [f32; 2]) -> bool {
    left.iter()
        .zip(right)
        .any(|(left, right)| (*left - right).abs() > 1.0e-6)
}

#[test]
fn snapshot_matching_is_case_insensitive_and_pinned() {
    let snapshot = LensfunSnapshot::pinned();
    assert_eq!(snapshot.commit, LENSFUN_DATABASE_COMMIT);
    let camera = snapshot
        .find_camera("sony", "Alpha 7 III")
        .expect("camera alias");
    let lens = snapshot
        .find_lens(Some(camera), "FE 24-105mm f/4 G OSS (Sony)")
        .expect("sanitized lens");
    assert_eq!(lens.maker, "Sony");
    assert!(lens.tca_at(50.0).is_some());
    assert!(snapshot.find_lens(None, "not in snapshot").is_none());
}

#[test]
fn history_round_trip_and_future_payload_are_lossless() {
    let parameters = canon_parameters();
    let encoded = encode_history(&parameters);
    let imported = LensCorrectionConfig::new(parameters.clone())
        .expect("config")
        .with_opaque_source(vec![9, 8, 7]);
    assert_eq!(imported.history_bytes(), [9, 8, 7]);
    assert!(matches!(
        decode_history(1, &encoded).expect("history"),
        LensCorrectionHistoryParameters::V1(value) if value == parameters
    ));
    assert!(matches!(
        decode_history(99, &[1, 2, 3]).expect("opaque history"),
        LensCorrectionHistoryParameters::Opaque { version: 99, bytes } if bytes == [1, 2, 3]
    ));
    assert!(decode_history(1, &[0, 1, 2]).is_err());
}

#[test]
fn checked_transforms_round_trip_and_roi_enclosure_have_global_origins() {
    let config = LensCorrectionConfig::new(canon_parameters()).expect("config");
    let plan = LensCorrectionPlan::new(dimensions(96, 64), config).expect("plan");
    let source = [17.0, 9.0];
    let output = plan.forward_point(source).expect("forward");
    assert!(points_differ(output, source));
    let restored = plan.back_point(output).expect("inverse");
    assert!((restored[0] - source[0]).abs() < 1.0e-3);
    assert!((restored[1] - source[1]).abs() < 1.0e-3);

    let requested = Roi::new(20, 11, 30, 22).expect("ROI");
    let input = plan.modify_roi_in(requested).expect("input ROI");
    assert!(input.x() <= requested.x());
    assert!(input.y() <= requested.y());
    assert!(input.right() <= 96);
    assert!(input.bottom() <= 64);
    assert_eq!(
        plan.modify_roi_out(requested).expect("output ROI"),
        requested
    );
}

#[test]
fn tca_override_is_channel_specific_and_correction_mode_is_explicit() {
    let mut parameters = sony_parameters();
    parameters.modify_flags = CorrectionFlags::ALL;
    parameters.tca_override = true;
    parameters.tca_red = 1.001;
    parameters.tca_blue = 0.999;
    let plan = LensCorrectionPlan::new(
        dimensions(64, 64),
        LensCorrectionConfig::new(parameters.clone()).expect("config"),
    )
    .expect("plan");
    let point = [60.0, 32.0];
    let red = plan.back_channel_point(point, 0).expect("red");
    let green = plan.back_channel_point(point, 1).expect("green");
    let blue = plan.back_channel_point(point, 2).expect("blue");
    assert!(points_differ(red, green));
    assert!(points_differ(blue, green));

    parameters.mode = LensCorrectionMode::Distort;
    let inverse = LensCorrectionPlan::new(
        dimensions(64, 64),
        LensCorrectionConfig::new(parameters).expect("config"),
    )
    .expect("inverse plan");
    assert!(points_differ(
        inverse.forward_point(point).expect("inverse forward"),
        plan.forward_point(point).expect("correct forward")
    ));
}

#[test]
fn cpu_execution_is_deterministic_preserves_alpha_and_supports_masks() {
    let plan = LensCorrectionPlan::new(
        dimensions(8, 8),
        LensCorrectionConfig::new(canon_parameters()).expect("config"),
    )
    .expect("plan");
    let input = (0..64)
        .map(|index| pixel(f32::from(u16::try_from(index).expect("index")) / 64.0))
        .collect::<Vec<_>>();
    let first = plan.execute(&input).expect("first execution");
    let second = plan.execute(&input).expect("second execution");
    assert_eq!(first, second);
    assert_eq!(first.pixels().len(), input.len());
    assert_eq!(first.dimensions(), dimensions(8, 8));

    let rgba = (0..64)
        .flat_map(|index| {
            [
                f32::from(u16::try_from(index).expect("index")),
                2.0,
                3.0,
                0.5,
            ]
        })
        .collect::<Vec<_>>();
    let output = plan
        .execute_interleaved(&rgba, 4, 32)
        .expect("RGBA execution");
    assert!(
        output
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| (pixel[3] - 0.5).abs() < 1.0e-6)
    );
    let mask = plan.execute_plane(
        &(0..64)
            .map(|value| f32::from(u16::try_from(value).expect("value")))
            .collect::<Vec<_>>(),
        8,
    );
    assert_eq!(mask.expect("mask").len(), 64);
}

#[test]
fn invalid_parameters_and_descriptor_are_rejected_or_validated() {
    let mut parameters = canon_parameters();
    parameters.scale = f32::NAN;
    assert!(LensCorrectionConfig::new(parameters).is_err());
    assert!(LensCorrectionParametersV1::new("", "", 0.0, 8.0).is_err());

    let descriptor = lenscorrection_descriptor();
    descriptor.validate().expect("descriptor");
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::Distortion
    );
    assert!(descriptor.capability.deterministic_cpu);
    assert_eq!(LensCorrectionMethod::Lensfun, LensCorrectionMethod::Lensfun);
}
