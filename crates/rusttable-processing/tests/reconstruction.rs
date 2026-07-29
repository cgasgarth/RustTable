use rusttable_color::ColorEncoding;
use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
    PhotoId, Revision,
};
use rusttable_processing::descriptor::{
    OperationFlags, color_reconstruction_descriptor, highlights_descriptor,
};
use rusttable_processing::operations::ReconstructionBudget;
use rusttable_processing::operations::colorreconstruction::{
    BILATERAL_MAX_RESOLUTION_RANGE, BILATERAL_MAX_RESOLUTION_SPATIAL,
    COLORRECONSTRUCTION_COMPATIBILITY_ID, ColorReconstructionConfig, ColorReconstructionPlan,
    ColorReconstructionPrecedence, ColorReconstructionV1, ColorReconstructionV2,
    ColorReconstructionV3, grid_rescale, hue_conversion,
    migrate_v1 as migrate_colorreconstruction_v1, migrate_v2 as migrate_colorreconstruction_v2,
};
use rusttable_processing::operations::highlights::{
    HighlightsConfig, HighlightsInputClass, HighlightsMethod, HighlightsPlan, HighlightsV1,
    HighlightsV2, HighlightsV3, HighlightsV4, RecoveryMode, WaveletScale,
};
use rusttable_processing::{
    CompiledPipeline, FiniteF32, LinearRgb, RasterDimensions, WorkingRgbImage, evaluate,
};

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
fn color_reconstruction_migrations_match_native_v1_and_v2_layouts() {
    let v1 = ColorReconstructionV1 {
        threshold: 90.0,
        spatial: 12.0,
        range: 4.0,
    };
    let v2 = ColorReconstructionV2 {
        threshold: 91.0,
        spatial: 13.0,
        range: 5.0,
        precedence: ColorReconstructionPrecedence::Chroma.id(),
    };
    let migrated_v1 = migrate_colorreconstruction_v1(v1);
    assert_eq!(migrated_v1.threshold.to_bits(), 90.0_f32.to_bits());
    assert_eq!(migrated_v1.spatial.to_bits(), 12.0_f32.to_bits());
    assert_eq!(migrated_v1.range.to_bits(), 4.0_f32.to_bits());
    assert_eq!(migrated_v1.hue.to_bits(), 0.66_f32.to_bits());
    assert_eq!(
        migrated_v1.precedence,
        ColorReconstructionPrecedence::None.id()
    );

    let migrated_v2 = migrate_colorreconstruction_v2(v2);
    assert_eq!(migrated_v2.threshold.to_bits(), 91.0_f32.to_bits());
    assert_eq!(migrated_v2.spatial.to_bits(), 13.0_f32.to_bits());
    assert_eq!(migrated_v2.range.to_bits(), 5.0_f32.to_bits());
    assert_eq!(migrated_v2.hue.to_bits(), 0.66_f32.to_bits());
    assert_eq!(
        migrated_v2.precedence,
        ColorReconstructionPrecedence::Chroma.id()
    );
}

#[test]
fn color_reconstruction_grid_geometry_matches_native_clamps_and_sigmas() {
    let config = ColorReconstructionConfig::new(
        100.0,
        400.0,
        10.0,
        0.66,
        ColorReconstructionPrecedence::None,
    )
    .expect("config");
    let plan = ColorReconstructionPlan::new(
        config,
        RasterDimensions::new(1_000, 500).expect("dimensions"),
        ReconstructionBudget::default(),
    )
    .expect("plan");
    let geometry = plan.geometry();
    assert_eq!(
        (geometry.size_x(), geometry.size_y(), geometry.size_z()),
        (5, 5, 11)
    );
    assert_eq!(geometry.sigma_s().to_bits(), 250.0_f32.to_bits());
    assert_eq!(geometry.sigma_r().to_bits(), 10.0_f32.to_bits());
    assert_eq!(
        geometry.native_memory_estimate_bytes(),
        5 * 5 * 11 * 4 * 4 * 2
    );
    assert!(plan.full_image_analysis());
    assert!(!plan.supports_tiling());
    assert!(!plan.reuses_preview_grid());

    let maximum_grid = ColorReconstructionPlan::new(
        ColorReconstructionConfig::new(100.0, 0.0, 0.0, 0.66, ColorReconstructionPrecedence::None)
            .expect("config"),
        RasterDimensions::new(1_000, 1_000).expect("dimensions"),
        ReconstructionBudget::default(),
    )
    .expect("maximum grid")
    .geometry();
    assert_eq!(
        (
            maximum_grid.size_x(),
            maximum_grid.size_y(),
            maximum_grid.size_z()
        ),
        (
            BILATERAL_MAX_RESOLUTION_SPATIAL + 1,
            BILATERAL_MAX_RESOLUTION_SPATIAL + 1,
            BILATERAL_MAX_RESOLUTION_RANGE + 1
        )
    );
}

