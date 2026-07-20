use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterText, ParameterValue,
};
use rusttable_image::{
    BlackWhiteLevels, CfaPattern, CfaPhase, ImageDimensions, Orientation, RawMosaic,
};
use rusttable_processing::operations::temperature::{
    ChannelMultipliers, TemperatureConfig, TemperatureConfigError, TemperatureLegacyParametersV2,
    TemperatureLegacyParametersV3, TemperatureLegacyParametersV4, TemperaturePlan,
    TemperaturePlanError, WhiteBalanceSource, WhiteBalanceStage, migrate_v2, migrate_v3,
    migrate_v4, multipliers_to_temperature_tint, temperature_tint_to_multipliers,
};
use rusttable_processing::{RawPrepareConfig, RawPreparePlan, temperature_descriptor};

fn scalar(value: f64) -> ParameterValue {
    ParameterValue::Scalar(FiniteF64::new(value).expect("finite scalar"))
}

fn text(value: &str) -> ParameterValue {
    ParameterValue::Text(ParameterText::try_from(value).expect("parameter text"))
}

fn operation(parameters: Vec<(&str, ParameterValue)>) -> Operation {
    Operation::new(
        OperationId::new(304).expect("operation id"),
        OperationKey::new("rusttable.temperature").expect("operation key"),
        true,
        parameters
            .into_iter()
            .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    )
    .expect("operation")
}

fn pixel(red: f32, green: f32, blue: f32) -> rusttable_processing::LinearRgb {
    rusttable_processing::LinearRgb::new(
        rusttable_processing::FiniteF32::new(red).expect("red"),
        rusttable_processing::FiniteF32::new(green).expect("green"),
        rusttable_processing::FiniteF32::new(blue).expect("blue"),
    )
}

#[test]
fn descriptor_and_registry_identity_are_temperature_compatible() {
    let descriptor = temperature_descriptor();
    descriptor.validate().expect("temperature descriptor");
    assert_eq!(descriptor.id.compatibility_name, "temperature");
    assert_eq!(descriptor.id.rust_id, "rusttable.temperature");
    assert_eq!(descriptor.id.parameter_version, 4);
    assert!(
        rusttable_processing::builtin_registry()
            .definition("rusttable.temperature")
            .is_some()
    );
}

#[test]
fn coefficients_are_positive_bounded_and_green_normalized() {
    let normalized =
        ChannelMultipliers::from_coefficients([2.0, 4.0, 1.0, 4.0]).expect("valid coefficients");
    for (actual, expected) in normalized
        .as_array()
        .map(rusttable_processing::FiniteF32::get)
        .into_iter()
        .zip([0.5, 1.0, 0.25, 1.0])
    {
        assert!((actual - expected).abs() < f32::EPSILON);
    }
    assert!(ChannelMultipliers::from_coefficients([0.0, 1.0, 1.0, 1.0]).is_err());
    assert!(ChannelMultipliers::new([9.0, 1.0, 1.0, 1.0]).is_err());
    assert!(ChannelMultipliers::new([2.0, 0.5, 1.0, 1.0]).is_err());
}

#[test]
fn darktable_parameter_migrations_preserve_coefficients_and_defaults() {
    let v2 = migrate_v2(TemperatureLegacyParametersV2 {
        temp_out: 6500.0,
        coefficients: [2.0, 1.0, 1.5],
    });
    assert_eq!(
        (v2.red, v2.green, v2.blue, v2.various, v2.preset),
        (2.0, 1.0, 1.5, 1.0, -1)
    );
    let v3 = migrate_v3(TemperatureLegacyParametersV3 {
        red: 2.0,
        green: 1.0,
        blue: 1.5,
        various: f32::NAN,
    });
    assert!((v3.various - 1.0).abs() < f32::EPSILON);
    let v4 = migrate_v4(TemperatureLegacyParametersV4 { preset: 3, ..v3 });
    assert_eq!(v4.preset, 3);
}

