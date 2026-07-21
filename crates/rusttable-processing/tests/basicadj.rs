use rusttable_core::{
    FiniteF64, Operation, OperationId, OperationKey, ParameterName, ParameterValue,
};
use rusttable_processing::{
    BasicAdjAnalysisPlan, BasicAdjAnalysisRaster, BasicAdjAnalysisRoi, BasicAdjAutoControls,
    BasicAdjConfig, BasicAdjParametersV2, FiniteF32, LinearRgb, PreserveColors,
    ProcessingOperationKind, RasterDimensions, builtin_registry,
};

fn operation(parameters: &[(&str, f64)]) -> Operation {
    Operation::new(
        OperationId::new(321).expect("operation ID"),
        OperationKey::new("rusttable.basicadj").expect("operation key"),
        true,
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite value")),
            )
        }),
    )
    .expect("operation")
}

#[test]
fn registry_compiles_basicadj_as_one_atomic_operation() {
    let prepared = builtin_registry()
        .prepare_cpu(&operation(&[
            ("exposure", 1.0),
            ("black_point", 0.05),
            ("contrast", 0.5),
            ("preserve_colors", 1.0),
        ]))
        .expect("basicadj factory");
    assert!(matches!(
        prepared.operation().kind(),
        ProcessingOperationKind::BasicAdj { config }
            if config.preserve_colors() == PreserveColors::Luminance
    ));
}

#[test]
fn compiler_rejects_unknown_preserve_colors_mode() {
    let error = builtin_registry()
        .prepare_cpu(&operation(&[("preserve_colors", 99.0)]))
        .expect_err("unknown mode must be rejected");
    assert!(error.to_string().contains("preserve-colors"));
}

#[test]
fn config_identity_includes_auto_clip_control() {
    let first = BasicAdjParametersV2::defaults();
    let mut second = first;
    second.clip = 0.1;
    let first = BasicAdjConfig::new(first).expect("first config");
    let second = BasicAdjConfig::new(second).expect("second config");
    let first_plan = rusttable_processing::BasicAdjPlan::new(first).expect("first plan");
    let second_plan = rusttable_processing::BasicAdjPlan::new(second).expect("second plan");
    assert_ne!(first_plan.identity(), second_plan.identity());
}

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

#[test]
fn analysis_is_stable_for_histogram_ties_and_repeated_runs() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let pixels = [
        pixel(0.1, 0.2, 0.3),
        pixel(0.4, 0.5, 0.6),
        pixel(0.7, 0.8, 0.9),
        pixel(1.0, 1.1, 1.2),
    ];
    let config = BasicAdjConfig::defaults().with_auto_controls(
        BasicAdjAutoControls::all()
            .with_brightness(false)
            .with_contrast(false),
    );
    let raster = BasicAdjAnalysisRaster::new(dimensions, &pixels, None).expect("raster");
    let first = BasicAdjAnalysisPlan::analyze(config, raster).expect("analysis");
    let second = BasicAdjAnalysisPlan::analyze(config, raster).expect("analysis");
    assert_eq!(first, second);
    assert_eq!(first.sample_count(), 12);
    assert_eq!(first.histogram().iter().sum::<u64>(), first.sample_count());
    assert!(first.percentiles()[2] <= first.percentiles()[4]);
    assert_ne!(first.identity(), [0; 32]);
}

#[test]
fn analysis_honors_mask_and_roi_before_resolving_one_plan() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let pixels = [
        pixel(0.1, 0.1, 0.1),
        pixel(0.2, 0.2, 0.2),
        pixel(0.3, 0.3, 0.3),
        pixel(0.4, 0.4, 0.4),
        pixel(0.5, 0.5, 0.5),
        pixel(0.6, 0.6, 0.6),
    ];
    let mask = [0.0, 1.0, 0.0, 0.0, 1.0, 0.0];
    let roi = BasicAdjAnalysisRoi::new(1, 0, 2, 2).expect("ROI");
    let raster = BasicAdjAnalysisRaster::with_roi(dimensions, &pixels, Some(&mask), roi)
        .expect("masked raster");
    let config = BasicAdjConfig::defaults()
        .with_auto_controls(BasicAdjAutoControls::none().with_exposure(true));
    let plan = rusttable_processing::BasicAdjPlan::resolve(config, raster).expect("plan");
    assert_ne!(plan.analysis_identity(), [0; 32]);
    assert!(plan.gpu_parameters().scale.is_finite());
}

#[test]
fn analysis_cancellation_never_publishes_a_partial_result() {
    let dimensions = RasterDimensions::new(2, 2).expect("dimensions");
    let pixels = [pixel(0.1, 0.2, 0.3); 4];
    let raster = BasicAdjAnalysisRaster::new(dimensions, &pixels, None).expect("raster");
    let config = BasicAdjConfig::defaults().with_auto_controls(BasicAdjAutoControls::all());
    let error = BasicAdjAnalysisPlan::analyze_with_cancellation(config, raster, || true)
        .expect_err("cancelled analysis");
    assert!(matches!(
        error,
        rusttable_processing::BasicAdjAnalysisError::Cancelled
    ));
}
