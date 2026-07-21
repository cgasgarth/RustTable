#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "deterministic f32 fixtures intentionally exercise the typed raster boundary"
)]

use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use rusttable_processing::descriptor::{OperationFlags, grain_descriptor};
use rusttable_processing::operations::grain::{
    GRAIN_LEGACY_PARAMETER_BYTES, GrainChannel, GrainConfig, GrainHistory, GrainParameterError,
    GrainParametersV1, GrainParametersV2, GrainPlan, grain_hash, hash_to_unit,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite"),
        FiniteF32::new(green).expect("finite"),
        FiniteF32::new(blue).expect("finite"),
    )
}

fn image(dimensions: RasterDimensions) -> Vec<LinearRgb> {
    (0..usize::try_from(dimensions.pixel_count()).expect("small image"))
        .map(|index| {
            let value = index as f32 / 16.0;
            pixel(value, 0.35 + value * 0.2, 0.15)
        })
        .collect()
}

#[test]
fn history_migrates_v1_and_preserves_legacy_payloads() {
    let v1 = GrainParametersV1::new(GrainChannel::Rgb, 8.0, 30.0);
    let history = GrainHistory::decode(1, &v1.to_bytes()).expect("v1 history");
    assert_eq!(
        history
            .migrate_v1()
            .expect("migration")
            .midtones_bias
            .to_bits(),
        0.0_f32.to_bits()
    );
    assert_eq!(history.payload(), v1.to_bytes());

    let mut legacy = vec![0_u8; GRAIN_LEGACY_PARAMETER_BYTES];
    legacy[..12].copy_from_slice(&v1.to_bytes());
    let legacy_history = GrainHistory::decode(1, &legacy).expect("legacy history");
    assert_eq!(legacy_history.payload(), legacy);
    assert!(matches!(
        GrainHistory::decode(9, &[1, 2, 3]),
        Ok(GrainHistory::Opaque { .. })
    ));
}

#[test]
fn v2_round_trip_defaults_and_validation_are_closed() {
    let parameters = GrainParametersV2::defaults();
    let history = GrainHistory::decode(2, &parameters.to_bytes()).expect("v2 history");
    assert_eq!(history.payload(), parameters.to_bytes().to_vec());
    assert!(matches!(
        GrainConfig::new(GrainParametersV2::new(
            GrainChannel::Lightness,
            0.0,
            25.0,
            100.0
        )),
        Err(GrainParameterError::OutOfRange("scale"))
    ));
    assert_eq!(
        hash_to_unit(grain_hash(7, 3, 5, 2, 1)).to_bits(),
        0x3f4c_7966
    );
}

#[test]
fn grain_is_repeatable_and_window_independent_for_every_channel() {
    let dimensions = RasterDimensions::new(7, 5).expect("dimensions");
    let source = image(dimensions);
    for channel in [
        GrainChannel::Hue,
        GrainChannel::Saturation,
        GrainChannel::Lightness,
        GrainChannel::Rgb,
    ] {
        let config = GrainConfig::new(GrainParametersV2::new(channel, 1600.0 / 213.2, 65.0, 100.0))
            .expect("config")
            .with_seed(0x1234_5678_9abc_def0);
        let plan = GrainPlan::new(config, dimensions).expect("plan");
        let full = plan.execute(&source).expect("full");
        let first = plan.execute_window(&source[..14], 0).expect("first");
        let second = plan.execute_window(&source[14..], 14).expect("second");
        assert_eq!(&full[..14], &first);
        assert_eq!(&full[14..], &second);
        assert_eq!(full, plan.execute(&source).expect("repeat"));
    }
}

#[test]
fn strength_zero_is_exact_identity_and_cancellation_publishes_nothing() {
    let dimensions = RasterDimensions::new(4, 4).expect("dimensions");
    let source = vec![pixel(2.0, -1.0, 0.25); 16];
    let config = GrainConfig::new(GrainParametersV2::new(
        GrainChannel::Lightness,
        1600.0 / 213.2,
        0.0,
        100.0,
    ))
    .expect("config");
    let plan = GrainPlan::new(config, dimensions).expect("plan");
    assert_eq!(plan.execute(&source).expect("identity"), source);

    let active = GrainPlan::new(
        GrainConfig::new(GrainParametersV2::defaults())
            .expect("config")
            .with_seed(5),
        dimensions,
    )
    .expect("plan");
    assert!(matches!(
        active.execute_with_cancel(&source, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
}

#[test]
fn descriptor_and_registry_publish_cpu_gpu_contracts() {
    let descriptor = grain_descriptor();
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_GPU));
    assert_eq!(descriptor.capability.gpu_tier, Some(1));
    assert_eq!(descriptor.migration.source_versions, vec![1, 2]);
    let definition = builtin_registry()
        .definition("rusttable.grain")
        .expect("registered grain");
    assert!(definition.cpu().is_some());
    assert_eq!(
        definition.gpu().expect("gpu binding").binding_id(),
        "rusttable.grain.wgsl"
    );
}

#[test]
fn operation_compiler_accepts_channel_enum_as_integer() {
    let operation = Operation::new(
        OperationId::new(42).expect("id"),
        OperationKey::new("rusttable.grain").expect("key"),
        true,
        [
            (
                ParameterName::new("channel").expect("name"),
                ParameterValue::Scalar(FiniteF64::new(3.0).expect("scalar")),
            ),
            (
                ParameterName::new("strength").expect("name"),
                ParameterValue::Scalar(FiniteF64::new(10.0).expect("scalar")),
            ),
        ],
    )
    .expect("operation");
    assert!(matches!(
        builtin_registry()
            .prepare_cpu(&operation)
            .expect("prepared")
            .operation()
            .kind(),
        rusttable_processing::ProcessingOperationKind::Grain { .. }
    ));
}
