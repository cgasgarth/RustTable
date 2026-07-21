#![allow(
    clippy::float_cmp,
    reason = "compatibility tests assert stable scalar values"
)]

use rusttable_processing::descriptor::OperationFlags;
use rusttable_processing::operations::shadhi::{
    SHADHI_V1_PARAMETER_BYTES, SHADHI_V5_PARAMETER_BYTES, ShadhiAlgorithm, ShadhiConfig,
    ShadhiHistory, ShadhiParametersV1, ShadhiParametersV5, ShadhiPlan, migrate_v1_to_v5,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry, descriptor};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

fn gaussian_config() -> ShadhiConfig {
    ShadhiConfig::new(ShadhiParametersV5 {
        shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
        radius: 10.0,
        ..ShadhiParametersV5::defaults()
    })
    .expect("Gaussian config")
}

#[test]
fn typed_legacy_layouts_migrate_and_unknown_payloads_round_trip() {
    let old = ShadhiParametersV1 {
        order: 0,
        radius: -100.0,
        shadows: 40.0,
        reserved1: 2.0,
        highlights: 20.0,
        reserved2: 0.0,
        compress: 50.0,
    };
    let migrated = migrate_v1_to_v5(old);
    assert_eq!(migrated.radius, 100.0);
    assert_eq!(migrated.shadows, 20.0);
    assert_eq!(migrated.highlights, -10.0);
    assert_eq!(migrated.shadhi_algo, ShadhiAlgorithm::Bilateral.id());
    assert_eq!(
        old.radius.to_le_bytes().len() + 24,
        SHADHI_V1_PARAMETER_BYTES
    );
    let current = ShadhiParametersV5::defaults();
    assert_eq!(current.to_bytes().len(), SHADHI_V5_PARAMETER_BYTES);
    assert_eq!(
        ShadhiHistory::decode(5, &current.to_bytes())
            .expect("v5 history")
            .payload(),
        current.to_bytes().to_vec()
    );
    let opaque = ShadhiHistory::decode(99, &[7, 8, 9]).expect("unknown history is retained");
    assert_eq!(opaque.payload(), vec![7, 8, 9]);
    assert_eq!(opaque.version(), 99);
}

#[test]
fn gaussian_rgb_plan_is_deterministic_and_bilateral_fails_closed() {
    let dimensions = RasterDimensions::new(3, 3).expect("dimensions");
    let input = vec![
        pixel(0.02, 0.04, 0.08),
        pixel(0.2, 0.3, 0.4),
        pixel(0.8, 0.7, 0.6),
        pixel(0.1, 0.2, 0.3),
        pixel(0.4, 0.5, 0.6),
        pixel(0.9, 0.8, 0.7),
        pixel(0.05, 0.06, 0.07),
        pixel(0.3, 0.2, 0.1),
        pixel(0.95, 0.9, 0.85),
    ];
    let plan = ShadhiPlan::new(gaussian_config(), dimensions).expect("Gaussian plan");
    let first = plan.execute(&input).expect("first execution");
    assert_eq!(first, plan.execute(&input).expect("second execution"));
    assert_ne!(first, input);

    let error = ShadhiPlan::new(ShadhiConfig::defaults(), dimensions)
        .expect_err("default bilateral mode must fail closed");
    assert!(error.to_string().contains("Lab/bilateral"));
}

#[test]
fn descriptor_and_registry_are_cpu_only_rgb_and_alpha_preserving() {
    let descriptor = descriptor::shadhi_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(descriptor.flags.contains(OperationFlags::FULL_IMAGE));
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_CPU));
    assert_eq!(descriptor.io.input.alpha, descriptor::AlphaPolicy::Preserve);
    assert_eq!(descriptor.migration.source_versions, [1, 2, 3, 4, 5]);
    let definition = builtin_registry()
        .definition("rusttable.shadhi")
        .expect("registry");
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
}
