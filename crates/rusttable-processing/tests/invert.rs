#![allow(clippy::float_cmp, reason = "tests assert exact compatibility maxima")]

use rusttable_processing::descriptor::{OperationFlags, invert_descriptor};
use rusttable_processing::operations::invert::{
    InvertConfig, InvertHistory, InvertParametersV1, InvertParametersV2, InvertPlan,
    migrate_v1_to_v2,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions, builtin_registry};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

#[test]
fn legacy_v1_migration_copies_rgb_and_keeps_the_historical_sentinel() {
    let migrated = migrate_v1_to_v2(InvertParametersV1::new([0.2, 0.4, 0.8]));
    assert_eq!(migrated.color[..3], [0.2, 0.4, 0.8]);
    assert!(migrated.color[3].is_nan());
    assert!(InvertConfig::from_v2(migrated).is_ok());
    assert_eq!(
        InvertHistory::decode(1, &InvertParametersV1::new([1.0; 3]).to_bytes())
            .unwrap()
            .version(),
        1
    );
    assert_eq!(
        InvertHistory::decode(2, &migrated.to_bytes())
            .unwrap()
            .version(),
        2
    );
}

#[test]
fn unknown_versions_remain_opaque_and_v2_bytes_are_exact() {
    let bytes = [7_u8; 23];
    let history = InvertHistory::decode(99, &bytes).expect("unknown versions are retained");
    assert_eq!(history.payload(), bytes);
    assert_eq!(
        InvertParametersV2::new([0.1, 0.2, 0.3, 0.4])
            .to_bytes()
            .len(),
        16
    );
}

#[test]
fn inversion_multiplies_processed_maximum_then_subtracts_and_clamps() {
    let config = InvertConfig::new([0.5, 0.75, 2.0, 1.0], [2.0, 0.5, 0.25, 1.0]).expect("config");
    let plan = InvertPlan::new(config, RasterDimensions::new(2, 1).unwrap());
    let output = plan
        .execute(&[pixel(0.2, 0.2, 0.9), pixel(-2.0, 0.0, 0.0)])
        .expect("invert");
    assert_eq!(output[0], pixel(0.8, 0.175, 0.0));
    assert_eq!(output[1], pixel(1.0, 0.375, 0.5));
    assert_eq!(InvertPlan::output_processed_maximum(), [1.0; 4]);
}

#[test]
fn registry_exposes_invert_as_deprecated_hidden_rgb_compatibility() {
    let descriptor = invert_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(descriptor.flags.contains(OperationFlags::DEPRECATED));
    assert!(descriptor.flags.contains(OperationFlags::HIDDEN));
    assert_eq!(descriptor.migration.source_versions, [1, 2]);
    let definition = builtin_registry()
        .definition("rusttable.invert")
        .expect("registry");
    assert!(definition.gpu().is_none());
    assert_eq!(definition.descriptor().id.compatibility_name, "invert");
}
