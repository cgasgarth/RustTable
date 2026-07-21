#![allow(
    clippy::float_cmp,
    reason = "compatibility tests assert stable scalar values"
)]

use rusttable_processing::operations::relight::{
    RELIGHT_PARAMETER_BYTES, RELIGHT_PRESETS, RelightConfig, RelightHistory, RelightParametersV1,
    RelightPlan,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry, descriptor};

fn pixel(value: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(value).expect("finite red"),
        FiniteF32::new(value).expect("finite green"),
        FiniteF32::new(value).expect("finite blue"),
    )
}

#[test]
fn v1_payload_defaults_presets_and_unknown_history_are_typed() {
    let defaults = RelightParametersV1::defaults();
    assert_eq!(defaults.to_bytes().len(), RELIGHT_PARAMETER_BYTES);
    assert_eq!(
        RelightParametersV1::from_bytes(&defaults.to_bytes()),
        Ok(defaults)
    );
    assert_eq!(
        RELIGHT_PRESETS[0].parameters,
        RelightParametersV1::new(0.25, 0.25, 4.0)
    );
    assert_eq!(
        RELIGHT_PRESETS[1].parameters,
        RelightParametersV1::new(-0.25, 0.25, 4.0)
    );
    let opaque = RelightHistory::decode(8, &[1, 2, 3]).expect("unknown history is retained");
    assert_eq!(opaque.payload(), vec![1, 2, 3]);
    assert_eq!(opaque.version(), 8);
}

#[test]
fn rgb_fill_light_is_deterministic_and_targets_the_selected_tonal_zone() {
    let dimensions = RasterDimensions::new(3, 1).expect("dimensions");
    let input = vec![pixel(0.08), pixel(0.5), pixel(0.92)];
    let plan = RelightPlan::new(
        RelightConfig::new(1.0, 0.08, 2.0).expect("config"),
        dimensions,
    );
    let first = plan.execute(&input).expect("first execution");
    let second = plan.execute(&input).expect("second execution");
    assert_eq!(first, second);
    assert!(first[0].red().get() > input[0].red().get());
    assert_eq!(first[2], input[2]);
}

#[test]
fn descriptor_and_registry_claim_only_deprecated_cpu_rgb_compatibility() {
    let descriptor = descriptor::relight_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(
        descriptor
            .flags
            .contains(descriptor::OperationFlags::DEPRECATED)
    );
    assert!(
        descriptor
            .flags
            .contains(descriptor::OperationFlags::HIDDEN)
    );
    assert_eq!(descriptor.io.input.alpha, descriptor::AlphaPolicy::Preserve);
    let definition = builtin_registry()
        .definition("rusttable.relight")
        .expect("registry");
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
}
