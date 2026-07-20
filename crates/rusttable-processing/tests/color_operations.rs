use rusttable_color::{BuiltinSpace, Primaries, rgb_to_xyz_matrix};
use rusttable_processing::operations::colorin::{
    ColorInConfig, ColorInConfigError, ColorInLegacyParameters, ColorInPlan, ColorInProfile,
    migrate,
};
use rusttable_processing::operations::primaries::{PrimariesConfig, PrimariesPlan};
use rusttable_processing::{FiniteF32, LinearRgb};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "{actual} != {expected}"
    );
}

#[test]
fn chromaticity_matrix_matches_builtin_srgb_evidence() {
    let space = BuiltinSpace::SrgbD65;
    let primaries = space.primaries().expect("sRGB primaries");
    let matrix = rgb_to_xyz_matrix(
        [
            (primaries.red().0.get(), primaries.red().1.get()),
            (primaries.green().0.get(), primaries.green().1.get()),
            (primaries.blue().0.get(), primaries.blue().1.get()),
        ],
        primaries.white(),
    )
    .expect("valid sRGB chromaticities");
    for (actual, expected) in matrix
        .rows()
        .into_iter()
        .zip(space.to_xyz_matrix().unwrap().rows())
    {
        assert_close(actual, expected, 0.000_3);
    }
}

#[test]
fn colorin_uses_linear_working_encoding_and_is_deterministic() {
    let plan = ColorInPlan::new(ColorInConfig::builtin(
        BuiltinSpace::SrgbD65,
        BuiltinSpace::Rec2020D65,
    ))
    .expect("built-in color transform");
    assert!(plan.output_encoding().is_linear());
    let input = [pixel(0.25, 0.5, 0.75)];
    let first = plan.execute(&input).expect("colorin execution");
    let second = plan.execute(&input).expect("colorin execution");
    assert_eq!(first, second);
    assert_ne!(first.receipt().input_digest(), [0; 32]);
    assert_ne!(first.receipt().output_digest(), [0; 32]);
}

#[test]
fn colorin_rejects_missing_profile_evidence_and_supports_historical_defaults() {
    let error = ColorInConfig::new(
        ColorInProfile::Missing("eprofile".to_owned()),
        BuiltinSpace::Rec2020D65.into(),
        rusttable_color::RenderingIntent::Perceptual,
        rusttable_processing::operations::colorin::ColorInNormalization::Off,
        true,
    )
    .expect_err("missing evidence must not fall back");
    assert_eq!(error, ColorInConfigError::MissingProfileEvidence);

    let migrated = migrate(
        1,
        ColorInLegacyParameters {
            input_profile: "srgb".to_owned(),
            working_profile: None,
            intent: 0,
            normalization: 0,
            blue_mapping: None,
        },
    )
    .expect("v1 migration");
    assert!(matches!(
        migrated.working(),
        ColorInProfile::Builtin(BuiltinSpace::Rec2020D65)
    ));
    assert!(migrated.blue_mapping());
}

#[test]
fn primaries_neutral_plan_is_identity_and_receipts_are_stable() {
    let plan = PrimariesPlan::new(PrimariesConfig::defaults(), Primaries::srgb())
        .expect("neutral primaries plan");
    let input = [pixel(0.2, 0.4, 0.8), pixel(1.0, 0.0, 0.5)];
    let output = plan.execute(&input).expect("primaries execution");
    for (actual, expected) in output.pixels().iter().zip(input) {
        assert_close(actual.red().get(), expected.red().get(), 0.000_02);
        assert_close(actual.green().get(), expected.green().get(), 0.000_02);
        assert_close(actual.blue().get(), expected.blue().get(), 0.000_02);
    }
    assert_eq!(output.receipt().plan_identity(), plan.identity());
}

#[test]
fn primaries_custom_rotation_changes_matrix_without_hidden_clipping() {
    let config =
        PrimariesConfig::new(0.0, 0.0, 0.1, 1.0, 0.0, 1.0, 0.0, 1.0).expect("valid rotation");
    let plan = PrimariesPlan::new(config, Primaries::srgb()).expect("rotated plan");
    let output = plan.execute(&[pixel(1.0, 0.0, 0.0)]).expect("execution");
    assert!((output.pixels()[0].red().get() - 1.0).abs() > 0.0001);
    assert!(output.pixels()[0].green().get().is_finite());
}

#[test]
fn color_operations_honor_cancellation() {
    let colorin = ColorInPlan::new(ColorInConfig::default()).expect("colorin plan");
    assert!(
        colorin
            .execute_with_cancel(&[pixel(0.1, 0.2, 0.3)], || true)
            .is_err()
    );
    let primaries =
        PrimariesPlan::new(PrimariesConfig::defaults(), Primaries::srgb()).expect("primaries plan");
    assert!(
        primaries
            .execute_with_cancel(&[pixel(0.1, 0.2, 0.3)], || true)
            .is_err()
    );
}
