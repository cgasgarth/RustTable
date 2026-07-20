use rusttable_processing::operations::ReconstructionBudget;
use rusttable_processing::operations::colorreconstruction::{
    ColorReconstructionConfig, ColorReconstructionPlan, ColorReconstructionPrecedence,
    ColorReconstructionV1, ColorReconstructionV2, ColorReconstructionV3,
};
use rusttable_processing::operations::highlights::{
    HighlightsConfig, HighlightsInputClass, HighlightsMethod, HighlightsPlan, HighlightsV1,
    HighlightsV2, HighlightsV3, HighlightsV4, RecoveryMode, WaveletScale,
};
use rusttable_processing::{FiniteF32, LinearRgb, RasterDimensions};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

fn dimensions() -> RasterDimensions {
    RasterDimensions::new(5, 3).expect("dimensions")
}

fn highlights_config(method: HighlightsMethod) -> HighlightsConfig {
    HighlightsConfig::new(
        method,
        1.0,
        1.0,
        0.0,
        4,
        WaveletScale::new(1).expect("scale"),
        0.4,
        2.0,
        RecoveryMode::Off,
        0.0,
    )
    .expect("valid highlights config")
}

#[test]
fn highlights_migrations_preserve_method_and_historical_defaults() {
    let v1 = HighlightsV1 {
        method: 1,
        blend_l: 0.7,
        blend_c: 0.2,
        strength: 0.9,
    };
    let v2 = HighlightsV2 {
        method: 2,
        blend_l: 0.7,
        blend_c: 0.2,
        strength: 0.9,
        clip: 0.8,
    };
    let v3 = HighlightsV3 {
        method: 3,
        blend_l: 0.7,
        blend_c: 0.2,
        strength: 0.9,
        clip: 0.8,
        noise_level: 0.1,
        iterations: 7,
        scales: 4,
        candidating: 0.2,
        combine: 3.0,
        recovery: 5,
    };
    let migrated_v1 = rusttable_processing::operations::highlights::migrate_v1(v1).expect("v1");
    let migrated_v2 = rusttable_processing::operations::highlights::migrate_v2(v2).expect("v2");
    let migrated_v3 = rusttable_processing::operations::highlights::migrate_v3(v3).expect("v3");
    assert_eq!(migrated_v1.method, 1);
    assert_eq!(migrated_v1.iterations, 1);
    assert_eq!(migrated_v1.scales, 5);
    assert_eq!(migrated_v2.clip.to_bits(), 0.8_f32.to_bits());
    assert_eq!(migrated_v3.recovery, 5);
    assert_eq!(
        HighlightsV4 {
            solid_color: 0.0,
            ..migrated_v3
        }
        .config()
        .expect("config")
        .method(),
        HighlightsMethod::GuidedLaplacians
    );
}

#[test]
fn all_highlight_methods_have_stable_ids_and_execute_real_reconstruction() {
    let mut input = vec![pixel(0.25, 0.35, 0.45); 15];
    input[7] = pixel(1.4, 0.1, 0.2);
    for method in [
        HighlightsMethod::Clip,
        HighlightsMethod::ReconstructLCh,
        HighlightsMethod::ReconstructColor,
        HighlightsMethod::GuidedLaplacians,
        HighlightsMethod::SegmentationBased,
        HighlightsMethod::InpaintOpposed,
    ] {
        let plan = HighlightsPlan::new(
            highlights_config(method),
            dimensions(),
            HighlightsInputClass::Rgb,
            ReconstructionBudget::default(),
        )
        .expect("plan");
        let first = plan.execute(&input).expect("method executes");
        let second = plan.execute(&input).expect("method repeats");
        assert_eq!(first.receipt(), second.receipt());
        assert_eq!(first.pixels(), second.pixels());
        assert!(first.diagnostics().affected()[7]);
        assert!(first.pixels().iter().all(|pixel| {
            pixel.red().get().is_finite()
                && pixel.green().get().is_finite()
                && pixel.blue().get().is_finite()
        }));
        input[7] = pixel(1.4, 0.1, 0.2);
    }
}

#[test]
fn highlights_cancellation_and_memory_limits_publish_nothing() {
    let plan = HighlightsPlan::new(
        highlights_config(HighlightsMethod::GuidedLaplacians),
        dimensions(),
        HighlightsInputClass::Rgb,
        ReconstructionBudget::new(1),
    );
    assert!(matches!(
        plan,
        Err(rusttable_processing::operations::OperationExecutionError::MemoryBudgetExceeded { .. })
    ));

    let plan = HighlightsPlan::new(
        highlights_config(HighlightsMethod::ReconstructColor),
        dimensions(),
        HighlightsInputClass::Rgb,
        ReconstructionBudget::default(),
    )
    .expect("plan");
    let input = vec![pixel(0.25, 0.35, 0.45); 15];
    assert_eq!(
        plan.execute_with_cancel(&input, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    );
}

#[test]
fn color_reconstruction_migrates_versions_and_preserves_luminance() {
    let v1 = ColorReconstructionV1 {
        threshold: 90.0,
        spatial: 12.0,
        range: 4.0,
    };
    let v2 = ColorReconstructionV2 {
        threshold: 90.0,
        spatial: 12.0,
        range: 4.0,
        precedence: 1,
    };
    let v3 = ColorReconstructionV3 {
        threshold: 90.0,
        spatial: 12.0,
        range: 4.0,
        hue: 0.5,
        precedence: 2,
    };
    assert_eq!(
        rusttable_processing::operations::colorreconstruction::migrate_v1(v1)
            .hue
            .to_bits(),
        0.66_f32.to_bits()
    );
    assert_eq!(
        rusttable_processing::operations::colorreconstruction::migrate_v2(v2).precedence,
        1
    );
    let config = v3.config().expect("v3 config");

    let input = [
        pixel(0.1, 0.4, 0.1),
        pixel(1.4, 1.2, 1.2),
        pixel(0.1, 0.1, 0.5),
    ];
    let plan = ColorReconstructionPlan::new(
        config,
        RasterDimensions::new(3, 1).expect("dimensions"),
        ReconstructionBudget::default(),
    )
    .expect("plan");
    let result = plan.execute(&input).expect("reconstruction");
    assert!(result.diagnostics().affected()[1]);
    let before = 0.2126 * input[1].red().get()
        + 0.7152 * input[1].green().get()
        + 0.0722 * input[1].blue().get();
    let after = 0.2126 * result.pixels()[1].red().get()
        + 0.7152 * result.pixels()[1].green().get()
        + 0.0722 * result.pixels()[1].blue().get();
    assert!((before - after).abs() < 0.000_01);
    assert_eq!(
        result.receipt(),
        &plan.execute(&input).expect("repeat").receipt().clone()
    );
}

#[test]
fn reconstruction_configs_reject_unknown_enums_without_substitution() {
    assert!(HighlightsMethod::from_id(99).is_err());
    assert!(RecoveryMode::from_id(99).is_err());
    assert!(ColorReconstructionPrecedence::from_id(99).is_err());
    assert!(WaveletScale::new(12).is_err());
    assert!(
        ColorReconstructionConfig::new(
            100.0,
            400.0,
            10.0,
            0.66,
            ColorReconstructionPrecedence::None
        )
        .is_ok()
    );
}
