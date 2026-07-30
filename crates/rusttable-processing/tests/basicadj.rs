use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::descriptor::basicadj_descriptor;
use rusttable_processing::operations::basicadj::{BasicAdjNormEvidence, BasicAdjRgba};
use rusttable_processing::{
    BasicAdjAnalysisPlan, BasicAdjAnalysisRaster, BasicAdjAnalysisRoi, BasicAdjAutoControls,
    BasicAdjConfig, BasicAdjParametersV1, BasicAdjParametersV2, CompiledOperationGraph,
    CompiledPipeline, FiniteF32, FrameBoundaryMode, FrameBoundaryOptions, LinearRgb,
    OperationMaskSet, PreserveColors, ProcessingOperationKind, RasterDimensions,
    WorkingFrameDescriptor, WorkingRgbImage, builtin_registry, evaluate_graph_at_frame_boundaries,
    evaluate_graph_at_frame_boundaries_with_masks,
};

fn operation(parameters: &[(&str, f64)]) -> Operation {
    operation_with_opacity(1.0, parameters)
}

fn operation_with_opacity(opacity: f64, parameters: &[(&str, f64)]) -> Operation {
    operation_with_enabled_opacity(true, opacity, parameters)
}

fn operation_with_enabled_opacity(
    enabled: bool,
    opacity: f64,
    parameters: &[(&str, f64)],
) -> Operation {
    Operation::new_with_opacity(
        OperationId::new(321).expect("operation ID"),
        OperationKey::new("rusttable.basicadj").expect("operation key"),
        enabled,
        OperationOpacity::new(opacity).expect("opacity"),
        parameters.iter().map(|(name, value)| {
            (
                ParameterName::new(*name).expect("parameter name"),
                ParameterValue::Scalar(FiniteF64::new(*value).expect("finite value")),
            )
        }),
    )
    .expect("operation")
}

fn graph(opacity: f64) -> CompiledOperationGraph {
    graph_with_operation(operation_with_opacity(opacity, &[("exposure", 1.0)]))
}

fn graph_with_operation(operation: Operation) -> CompiledOperationGraph {
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [operation],
    )
    .expect("edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("pipeline");
    CompiledOperationGraph::from_pipeline(&pipeline)
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

#[test]
fn selected_roi_and_mask_use_native_pixel_boundaries() {
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
    let result = BasicAdjAnalysisPlan::analyze(config, raster).expect("analysis");
    assert_eq!(result.sample_count(), 6);
    assert_eq!(result.histogram()[1638], 3);
    assert_eq!(result.histogram()[4096], 3);
}

#[test]
fn one_pixel_selected_roi_falls_back_to_full_frame() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let pixels = [
        pixel(0.1, 0.1, 0.1),
        pixel(0.2, 0.2, 0.2),
        pixel(0.3, 0.3, 0.3),
        pixel(0.4, 0.4, 0.4),
        pixel(0.5, 0.5, 0.5),
        pixel(0.6, 0.6, 0.6),
    ];
    let roi = BasicAdjAnalysisRoi::new(1, 0, 1, 1).expect("one-pixel ROI");
    let raster =
        BasicAdjAnalysisRaster::with_roi(dimensions, &pixels, None, roi).expect("analysis raster");
    let config = BasicAdjConfig::defaults()
        .with_auto_controls(BasicAdjAutoControls::none().with_exposure(true));
    let result = BasicAdjAnalysisPlan::analyze(config, raster).expect("analysis");
    assert_eq!(result.sample_count(), 18);
}

