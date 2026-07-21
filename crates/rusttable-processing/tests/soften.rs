use rusttable_processing::operations::soften::{
    SOFTEN_PARAMETER_BYTES, SoftenConfig, SoftenHistory, SoftenParametersV1, SoftenPlan,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry, descriptor};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("dimensions")
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("red"),
        FiniteF32::new(green).expect("green"),
        FiniteF32::new(blue).expect("blue"),
    )
}

#[test]
fn v1_payload_defaults_and_unknown_history_are_typed() {
    let parameters = SoftenParametersV1::defaults();
    assert_eq!(parameters.to_bytes().len(), SOFTEN_PARAMETER_BYTES);
    assert_eq!(
        SoftenParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert_eq!(
        SoftenHistory::decode(1, &parameters.to_bytes()).expect("v1 history"),
        SoftenHistory::V1(parameters)
    );
    assert_eq!(
        SoftenHistory::decode(9, &[4, 5]).expect("future history"),
        SoftenHistory::Opaque {
            version: 9,
            bytes: vec![4, 5]
        }
    );
}

#[test]
fn descriptor_registry_and_validation_match_the_backend_contract() {
    let descriptor = descriptor::soften_descriptor();
    descriptor.validate().expect("soften descriptor");
    assert_eq!(descriptor.id.compatibility_name, "soften");
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::FullImage
    );
    assert_eq!(
        descriptor.io.output.alpha,
        rusttable_processing::descriptor::AlphaPolicy::Preserve
    );
    assert!(builtin_registry().definition("rusttable.soften").is_some());
    assert!(SoftenConfig::new(50.0, 100.0, 3.0, 50.0).is_err());
}

#[test]
fn zero_mix_is_exact_pass_through_and_adjustment_is_source_immutable() {
    let dimensions = dimensions(4, 4);
    let input = vec![pixel(0.2, 0.4, 0.8); 16];
    let identity = SoftenPlan::new(
        SoftenConfig::new(50.0, 100.0, 0.33, 0.0).expect("identity config"),
        dimensions,
    )
    .expect("identity plan")
    .execute(&input, dimensions)
    .expect("identity execution");
    assert_eq!(identity, input);

    let plan = SoftenPlan::new(
        SoftenConfig::new(0.0, 0.0, 1.0, 100.0).expect("adjust config"),
        dimensions,
    )
    .expect("adjust plan");
    let first = plan.execute(&input, dimensions).expect("first");
    let second = plan.execute(&input, dimensions).expect("second");
    assert_eq!(first, second);
    assert_ne!(first, input);
}