#[test]
fn color_reconstruction_grid_rescale_matches_native_roi_coordinates() {
    assert_eq!(grid_rescale(7, 11, 100, 200, 90, 180, 0.5), (-36.5, -74.5));
    assert_eq!(grid_rescale(0, 0, 0, 0, 0, 0, 1.0), (0.0, 0.0));
}

#[test]
fn color_reconstruction_lab_grid_propagates_source_chroma_ratio() {
    let config = ColorReconstructionV3 {
        threshold: 100.0,
        spatial: 1.0,
        range: 50.0,
        hue: 0.5,
        precedence: ColorReconstructionPrecedence::None.id(),
    }
    .config()
    .expect("v3 config");
    let mut input = vec![pixel(50.0, 20.0, -10.0); 25];
    input[12] = pixel(110.0, 0.0, 0.0);
    let plan = ColorReconstructionPlan::new(
        config,
        RasterDimensions::new(5, 5).expect("dimensions"),
        ReconstructionBudget::default(),
    )
    .expect("plan");
    let result = plan.execute(&input).expect("reconstruction");
    let center = result.pixels()[12];
    assert_eq!(center.red().get().to_bits(), 110.0_f32.to_bits());
    assert!((center.green().get() - 44.0).abs() < 0.000_02);
    assert!((center.blue().get() + 22.0).abs() < 0.000_02);
    assert!(result.diagnostics().affected()[12]);
    assert!(result.diagnostics().candidate()[12]);
    assert_eq!(
        result.receipt().compatibility_name(),
        COLORRECONSTRUCTION_COMPATIBILITY_ID
    );
    assert_eq!(result.receipt().compatibility_name(), "colorreconstruct");
    assert_eq!(
        result.receipt(),
        plan.execute(&input).expect("repeat").receipt()
    );
}

