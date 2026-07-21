use rusttable_processing::operations::bloom::{
    BLOOM_PARAMETER_BYTES, BloomConfig, BloomHistory, BloomParametersV1, BloomPlan,
};
use rusttable_processing::operations::convolution::BoxKernel;
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
    let parameters = BloomParametersV1::defaults();
    assert_eq!(parameters.to_bytes().len(), BLOOM_PARAMETER_BYTES);
    assert_eq!(
        BloomParametersV1::from_bytes(&parameters.to_bytes()),
        Ok(parameters)
    );
    assert_eq!(
        BloomHistory::decode(1, &parameters.to_bytes()).expect("v1 history"),
        BloomHistory::V1(parameters)
    );
    assert_eq!(
        BloomHistory::decode(9, &[1, 2, 3]).expect("future history"),
        BloomHistory::Opaque {
            version: 9,
            bytes: vec![1, 2, 3]
        }
    );
}

#[test]
fn descriptor_registry_and_validation_match_the_backend_contract() {
    let descriptor = descriptor::bloom_descriptor();
    descriptor.validate().expect("bloom descriptor");
    assert_eq!(descriptor.id.compatibility_name, "bloom");
    assert_eq!(descriptor.id.parameter_version, 1);
    assert_eq!(
        descriptor.roi,
        rusttable_processing::descriptor::RoiKind::FullImage
    );
    assert_eq!(
        descriptor.io.output.alpha,
        rusttable_processing::descriptor::AlphaPolicy::Preserve
    );
    assert!(builtin_registry().definition("rusttable.bloom").is_some());
    assert!(BloomConfig::new(-1.0, 90.0, 25.0).is_err());
}

#[test]
fn box_blur_clamps_edges_without_changing_constant_fields() {
    let input = [1.0; 5];
    let output = BoxKernel::new(2)
        .apply_scalar(
            &input,
            dimensions(5, 1),
            rusttable_processing::operations::ReconstructionBudget::default(),
        )
        .expect("blur");
    assert_eq!(output, input);
}

#[test]
fn extraction_is_frozen_and_execution_is_deterministic() {
    let config = BloomConfig::new(0.0, 0.0, 25.0).expect("config");
    let plan = BloomPlan::new(config, dimensions(9, 1)).expect("plan");
    assert_eq!(plan.radius(), 3);
    let mut input = vec![pixel(0.0, 0.0, 0.0); 9];
    input[4] = pixel(1.0, 1.0, 1.0);
    let first = plan.execute(&input, dimensions(9, 1)).expect("first");
    let second = plan.execute(&input, dimensions(9, 1)).expect("second");
    assert_eq!(first, second);
    assert!(first[0].red().get() > 0.0);
    assert!(first[4].red().get() <= 1.0);
}
