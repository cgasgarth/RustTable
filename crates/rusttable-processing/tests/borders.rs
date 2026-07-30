use rusttable_image::Roi;
use rusttable_processing::{
    BordersAspect, BordersBasis, BordersConfig, BordersHistory, BordersOrientation,
    BordersParametersV4, BordersPlan, FiniteF32, LinearRgb, RasterDimensions,
    decode_borders_history, migrate_borders_history,
};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(value).expect("red"),
        FiniteF32::new(value).expect("green"),
        FiniteF32::new(value).expect("blue"),
    )
}

#[test]
fn v4_parameters_round_trip_and_unknown_history_stays_opaque() {
    let parameters = BordersParametersV4::new(BordersConfig::defaults());
    let bytes = parameters.to_bytes();
    assert_eq!(bytes.len(), 120);
    assert_eq!(
        BordersParametersV4::from_bytes(&bytes).expect("decode"),
        parameters
    );
    assert!(matches!(
        decode_borders_history(4, &bytes).expect("history"),
        BordersHistory::V4(value) if value == parameters
    ));
    assert!(matches!(
        decode_borders_history(99, &[1, 2, 3]).expect("opaque"),
        BordersHistory::Opaque { version: 99, bytes } if bytes == [1, 2, 3]
    ));
}

#[test]
fn v1_migration_preserves_color_and_normalizes_orientation() {
    let legacy = rusttable_processing::BordersParametersV1::new([0.2, 0.3, 0.4], 0.5, 0.2)
        .expect("legacy parameters");
    let migrated = migrate_borders_history(BordersHistory::V1(legacy)).expect("migration");
    assert!(
        migrated
            .config
            .color
            .floats()
            .into_iter()
            .zip([0.2, 0.3, 0.4])
            .all(|(actual, expected)| (actual - expected).abs() < 1.0e-6)
    );
    assert_eq!(
        migrated.config.aspect,
        BordersAspect::custom(2.0).expect("aspect")
    );
    assert_eq!(migrated.config.orientation, BordersOrientation::Portrait);
}

#[test]
fn geometry_execution_and_roi_mapping_are_deterministic() {
    let config = BordersConfig::new(
        [1.0, 0.0, 0.0],
        BordersAspect::Constant,
        BordersOrientation::Auto,
        0.5,
        0.5,
        0.5,
        0.2,
        0.5,
        [0.0, 1.0, 0.0],
        true,
        BordersBasis::Auto,
    )
    .expect("config");
    let plan = BordersPlan::new(config, dimensions(2, 2)).expect("plan");
    assert_eq!(plan.output_dimensions(), dimensions(4, 4));
    assert_eq!(
        plan.modify_roi_out(Roi::new(0, 0, 2, 2).expect("source ROI"))
            .expect("output ROI"),
        Roi::new(1, 1, 2, 2).expect("output ROI")
    );
    assert_eq!(
        plan.modify_roi_in(Roi::new(0, 0, 1, 1).expect("border ROI"))
            .expect("input ROI"),
        None
    );
    let input = vec![pixel(0.25); 4];
    let first = plan.execute(&input).expect("execution");
    assert_eq!(first, plan.execute(&input).expect("repeat execution"));
    assert_eq!(first.pixels()[5], input[0]);
    assert!((first.pixels()[0].red().get() - 1.0).abs() < 1.0e-6);
    assert!(matches!(
        plan.execute_with_cancel(&input, || true),
        Err(rusttable_processing::BordersExecutionError::Cancelled)
    ));
}