#[test]
fn all_preserve_color_modes_keep_the_native_norm_order() {
    let source = [pixel(0.1, 0.3, 0.6)];
    for mode in 0..=6 {
        let mut parameters = BasicAdjParametersV2::defaults();
        parameters.contrast = 0.75;
        parameters.preserve_colors = mode;
        let plan = rusttable_processing::BasicAdjPlan::new(
            BasicAdjConfig::new(parameters).expect("parameters"),
        )
        .expect("plan");
        let output = if mode == PreserveColors::Luminance.id() {
            plan.execute_with_working_frame(&source, 0, WorkingFrameDescriptor::srgb())
                .expect("profile-aware execution")
        } else {
            plan.execute(&source, 0).expect("norm execution")
        };
        assert!(output[0].red().get().is_finite());
        assert!(output[0].green().get().is_finite());
        assert!(output[0].blue().get().is_finite());
    }
}

#[test]
fn luminance_uses_the_authoritative_working_profile_not_camera_coefficients() {
    let mut parameters = BasicAdjParametersV2::defaults();
    parameters.contrast = 1.0;
    let plan = rusttable_processing::BasicAdjPlan::new(
        BasicAdjConfig::new(parameters).expect("parameters"),
    )
    .expect("plan");
    let source = [pixel(0.1, 0.3, 0.6)];
    let srgb = plan
        .execute_with_working_frame(&source, 0, WorkingFrameDescriptor::srgb())
        .expect("sRGB profile");
    let rec2020 = plan
        .execute_with_working_frame(&source, 0, WorkingFrameDescriptor::rec2020())
        .expect("Rec. 2020 profile");
    assert_ne!(
        srgb[0].red().get().to_bits(),
        rec2020[0].red().get().to_bits()
    );
    let default = plan.execute(&source, 0).expect("default sRGB execution");
    assert_eq!(
        default[0].red().get().to_bits(),
        srgb[0].red().get().to_bits()
    );

    let mut highlight_parameters = BasicAdjParametersV2::defaults();
    highlight_parameters.hlcompr = 25.0;
    let highlight_plan = rusttable_processing::BasicAdjPlan::new(
        BasicAdjConfig::new(highlight_parameters).expect("highlight parameters"),
    )
    .expect("highlight plan");
    let default_highlight = highlight_plan
        .execute(&source, 0)
        .expect("default sRGB highlight execution");
    let profile_highlight = highlight_plan
        .execute_with_working_frame(&source, 0, WorkingFrameDescriptor::srgb())
        .expect("profile-aware highlight execution");
    assert_eq!(default_highlight, profile_highlight);
}

#[test]
fn rgba_execution_preserves_alpha_through_candidate_reconstruction() {
    let mut parameters = BasicAdjParametersV2::defaults();
    parameters.exposure = 0.5;
    parameters.saturation = 0.25;
    let plan = rusttable_processing::BasicAdjPlan::new(
        BasicAdjConfig::new(parameters).expect("parameters"),
    )
    .expect("plan");
    let alpha = FiniteF32::new(0.37).expect("alpha");
    let input = [BasicAdjRgba::new(pixel(0.1, 0.2, 0.3), alpha)];
    let output = plan
        .execute_rgba_with_working_frame(&input, 0, WorkingFrameDescriptor::srgb())
        .expect("RGBA execution");
    assert_eq!(output[0].alpha().get().to_bits(), alpha.get().to_bits());
}

#[test]
fn source_ordered_cpu_vector_matches_exposure_stage() {
    let mut parameters = BasicAdjParametersV2::defaults();
    parameters.exposure = 1.0;
    let plan = rusttable_processing::BasicAdjPlan::new(
        BasicAdjConfig::new(parameters).expect("parameters"),
    )
    .expect("plan");
    let output = plan
        .execute(&[pixel(0.1, 0.25, 0.5), pixel(0.75, 1.0, 1.5)], 0)
        .expect("exposure execution");
    let expected = [pixel(0.2, 0.5, 1.0), pixel(1.5, 2.0, 3.0)];
    assert_eq!(output, expected);
}