#[test]
fn canonical_evaluator_crosses_rgb_lab_boundaries_for_color_reconstruction() {
    let scalar = |value| {
        ParameterValue::Scalar(FiniteF64::new(value).expect("finite reconstruction parameter"))
    };
    let operation = Operation::new(
        OperationId::new(91).expect("operation ID"),
        OperationKey::new("rusttable.colorreconstruct").expect("operation key"),
        true,
        [
            ("threshold", scalar(100.0)),
            ("spatial", scalar(4.0)),
            ("range", scalar(50.0)),
            ("hue", scalar(0.66)),
            ("precedence", ParameterValue::Integer(0)),
        ]
        .into_iter()
        .map(|(name, value)| (ParameterName::new(name).expect("parameter name"), value)),
    )
    .expect("Color Reconstruction operation");
    let edit = Edit::from_parts(
        EditId::new(92).expect("edit ID"),
        PhotoId::new(93).expect("photo ID"),
        Revision::ZERO,
        Revision::from_u64(1),
        [operation],
    )
    .expect("edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("pipeline");
    let dimensions = RasterDimensions::new(5, 5).expect("dimensions");
    let mut pixels = vec![pixel(0.8, 0.1, 0.1); 25];
    pixels[12] = pixel(2.0, 2.0, 2.0);
    let input = WorkingRgbImage::new(dimensions, pixels).expect("working image");

    let output = evaluate(&pipeline, &input).expect("RGB-to-Lab reconstruction evaluation");
    let center = output.pixel_slice()[12];
    assert_eq!(output.dimensions(), dimensions);
    assert_ne!(center, input.pixel_slice()[12]);
    assert!(center.red().get() > center.green().get());
}

#[test]
fn color_reconstruction_preserves_frames_without_usable_highlight_evidence() {
    let config =
        ColorReconstructionConfig::new(100.0, 1.0, 10.0, 0.66, ColorReconstructionPrecedence::None)
            .expect("config");
    let plan = ColorReconstructionPlan::new(
        config,
        RasterDimensions::new(5, 1).expect("dimensions"),
        ReconstructionBudget::default(),
    )
    .expect("plan");

    let below_transition = vec![pixel(94.0, 30.0, -15.0); 5];
    let result = plan.execute(&below_transition).expect("passthrough");
    assert_eq!(result.pixels(), below_transition);
    assert!(result.diagnostics().affected().iter().all(|value| !value));

    let no_evidence = vec![pixel(110.0, 30.0, -15.0); 5];
    let result = plan.execute(&no_evidence).expect("zero-weight passthrough");
    assert_eq!(result.pixels(), no_evidence);
    assert!(result.diagnostics().candidate().iter().all(|value| !value));
}

#[test]
fn color_reconstruction_hue_conversion_and_cancellation_are_source_derived() {
    let hues = [
        hue_conversion(0.0),
        hue_conversion(0.166),
        hue_conversion(0.498),
        hue_conversion(0.664),
        hue_conversion(1.0),
    ];
    let expected = [
        0.712_950_9,
        1.735_924_5,
        -2.867_277_1,
        -1.059_206_7,
        0.712_950_9,
    ];
    for (actual, expected) in hues.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.000_001);
    }

    let plan = ColorReconstructionPlan::new(
        ColorReconstructionConfig::new(100.0, 1.0, 10.0, 0.66, ColorReconstructionPrecedence::Hue)
            .expect("config"),
        RasterDimensions::new(5, 5).expect("dimensions"),
        ReconstructionBudget::default(),
    )
    .expect("plan");
    let input = vec![pixel(110.0, 0.0, 0.0); 25];
    assert_eq!(
        plan.execute_with_cancel(&input, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    );
}

#[test]
fn color_reconstruction_accepts_finite_native_state_but_rejects_nonfinite_values() {
    let native_outlier = ColorReconstructionConfig::new(
        200.0,
        2_000.0,
        75.0,
        -0.25,
        ColorReconstructionPrecedence::None,
    )
    .expect("native commit_params accepts finite values outside editor bounds");
    assert_eq!(
        native_outlier.threshold().get().to_bits(),
        200.0_f32.to_bits()
    );
    assert_eq!(
        native_outlier.spatial().get().to_bits(),
        2_000.0_f32.to_bits()
    );
    assert_eq!(native_outlier.range().get().to_bits(), 75.0_f32.to_bits());
    assert_eq!(native_outlier.hue().get().to_bits(), (-0.25_f32).to_bits());
    assert!(
        ColorReconstructionConfig::new(
            100.0,
            f32::NAN,
            10.0,
            0.66,
            ColorReconstructionPrecedence::None
        )
        .is_err()
    );
    let plan = ColorReconstructionPlan::new(
        ColorReconstructionConfig::new(100.0, 1.0, 10.0, 0.66, ColorReconstructionPrecedence::None)
            .expect("config"),
        RasterDimensions::new(5, 5).expect("dimensions"),
        ReconstructionBudget::new(1),
    );
    assert!(matches!(
        plan,
        Err(rusttable_processing::operations::OperationExecutionError::MemoryBudgetExceeded { .. })
    ));
}

#[test]
fn reconstruction_descriptors_preserve_distinct_colorspaces_and_native_flags() {
    let highlights = highlights_descriptor();
    assert_eq!(
        highlights.io.input.encodings,
        vec![ColorEncoding::LinearSrgbD65]
    );
    assert_eq!(
        highlights.io.output.encodings,
        vec![ColorEncoding::LinearSrgbD65]
    );

    let reconstruction = color_reconstruction_descriptor();
    assert_eq!(
        reconstruction.io.input.encodings,
        vec![ColorEncoding::LabD50]
    );
    assert_eq!(
        reconstruction.io.output.encodings,
        vec![ColorEncoding::LabD50]
    );
    assert!(
        reconstruction
            .flags
            .contains(OperationFlags::STYLE_ELIGIBLE)
    );
    assert!(
        reconstruction
            .flags
            .contains(OperationFlags::MULTI_INSTANCE)
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
