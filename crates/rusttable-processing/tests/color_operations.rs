use rusttable_color::{BuiltinSpace, Primaries, rgb_to_xyz_matrix};
use rusttable_processing::operations::colorcorrection::{
    ColorCorrectionConfig, ColorCorrectionMode, ColorCorrectionPlan,
};
use rusttable_processing::operations::colorin::{
    ColorInConfig, ColorInConfigError, ColorInLegacyParameters, ColorInPlan, ColorInProfile,
    migrate,
};
use rusttable_processing::operations::colorout::{
    ColorOutConfig, ColorOutGamutMode, ColorOutPlan, ColorOutProfile,
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
    assert_eq!(
        plan.output_frame().encoding(),
        rusttable_color::ColorEncoding::LinearRec2020D65
    );
    assert_eq!(
        plan.output_frame().provenance(),
        rusttable_processing::WorkingProfileProvenance::Selected
    );
}

#[test]
fn colorin_invalid_working_evidence_uses_explicit_rec2020_fallback() {
    let config = ColorInConfig::new(
        BuiltinSpace::SrgbD65.into(),
        ColorInProfile::Missing("invalid-working.icc".to_owned()),
        rusttable_color::RenderingIntent::Perceptual,
        rusttable_processing::operations::colorin::ColorInNormalization::Off,
        true,
    )
    .expect("input evidence is valid");
    let plan = ColorInPlan::new(config).expect("fallback plan");
    assert_eq!(
        plan.output_frame().encoding(),
        rusttable_color::ColorEncoding::LinearRec2020D65
    );
    assert_eq!(
        plan.output_frame().provenance(),
        rusttable_processing::WorkingProfileProvenance::FallbackRec2020
    );
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

#[test]
fn colorout_applies_transfer_and_preserves_deterministic_gpu_parity() {
    let plan = ColorOutPlan::new(ColorOutConfig::builtin(BuiltinSpace::SrgbD65))
        .expect("sRGB output plan");
    let input = [pixel(0.25, 0.5, 0.75)];
    let cpu = plan.execute(&input).expect("CPU output");
    let gpu = plan.execute_wgpu(&input, || false).expect("WGPU output");
    assert_eq!(cpu, gpu);
    assert!(cpu.pixels()[0].red().get() > input[0].red().get());
    assert_eq!(
        plan.executor(),
        rusttable_processing::operations::colorout::ColorOutExecutor::WgpuMatrix
    );
    assert_eq!(
        cpu.terminal().descriptor().encoding(),
        rusttable_color::ColorEncoding::SrgbD65
    );
    assert_eq!(
        cpu.terminal().descriptor().transfer(),
        rusttable_color::TransferFunction::Srgb
    );
    let rec2020_plan = ColorOutPlan::new_with_working_frame(
        ColorOutConfig::builtin(BuiltinSpace::SrgbD65),
        rusttable_processing::WorkingFrameDescriptor::rec2020(),
    )
    .expect("working-profile-aware output plan");
    assert_eq!(
        rec2020_plan.terminal_descriptor().source_frame(),
        rusttable_processing::WorkingFrameDescriptor::rec2020()
    );
    assert_ne!(cpu.receipt().output_digest(), [0; 32]);
}

#[test]
fn colorout_rejects_missing_profiles_and_publishes_gamut_diagnostics() {
    let missing = ColorOutConfig::new(
        ColorOutProfile::Missing("profile.icc".to_owned()),
        rusttable_color::RenderingIntent::Relative,
        rusttable_color::BlackPointCompensation::Disabled,
        None,
        ColorOutGamutMode::Warning,
    );
    assert!(missing.is_err(), "missing profile evidence must block");
    let plan = ColorOutPlan::new(
        ColorOutConfig::new(
            BuiltinSpace::SrgbD65.into(),
            rusttable_color::RenderingIntent::Relative,
            rusttable_color::BlackPointCompensation::Disabled,
            None,
            ColorOutGamutMode::Warning,
        )
        .expect("valid warning config"),
    )
    .expect("warning plan");
    let output = plan
        .execute(&[pixel(2.0, 0.0, 0.0)])
        .expect("diagnostic output");
    assert!(output.gamut_mask()[0]);
}

#[test]
fn colorcorrection_neutral_is_identity_and_axis_mode_is_cancelable() {
    let neutral =
        ColorCorrectionPlan::new(ColorCorrectionConfig::defaults()).expect("neutral plan");
    let input = [pixel(0.2, 0.4, 0.8), pixel(-0.1, 1.5, 0.3)];
    let output = neutral.execute(&input).expect("neutral execution");
    for (actual, expected) in output.pixels().iter().zip(input) {
        assert_close(actual.red().get(), expected.red().get(), 0.000_01);
        assert_close(actual.green().get(), expected.green().get(), 0.000_01);
        assert_close(actual.blue().get(), expected.blue().get(), 0.000_01);
    }
    let config = ColorCorrectionConfig::new(
        [0.0, -0.15, 0.1],
        [0.0, 0.2, -0.1],
        1.15,
        0.6,
        0.0,
        ColorCorrectionMode::Axis,
    )
    .expect("axis config");
    let plan = ColorCorrectionPlan::new(config).expect("axis plan");
    assert!(plan.execute_with_cancel(&input, || true).is_err());
    let cpu = plan.execute(&input).expect("axis CPU");
    let gpu = plan.execute_wgpu(&input, || false).expect("axis WGPU");
    assert_eq!(cpu, gpu);
    assert_ne!(cpu.receipt().plan_identity(), [0; 32]);
}