#[test]
fn temperature_tint_conversion_round_trips_within_f32_contract() {
    for (temperature, tint) in [(2200.0, 1.0), (4000.0, 1.0), (6500.0, 1.1), (25000.0, 1.0)] {
        let multipliers = temperature_tint_to_multipliers(temperature, tint)
            .expect("temperature/tint conversion");
        let round_trip = multipliers_to_temperature_tint(multipliers)
            .expect("inverse temperature/tint conversion");
        assert!((round_trip.temperature_kelvin().get() - temperature).abs() < 2.0);
        assert!((round_trip.tint().get() - tint).abs() < 0.02);
    }
}

#[test]
fn temperature_plan_scales_rgb_without_clipping_and_has_stable_receipt() {
    let config = TemperatureConfig::new(
        ChannelMultipliers::new([2.0, 1.0, 0.5, 1.0]).expect("multipliers"),
        WhiteBalanceSource::Custom,
    )
    .expect("config");
    let plan = TemperaturePlan::new(config.clone()).expect("plan");
    let output = plan.execute(&[pixel(-1.0, 2.0, 0.75)]).expect("execution");
    assert_eq!(output.pixels()[0], pixel(-2.0, 2.0, 0.375));
    assert_eq!(output.receipt().multipliers(), config.multipliers());
    assert_eq!(output.receipt().identity(), plan.receipt().identity());
    assert!(
        plan.execute_with_cancel(&[pixel(0.1, 0.2, 0.3)], || true)
            .is_err()
    );
}

#[test]
fn raw_execution_uses_cfa_color_and_phase_instead_of_array_position() {
    let pattern = CfaPattern::bayer_rggb();
    let raw = RawMosaic::new(
        ImageDimensions::new(2, 2).expect("dimensions"),
        2,
        vec![100, 200, 300, 400],
        pattern,
        CfaPhase::new(0, 0, pattern),
        BlackWhiteLevels::new(0, 400).expect("levels"),
        Orientation::Normal,
    )
    .expect("raw");
    let normalized = RawPreparePlan::new(&raw, RawPrepareConfig::default())
        .expect("prepare plan")
        .execute(&raw)
        .expect("normalized raw");
    let config = TemperatureConfig::with_details(
        ChannelMultipliers::new([2.0, 1.0, 0.5, 1.5]).expect("multipliers"),
        WhiteBalanceSource::AsShot,
        WhiteBalanceStage::PreDemosaic,
        None,
        None,
    )
    .expect("config");
    let output = TemperaturePlan::new(config)
        .expect("plan")
        .execute_raw(&normalized)
        .expect("raw execution");
    let values: Vec<_> = output.samples().iter().map(|value| value.get()).collect();
    assert_eq!(values, vec![0.5, 0.5, 0.75, 0.5]);
    assert_eq!(output.cfa(), normalized.cfa());
}

#[test]
fn operation_compilation_consumes_resolved_coefficients_and_blocks_unresolved_presets() {
    let resolved = operation(vec![
        ("red", scalar(2.0)),
        ("green", scalar(1.0)),
        ("blue", scalar(0.5)),
        ("various", scalar(1.0)),
        ("source", text("custom")),
    ]);
    let prepared = rusttable_processing::builtin_registry()
        .prepare_cpu(&resolved)
        .expect("temperature factory");
    assert!(matches!(
        prepared.operation().kind(),
        rusttable_processing::ProcessingOperationKind::Temperature { .. }
    ));

    let unresolved = operation(vec![
        ("red", scalar(2.0)),
        ("green", scalar(1.0)),
        ("blue", scalar(0.5)),
        ("preset", scalar(17.0)),
    ]);
    let error = rusttable_processing::builtin_registry()
        .prepare_cpu(&unresolved)
        .expect_err("unresolved named preset must block");
    assert!(error.to_string().contains("immutable preset provenance"));
}

#[test]
fn temperature_config_rejects_preset_without_immutable_evidence() {
    let error = TemperatureConfig::new(
        ChannelMultipliers::new([1.0; 4]).expect("multipliers"),
        WhiteBalanceSource::Preset,
    )
    .expect_err("preset evidence");
    assert_eq!(error, TemperatureConfigError::MissingPresetProvenance);
    assert!(!matches!(
        error,
        TemperatureConfigError::UnexpectedTemperatureTint
    ));
    let _ = TemperaturePlanError::Config(error);
}