#[test]
fn one_resolved_plan_reuses_exactly_across_zero_overlap_tiles() {
    let dimensions = RasterDimensions::new(4, 1).expect("dimensions");
    let pixels = [
        pixel(0.1, 0.2, 0.3),
        pixel(0.4, 0.5, 0.6),
        pixel(0.7, 0.8, 0.9),
        pixel(1.0, 1.1, 1.2),
    ];
    let raster = BasicAdjAnalysisRaster::new(dimensions, &pixels, None).expect("raster");
    let config = BasicAdjConfig::defaults()
        .with_auto_controls(BasicAdjAutoControls::all().with_contrast(false));
    let plan = rusttable_processing::BasicAdjPlan::resolve(config, raster).expect("plan");
    let full = plan
        .execute_with_working_frame(&pixels, 0, WorkingFrameDescriptor::srgb())
        .expect("full execution");
    let mut tiled = plan
        .execute_with_working_frame(&pixels[..2], 0, WorkingFrameDescriptor::srgb())
        .expect("first tile");
    tiled.extend(
        plan.execute_with_working_frame(&pixels[2..], 2, WorkingFrameDescriptor::srgb())
            .expect("second tile"),
    );
    assert_eq!(full, tiled);
}

#[test]
fn invalid_exposure_scale_and_nonfinite_contrast_are_cpu_failures() {
    let mut scale_parameters = BasicAdjParametersV2::defaults();
    scale_parameters.exposure = 18.0;
    scale_parameters.black_point = (-18.0_f32).exp2();
    let scale_config = BasicAdjConfig::new(scale_parameters).expect("scale parameters");
    assert!(matches!(
        rusttable_processing::BasicAdjPlan::new(scale_config),
        Err(rusttable_processing::BasicAdjPlanError::InvalidExposureScale)
    ));

    let mut contrast_parameters = BasicAdjParametersV2::defaults();
    contrast_parameters.contrast = 5.0;
    let plan = rusttable_processing::BasicAdjPlan::new(
        BasicAdjConfig::new(contrast_parameters).expect("contrast parameters"),
    )
    .expect("plan");
    let evidence = BasicAdjNormEvidence::new([1.0, 0.0, 0.0], [1; 32]).expect("evidence");
    let error = plan
        .execute_with_norm_evidence(&[pixel(f32::MAX, 0.0, 0.0)], 0, &evidence)
        .expect_err("non-finite contrast reconstruction must fail closed");
    assert!(matches!(
        error,
        rusttable_processing::operations::OperationExecutionError::NonFiniteResult { .. }
    ));
}

#[test]
fn invalid_norm_evidence_fails_closed_before_execution() {
    let error = BasicAdjNormEvidence::new([0.0, 0.0, 0.0], [2; 32])
        .expect_err("zero luminance evidence must be rejected");
    assert!(matches!(
        error,
        rusttable_processing::operations::basicadj::BasicAdjNormEvidenceError::InvalidCoefficients
    ));
}

#[test]
fn cancellation_and_zero_overlap_publish_no_partial_tile() {
    let plan = rusttable_processing::BasicAdjPlan::new(BasicAdjConfig::defaults()).expect("plan");
    let empty = plan.execute(&[], 0).expect("zero-overlap tile is identity");
    assert!(empty.is_empty());

    let pixels = vec![pixel(0.2, 0.3, 0.4); 2048];
    let evidence = BasicAdjNormEvidence::from_working_frame(WorkingFrameDescriptor::srgb())
        .expect("profile evidence");
    let polls = std::cell::Cell::new(0_usize);
    let error = plan
        .execute_with_norm_evidence_and_cancellation(&pixels, 0, &evidence, || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 2
        })
        .expect_err("cancellation must prevent publication");
    assert!(matches!(
        error,
        rusttable_processing::operations::OperationExecutionError::Cancelled
    ));
    assert!(polls.get() >= 2);
}

#[test]
fn direct_norm_evidence_receipts_are_profile_qualified() {
    let plan = rusttable_processing::BasicAdjPlan::new(BasicAdjConfig::defaults()).expect("plan");
    let srgb = BasicAdjNormEvidence::from_working_frame(WorkingFrameDescriptor::srgb())
        .expect("sRGB evidence");
    let rec2020 = BasicAdjNormEvidence::from_working_frame(WorkingFrameDescriptor::rec2020())
        .expect("Rec. 2020 evidence");
    assert_ne!(
        plan.receipt_with_norm_evidence(srgb).profile_identity(),
        plan.receipt_with_norm_evidence(rec2020).profile_identity()
    );
}

#[test]
fn v2_descriptor_preserves_native_field_order() {
    let descriptor = basicadj_descriptor();
    let fields: Vec<_> = descriptor
        .parameters
        .iter()
        .map(|parameter| parameter.id.as_str())
        .collect();
    assert_eq!(
        fields,
        vec![
            "black_point",
            "exposure",
            "hlcompr",
            "hlcomprthresh",
            "contrast",
            "preserve_colors",
            "middle_grey",
            "brightness",
            "saturation",
            "vibrance",
            "clip",
        ]
    );
}

#[test]
fn v1_migration_preserves_native_fields_and_inserts_neutral_vibrance() {
    let old = BasicAdjParametersV1 {
        black_point: -0.25,
        exposure: 1.5,
        hlcompr: 42.0,
        hlcomprthresh: 7.0,
        contrast: 0.75,
        preserve_colors: PreserveColors::Power.id(),
        middle_grey: 20.0,
        brightness: -0.5,
        saturation: 0.25,
        clip: -0.1,
    };
    let migrated = rusttable_processing::migrate_v1_to_v2(old);
    assert_eq!(migrated.black_point.to_bits(), old.black_point.to_bits());
    assert_eq!(migrated.exposure.to_bits(), old.exposure.to_bits());
    assert_eq!(migrated.hlcompr.to_bits(), old.hlcompr.to_bits());
    assert_eq!(
        migrated.hlcomprthresh.to_bits(),
        old.hlcomprthresh.to_bits()
    );
    assert_eq!(migrated.contrast.to_bits(), old.contrast.to_bits());
    assert_eq!(migrated.preserve_colors, old.preserve_colors);
    assert_eq!(migrated.middle_grey.to_bits(), old.middle_grey.to_bits());
    assert_eq!(migrated.brightness.to_bits(), old.brightness.to_bits());
    assert_eq!(migrated.saturation.to_bits(), old.saturation.to_bits());
    assert_eq!(migrated.vibrance.to_bits(), 0.0_f32.to_bits());
    assert_eq!(migrated.clip.to_bits(), old.clip.to_bits());
}

#[test]
fn graph_fails_closed_for_unported_basicadj_blend_and_mask_semantics() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = WorkingRgbImage::new(dimensions, vec![pixel(0.4, 0.3, 0.2)]).expect("input");
    let alpha = [f32::from_bits(0x3eab_cdef)];
    let operation_id = OperationId::new(321).expect("operation ID");
    let masks = OperationMaskSet::from_entries([(
        operation_id,
        MaskRaster::new(1, 1, vec![0.5]).expect("mask"),
    )])
    .expect("mask set");

    let masked = evaluate_graph_at_frame_boundaries_with_masks(
        &graph(1.0),
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    );
    assert!(masked.is_err(), "Basic Adjust masks must fail closed");

    let blended = evaluate_graph_at_frame_boundaries(
        &graph(0.5),
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    );
    assert!(blended.is_err(), "Basic Adjust opacity must fail closed");
}

#[test]
fn disabled_basicadj_is_an_identity_pass_through() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = WorkingRgbImage::new(dimensions, vec![pixel(0.4, 0.3, 0.2)]).expect("input");
    let alpha = [f32::from_bits(0x3eab_cdef)];
    let graph = graph_with_operation(operation_with_enabled_opacity(
        false,
        0.5,
        &[("exposure", 4.0)],
    ));
    let output = evaluate_graph_at_frame_boundaries(
        &graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("disabled operation passes through");
    assert_eq!(output.image().pixel_slice(), input.pixel_slice());
    assert_eq!(output.alpha()[0].to_bits(), alpha[0].to_bits());
}
