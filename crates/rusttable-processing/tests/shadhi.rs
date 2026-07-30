#![allow(
    clippy::float_cmp,
    reason = "compatibility tests assert stable scalar values"
)]

use rusttable_core::{
    Edit, EditId, FiniteF64, Operation, OperationId, OperationKey, OperationOpacity, ParameterName,
    ParameterValue, PhotoId, Revision,
};
use rusttable_masks::MaskRaster;
use rusttable_processing::descriptor::OperationFlags;
use rusttable_processing::operations::shadhi::{
    SHADHI_V1_PARAMETER_BYTES, SHADHI_V5_PARAMETER_BYTES, ShadhiAlgorithm, ShadhiConfig,
    ShadhiHistory, ShadhiParametersV1, ShadhiParametersV5, ShadhiPixel, ShadhiPlan,
    migrate_v1_to_v5,
};
use rusttable_processing::{
    CompiledOperationGraph, CompiledPipeline, EvaluationError, FiniteF32, FrameBoundaryMode,
    FrameBoundaryOptions, LinearRgb, OperationMaskSet, RasterDimensions, WorkingFrameDescriptor,
    WorkingRgbImage, builtin_registry, descriptor, evaluate_bilateral_shadhi_with,
    evaluate_bilateral_shadhi_with_cancellation, evaluate_graph_at_frame_boundaries,
    evaluate_graph_at_frame_boundaries_with_masks,
    evaluate_graph_with_basicadj_plans_and_masks_with_cancellation,
};
use rusttable_processing::{
    ShadhiBilateralBoundaryError, ShadhiBilateralEvaluationError, common::bilateral::BilateralGrid,
};

fn pixel(red: f32, green: f32, blue: f32) -> LinearRgb {
    LinearRgb::new(
        FiniteF32::new(red).expect("finite red"),
        FiniteF32::new(green).expect("finite green"),
        FiniteF32::new(blue).expect("finite blue"),
    )
}

fn assert_close(actual: f32, expected: f32) {
    let tolerance = 16.0 * f32::EPSILON * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual} (tolerance {tolerance})"
    );
}

fn gaussian_config() -> ShadhiConfig {
    ShadhiConfig::new(ShadhiParametersV5 {
        shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
        radius: 10.0,
        ..ShadhiParametersV5::defaults()
    })
    .expect("Gaussian config")
}

fn working_fixture(dimensions: RasterDimensions) -> WorkingRgbImage {
    let width = dimensions.width();
    let width_f = f32::from(u16::try_from(width).expect("fixture width fits u16"));
    let height_f = f32::from(u16::try_from(dimensions.height()).expect("fixture height fits u16"));
    let pixels = (0..dimensions.pixel_count())
        .map(|index| {
            let x = f32::from(
                u16::try_from(index % u64::from(width)).expect("fixture x coordinate fits u16"),
            );
            let y = f32::from(
                u16::try_from(index / u64::from(width)).expect("fixture y coordinate fits u16"),
            );
            pixel(
                0.1 + x / (2.0 * width_f),
                0.15 + y / (2.0 * height_f),
                0.2 + (x + y) / (2.0 * (width_f + height_f)),
            )
        })
        .collect();
    WorkingRgbImage::new_with_frame(dimensions, pixels, WorkingFrameDescriptor::rec2020())
        .expect("working fixture")
}

fn cpu_bilateral_base(
    request: rusttable_processing::ShadhiBilateralRequest<'_>,
) -> Result<Vec<[f32; 4]>, rusttable_processing::common::bilateral::BilateralError> {
    let geometry = request.geometry();
    let mut grid = BilateralGrid::new(
        geometry.width(),
        geometry.height(),
        geometry.effective_sigma_s(),
        geometry.effective_sigma_r(),
    )?;
    grid.splat(request.guide())?;
    grid.blur()?;
    grid.slice(request.guide(), request.detail())
}

fn shadhi_graph(operation_id: OperationId, opacity: f32) -> CompiledOperationGraph {
    let operation = Operation::new_with_opacity(
        operation_id,
        OperationKey::new("rusttable.shadhi").expect("operation key"),
        true,
        OperationOpacity::new(f64::from(opacity)).expect("operation opacity"),
        std::iter::empty::<(ParameterName, ParameterValue)>(),
    )
    .expect("default bilateral shadhi");
    let edit = Edit::from_parts(
        EditId::new(0x5a).expect("edit ID"),
        PhotoId::new(0x5b).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [operation],
    )
    .expect("edit");
    CompiledOperationGraph::compile(&edit).expect("compiled graph")
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
fn lab_plan_executes_both_algorithms_and_is_deterministic() {
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
    let lab_input = input
        .iter()
        .map(|value| {
            ShadhiPixel::new(
                value.red().get() * 100.0,
                value.green().get() * 128.0,
                value.blue().get() * 128.0,
                1.0,
            )
        })
        .collect::<Vec<_>>();
    let gaussian = ShadhiPlan::new(gaussian_config(), dimensions).expect("Gaussian plan");
    let first = gaussian
        .execute_lab(&lab_input, None, 1.0, || false)
        .expect("first execution");
    assert_eq!(
        first,
        gaussian
            .execute_lab(&lab_input, None, 1.0, || false)
            .expect("second execution")
    );
    assert_ne!(first, lab_input);

    let bilateral =
        ShadhiPlan::new(ShadhiConfig::defaults(), dimensions).expect("default bilateral plan");
    let output = bilateral
        .execute_lab(&lab_input, None, 1.0, || false)
        .expect("default bilateral execution");
    assert_eq!(output.len(), lab_input.len());
    assert!(
        output
            .iter()
            .flat_map(|pixel| pixel.channels())
            .all(f32::is_finite)
    );
}

#[test]
fn bilateral_plan_routes_the_darktable_grid_base_layer() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let input = [
        [10.0, -15.0, 22.0, 0.10],
        [24.0, 18.0, -9.0, 0.20],
        [41.0, 5.0, 13.0, 0.30],
        [57.0, -4.0, -21.0, 0.40],
        [73.0, 27.0, 2.0, 0.50],
        [91.0, -31.0, 7.0, 0.60],
    ]
    .map(ShadhiPixel::from_channels);
    let config = ShadhiConfig::new(ShadhiParametersV5 {
        radius: 1.0,
        ..ShadhiParametersV5::defaults()
    })
    .expect("bilateral config");
    let plan = ShadhiPlan::new(config, dimensions).expect("bilateral plan");
    let output = plan
        .execute_lab(&input, None, 1.0, || false)
        .expect("bilateral execution");
    let expected_lightness = [
        15.150_055, 29.017_86, 42.203_65, 56.430_5, 68.676_796, 85.924_04,
    ];
    let expected_chroma = [
        [-20.543_82, 30.130_936],
        [19.720_814, -9.860_407],
        [5.025_150_3, 13.065_391],
        [-4.000_748_6, -21.003_931],
        [27.545_044, 2.040_373_6],
        [-36.915_863, 8.335_84],
    ];
    for (((actual, source), expected), chroma) in output
        .iter()
        .zip(&input)
        .zip(expected_lightness)
        .zip(expected_chroma)
    {
        let actual = actual.channels();
        let source = source.channels();
        assert_close(actual[0], expected);
        assert_close(actual[1], chroma[0]);
        assert_close(actual[2], chroma[1]);
        assert_eq!(
            actual[3], source[3],
            "alpha/spare must pass through the grid route"
        );
    }

    let mut cancellation_polls = 0;
    let cancelled = plan.execute_lab(&input, None, 1.0, || {
        cancellation_polls += 1;
        cancellation_polls > 1
    });
    assert!(matches!(
        cancelled,
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
    assert!(
        cancellation_polls > 1,
        "cancellation must be polled inside the bilateral grid sequence"
    );
}

#[test]
fn shadhi_chroma_correction_uses_pre_overlay_lightness_references() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let input = [ShadhiPixel::new(70.0, 32.0, -16.0, 1.0)];
    let filtered_base = [[80.0, 0.0, 0.0, 1.0]];
    let corrected_a = |highlights_ccorrect| {
        let config = ShadhiConfig::new(ShadhiParametersV5 {
            radius: 1.0,
            highlights_ccorrect,
            ..ShadhiParametersV5::defaults()
        })
        .expect("bilateral config");
        ShadhiPlan::new(config, dimensions)
            .expect("plan")
            .execute_lab_with_filtered_base(&input, &filtered_base, None, 1.0, || false)
            .expect("mixed output")[0]
            .channels()[1]
    };

    let reduced = corrected_a(0.0);
    let increased = corrected_a(100.0);
    assert!(reduced < input[0].channels()[1]);
    assert!(increased > input[0].channels()[1]);
}

#[test]
fn shadhi_whitepoint_and_bound_flags_match_retained_lab_rescale() {
    let dimensions = RasterDimensions::new(1, 1).expect("dimensions");
    let execute = |parameters: ShadhiParametersV5, lightness: f32, base_lightness: f32| {
        let config = ShadhiConfig::new(parameters).expect("source-derived config");
        ShadhiPlan::new(config, dimensions)
            .expect("plan")
            .execute_lab_with_filtered_base(
                &[ShadhiPixel::new(lightness, 24.0, -12.0, 0.5)],
                &[[base_lightness, 0.0, 0.0, 1.0]],
                None,
                1.0,
                || false,
            )
            .expect("mixed output")[0]
            .channels()
    };

    // `shadhi.c:421-422,489` divides positive normalized L by the derived
    // whitepoint and later applies only `_Lab_rescale` (`* 100`). It does not
    // multiply whitepoint back or add a final global clamp.
    for (whitepoint, lightness, expected) in [(10.0, 40.0, 44.444_443), (-10.0, 60.0, 54.545_456)] {
        let output = execute(
            ShadhiParametersV5 {
                shadows: 0.0,
                whitepoint,
                highlights: 0.0,
                flags: 0,
                ..ShadhiParametersV5::defaults()
            },
            lightness,
            50.0,
        );
        assert_close(output[0], expected);
        assert_eq!(output[3], 0.5);
    }

    // Retained flags are intentionally asymmetric in the shadow pass:
    // highlights-L controls the pre-overlay `la`, while shadows-L controls
    // the post-overlay clamp. With an unbound bilateral mask, these fixtures
    // distinguish both mixed flag combinations.
    for (flags, expected) in [(128 | 8, 100.0), (128 | 1, 104.938_27)] {
        let output = execute(
            ShadhiParametersV5 {
                shadows: 50.0,
                whitepoint: 10.0,
                highlights: -100.0,
                compress: 10.0,
                flags,
                ..ShadhiParametersV5::defaults()
            },
            70.0,
            0.0,
        );
        assert_close(output[0], expected);
        assert_eq!(output[3], 0.5);
    }
}

#[test]
fn shadhi_plan_identity_covers_every_output_affecting_color_field() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let identity = |parameters| {
        ShadhiPlan::new(ShadhiConfig::new(parameters).expect("config"), dimensions)
            .expect("plan")
            .cache_identity()
    };
    let base = identity(ShadhiParametersV5::defaults());
    assert_eq!(
        base,
        [
            0xac, 0xda, 0x00, 0x01, 0x5c, 0x08, 0x04, 0x32, 0x5d, 0x24, 0xae, 0xec, 0x9d, 0x1b,
            0x16, 0x56, 0xe0, 0x8e, 0x5c, 0x1c, 0x30, 0xe2, 0x91, 0x5f, 0x81, 0x17, 0x47, 0x56,
            0x1e, 0x27, 0x67, 0x6c,
        ],
        "v2 identity domain must remain stable"
    );

    for parameters in [
        ShadhiParametersV5 {
            shadows_ccorrect: 99.0,
            ..ShadhiParametersV5::defaults()
        },
        ShadhiParametersV5 {
            highlights_ccorrect: 49.0,
            ..ShadhiParametersV5::defaults()
        },
        ShadhiParametersV5 {
            low_approximation: 0.000_002,
            ..ShadhiParametersV5::defaults()
        },
    ] {
        assert_ne!(base, identity(parameters));
    }
}

#[test]
fn callback_bilateral_route_matches_the_canonical_cpu_lab_boundary() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let opacity = 0.37_f32;
    let operation = Operation::new_with_opacity(
        OperationId::new(0x51).expect("operation ID"),
        OperationKey::new("rusttable.shadhi").expect("operation key"),
        true,
        OperationOpacity::new(f64::from(opacity)).expect("operation opacity"),
        std::iter::empty::<(ParameterName, ParameterValue)>(),
    )
    .expect("default bilateral shadhi");
    let edit = Edit::from_parts(
        EditId::new(0x52).expect("edit ID"),
        PhotoId::new(0x53).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [operation],
    )
    .expect("edit");
    let graph = CompiledOperationGraph::from_pipeline(
        &CompiledPipeline::compile(&edit).expect("compiled pipeline"),
    );
    let canonical = evaluate_graph_at_frame_boundaries(
        &graph,
        &input,
        &vec![1.0; usize::try_from(dimensions.pixel_count()).expect("fixture pixels fit usize")],
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("canonical shadhi")
    .image()
    .clone();

    let accelerated = evaluate_bilateral_shadhi_with(
        &input,
        ShadhiConfig::defaults(),
        opacity,
        cpu_bilateral_base,
    )
    .expect("callback shadhi");

    assert_eq!(accelerated.frame(), canonical.frame());
    assert_eq!(accelerated.pixel_slice(), canonical.pixel_slice());
}

#[test]
fn graph_mask_and_opacity_are_combined_once_inside_the_shadhi_lab_boundary() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let operation_id = OperationId::new(0x59).expect("operation ID");
    let pixel_count = usize::try_from(dimensions.pixel_count()).expect("fixture pixels fit usize");
    let alpha = vec![1.0; pixel_count];
    let mask = MaskRaster::new(
        dimensions.width(),
        dimensions.height(),
        vec![0.5; pixel_count],
    )
    .expect("mask");
    let masks = OperationMaskSet::from_entries([(operation_id, mask)]).expect("operation mask set");

    let masked_half = evaluate_graph_at_frame_boundaries_with_masks(
        &shadhi_graph(operation_id, 0.5),
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        Some(&masks),
        || false,
    )
    .expect("masked half-opacity Shadhi")
    .image()
    .clone();
    let unmasked_quarter = evaluate_graph_at_frame_boundaries(
        &shadhi_graph(operation_id, 0.25),
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("quarter-opacity Shadhi")
    .image()
    .clone();

    // Darktable executes Shadhi on Lab and the pixelpipe combines module
    // opacity with mask coverage in that same color space.
    assert_eq!(masked_half.pixel_slice(), unmasked_quarter.pixel_slice());

    let unmasked_half = evaluate_graph_at_frame_boundaries(
        &shadhi_graph(operation_id, 0.5),
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("half-opacity Shadhi")
    .image()
    .clone();
    let differs_from_post_rgb_blend = masked_half
        .pixel_slice()
        .iter()
        .zip(unmasked_half.pixel_slice())
        .zip(input.pixel_slice())
        .any(|((actual, candidate), source)| {
            let legacy = [
                source.red().get() + (candidate.red().get() - source.red().get()) * 0.5,
                source.green().get() + (candidate.green().get() - source.green().get()) * 0.5,
                source.blue().get() + (candidate.blue().get() - source.blue().get()) * 0.5,
            ];
            actual.red().get().to_bits() != legacy[0].to_bits()
                || actual.green().get().to_bits() != legacy[1].to_bits()
                || actual.blue().get().to_bits() != legacy[2].to_bits()
        });
    assert!(
        differs_from_post_rgb_blend,
        "the fixture must distinguish native Lab coverage from a later RGB blend"
    );
}

#[test]
fn callback_receives_source_derived_effective_geometry_and_base_detail() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let config = ShadhiConfig::new(ShadhiParametersV5 {
        radius: 0.1,
        ..ShadhiParametersV5::defaults()
    })
    .expect("small-radius bilateral");
    let mut calls = 0;

    let output = evaluate_bilateral_shadhi_with(&input, config, 0.5, |request| {
        calls += 1;
        let geometry = request.geometry();
        assert_eq!(geometry.width(), 8);
        assert_eq!(geometry.height(), 6);
        assert_eq!(geometry.grid_dimensions(), [17, 13, 5]);
        assert_eq!(geometry.effective_sigma_s().to_bits(), 0.5_f32.to_bits());
        assert_eq!(geometry.effective_sigma_r().to_bits(), 25.0_f32.to_bits());
        assert_eq!(request.detail().to_bits(), (-1.0_f32).to_bits());
        assert_eq!(
            request.guide().len(),
            usize::try_from(dimensions.pixel_count()).expect("fixture pixels fit usize")
        );
        Ok::<_, rusttable_processing::common::bilateral::BilateralError>(request.guide().to_vec())
    })
    .expect("callback result");

    assert_eq!(calls, 1);
    assert_eq!(output.dimensions(), dimensions);
    assert_eq!(output.frame(), input.frame());
}

#[test]
fn callback_zero_opacity_is_bit_exact_and_skips_backend() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let mut calls = 0;

    let output = evaluate_bilateral_shadhi_with(
        &input,
        ShadhiConfig::defaults(),
        0.0,
        |_| -> Result<Vec<[f32; 4]>, rusttable_processing::common::bilateral::BilateralError> {
            calls += 1;
            panic!("zero opacity must not invoke the bilateral backend");
        },
    )
    .expect("zero-opacity identity");

    assert_eq!(calls, 0);
    assert_eq!(output, input);
}

#[test]
fn callback_cancellation_skips_backend_and_prevents_publication() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let mut calls = 0;
    let pre_cancelled = evaluate_bilateral_shadhi_with_cancellation(
        &input,
        ShadhiConfig::defaults(),
        1.0,
        |request| {
            calls += 1;
            cpu_bilateral_base(request)
        },
        || true,
    )
    .expect_err("pre-cancelled evaluation");
    assert_eq!(calls, 0);
    assert!(matches!(
        pre_cancelled,
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(
            rusttable_processing::operations::OperationExecutionError::Cancelled
        ))
    ));

    let cancelled = std::cell::Cell::new(false);
    let after_backend = evaluate_bilateral_shadhi_with_cancellation(
        &input,
        ShadhiConfig::defaults(),
        1.0,
        |request| {
            let filtered = request.guide().to_vec();
            cancelled.set(true);
            Ok::<_, rusttable_processing::common::bilateral::BilateralError>(filtered)
        },
        || cancelled.get(),
    )
    .expect_err("cancellation after backend must prevent publication");
    assert!(matches!(
        after_backend,
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(
            rusttable_processing::operations::OperationExecutionError::Cancelled
        ))
    ));
}

#[test]
fn canonical_graph_surfaces_shadhi_cancellation_as_typed_error() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let operation_id = OperationId::new(0x5c).expect("operation ID");
    let graph = shadhi_graph(operation_id, 1.0);
    let polls = std::cell::Cell::new(0_usize);

    let error = evaluate_graph_with_basicadj_plans_and_masks_with_cancellation(
        &graph,
        &input,
        None,
        None,
        || {
            let next = polls.get() + 1;
            polls.set(next);
            next >= 2
        },
    )
    .expect_err("cancellation must prevent graph output publication");

    assert_eq!(
        error,
        EvaluationError::Cancelled {
            step_index: rusttable_processing::PipelineStepIndex::new(0),
            operation_id,
        }
    );
    assert_eq!(polls.get(), 2);
}

#[test]
fn callback_rejects_invalid_opacity_before_invoking_backend() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let input = working_fixture(dimensions);

    for opacity in [f32::NAN, -0.01, 1.01] {
        let mut calls = 0;
        let error =
            evaluate_bilateral_shadhi_with(&input, ShadhiConfig::defaults(), opacity, |_| {
                calls += 1;
                Ok::<_, rusttable_processing::common::bilateral::BilateralError>(Vec::new())
            })
            .expect_err("invalid opacity");

        assert_eq!(calls, 0);
        assert!(matches!(
            error,
            ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(
                rusttable_processing::operations::OperationExecutionError::NonFiniteResult {
                    pixel: 0,
                    ..
                }
            ))
        ));
    }
}

#[test]
fn callback_ignores_nonfinite_filtered_channels_not_used_by_shadhi() {
    let dimensions = RasterDimensions::new(8, 6).expect("dimensions");
    let input = working_fixture(dimensions);
    let expected =
        evaluate_bilateral_shadhi_with(&input, ShadhiConfig::defaults(), 1.0, |request| {
            Ok::<_, rusttable_processing::common::bilateral::BilateralError>(
                request.guide().to_vec(),
            )
        })
        .expect("finite filtered base");

    let actual = evaluate_bilateral_shadhi_with(&input, ShadhiConfig::defaults(), 1.0, |request| {
        let mut filtered = request.guide().to_vec();
        for channels in &mut filtered {
            channels[1] = f32::NAN;
            channels[2] = f32::INFINITY;
            channels[3] = f32::NEG_INFINITY;
        }
        Ok::<_, rusttable_processing::common::bilateral::BilateralError>(filtered)
    })
    .expect("unused filtered channels");

    assert_eq!(actual, expected);
}

#[test]
fn callback_output_is_checked_before_shadhi_mixing() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let input = working_fixture(dimensions);

    let wrong_shape =
        evaluate_bilateral_shadhi_with(&input, ShadhiConfig::defaults(), 1.0, |request| {
            Ok::<_, rusttable_processing::common::bilateral::BilateralError>(
                request.guide()[..request.guide().len() - 1].to_vec(),
            )
        })
        .expect_err("short callback output");
    assert!(matches!(
        wrong_shape,
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(
            rusttable_processing::operations::OperationExecutionError::DimensionsMismatch {
                expected: 6,
                actual: 5,
            }
        ))
    ));

    let non_finite =
        evaluate_bilateral_shadhi_with(&input, ShadhiConfig::defaults(), 1.0, |request| {
            let mut output = request.guide().to_vec();
            output[2][0] = f32::NAN;
            Ok::<_, rusttable_processing::common::bilateral::BilateralError>(output)
        })
        .expect_err("non-finite callback output");
    assert!(matches!(
        non_finite,
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(
            rusttable_processing::operations::OperationExecutionError::NonFiniteResult {
                pixel: 2,
                ..
            }
        ))
    ));
}

#[test]
fn callback_backend_failure_remains_typed_for_pixelpipe_fallback() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let input = working_fixture(dimensions);
    let error = evaluate_bilateral_shadhi_with(&input, ShadhiConfig::defaults(), 1.0, |_| {
        Err::<Vec<[f32; 4]>, _>(rusttable_processing::common::bilateral::BilateralError::Cancelled)
    })
    .expect_err("backend failure");

    assert!(matches!(
        error,
        ShadhiBilateralEvaluationError::Backend(
            rusttable_processing::common::bilateral::BilateralError::Cancelled
        )
    ));
}

#[test]
fn callback_route_rejects_non_bilateral_shadhi_before_invoking_backend() {
    let dimensions = RasterDimensions::new(3, 2).expect("dimensions");
    let input = working_fixture(dimensions);
    let mut called = false;
    let error = evaluate_bilateral_shadhi_with(&input, gaussian_config(), 1.0, |_| {
        called = true;
        Ok::<_, rusttable_processing::common::bilateral::BilateralError>(Vec::new())
    })
    .expect_err("Gaussian shadhi has no bilateral callback route");

    assert!(!called);
    assert!(matches!(
        error,
        ShadhiBilateralEvaluationError::Boundary(ShadhiBilateralBoundaryError::Operation(
            rusttable_processing::operations::OperationExecutionError::UnsupportedCapability(
                "external bilateral base requires bilateral shadhi"
            )
        ))
    ));
}

#[test]
fn bilateral_plan_rejects_combined_base_and_grid_memory() {
    let dimensions = RasterDimensions::new(2_000, 2_000).expect("dimensions");
    let gaussian = ShadhiConfig::new(ShadhiParametersV5 {
        radius: 0.1,
        shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
        ..ShadhiParametersV5::defaults()
    })
    .expect("Gaussian config");
    ShadhiPlan::new(gaussian, dimensions).expect("base buffers fit the default budget");

    let bilateral = ShadhiConfig::new(ShadhiParametersV5 {
        radius: 0.1,
        ..ShadhiParametersV5::defaults()
    })
    .expect("bilateral config");
    let error = ShadhiPlan::new(bilateral, dimensions)
        .expect_err("base plus bilateral grid exceeds budget");
    let rusttable_processing::operations::OperationExecutionError::MemoryBudgetExceeded {
        required,
        budget,
    } = error
    else {
        panic!("unexpected plan error: {error}");
    };
    assert!(required > budget);

    let allocation = rusttable_processing::operations::OperationExecutionError::AllocationFailed {
        required: 4_096,
    };
    assert_eq!(
        allocation.to_string(),
        "operation could not allocate a required 4096-byte buffer"
    );
}

#[test]
fn historical_gaussian_orders_are_typed_and_executable() {
    let dimensions = RasterDimensions::new(5, 5).expect("dimensions");
    let input = vec![ShadhiPixel::new(35.0, 18.0, -12.0, 1.0); 25];
    for order in 0..=2 {
        let config = ShadhiConfig::new(ShadhiParametersV5 {
            order,
            shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
            radius: 3.0,
            ..ShadhiParametersV5::defaults()
        })
        .expect("historical order");
        let plan = ShadhiPlan::new(config, dimensions).expect("plan");
        let output = plan
            .execute_lab(&input, None, 1.0, || false)
            .expect("Gaussian order execution");
        assert!(
            output
                .iter()
                .flat_map(|pixel| pixel.channels())
                .all(f32::is_finite)
        );
    }
    assert!(
        ShadhiConfig::new(ShadhiParametersV5 {
            order: 3,
            ..ShadhiParametersV5::defaults()
        })
        .is_err()
    );
}

#[test]
fn descriptor_and_registry_advertise_lab_cpu_fallback_and_blending() {
    let descriptor = descriptor::shadhi_descriptor();
    descriptor.validate().expect("descriptor");
    assert!(descriptor.flags.contains(OperationFlags::FULL_IMAGE));
    assert!(descriptor.flags.contains(OperationFlags::DETERMINISTIC_CPU));
    assert!(descriptor.flags.contains(OperationFlags::MASKS));
    assert!(descriptor.flags.contains(OperationFlags::BLENDING));
    assert_eq!(descriptor.io.input.alpha, descriptor::AlphaPolicy::Preserve);
    assert_eq!(descriptor.io.input.channels, 4);
    assert_eq!(
        descriptor.io.input.encodings,
        vec![rusttable_color::ColorEncoding::LabD50]
    );
    assert!(descriptor.mask_blend.consumes_mask);
    assert_eq!(descriptor.migration.source_versions, [1, 2, 3, 4, 5]);
    let definition = builtin_registry()
        .definition("rusttable.shadhi")
        .expect("registry");
    assert!(definition.cpu().is_some());
    assert!(definition.gpu().is_none());
}

#[test]
fn masks_opacity_receipts_and_cancellation_are_part_of_the_plan_contract() {
    let dimensions = RasterDimensions::new(3, 3).expect("dimensions");
    let input = vec![ShadhiPixel::new(20.0, 5.0, -3.0, 0.25); 9];
    let plan = ShadhiPlan::new(
        ShadhiConfig::new(ShadhiParametersV5 {
            shadhi_algo: ShadhiAlgorithm::Gaussian.id(),
            radius: 2.0,
            ..ShadhiParametersV5::defaults()
        })
        .expect("config"),
        dimensions,
    )
    .expect("plan");
    let mask = vec![0.0; input.len()];
    let output = plan
        .execute_lab(&input, Some(&mask), 1.0, || false)
        .expect("masked execution");
    assert_eq!(output, input);
    let (_, receipt) = plan
        .execute_with_receipt(&input, None, 0.5, || false)
        .expect("receipt execution");
    assert_eq!(receipt.plan_identity(), plan.cache_identity());
    assert_ne!(receipt.input_identity(), receipt.output_identity());
    assert!(matches!(
        plan.execute_lab(&input, None, 1.0, || true),
        Err(rusttable_processing::operations::OperationExecutionError::Cancelled)
    ));
}

#[test]
fn mixed_rgb_lab_preview_and_export_share_the_same_shadhi_plan_and_alpha() {
    let dimensions = RasterDimensions::new(8, 8).expect("dimensions");
    let pixels = (0..64)
        .map(|index| {
            let x = index % 8;
            let y = index / 8;
            let x = f32::from(u16::try_from(x).expect("x fits u16"));
            let y = f32::from(u16::try_from(y).expect("y fits u16"));
            pixel(0.1 + x / 16.0, 0.15 + y / 16.0, 0.2 + (x + y) / 32.0)
        })
        .collect();
    let input =
        WorkingRgbImage::new_with_frame(dimensions, pixels, WorkingFrameDescriptor::rec2020())
            .expect("input");
    let shadhi = builtin_registry()
        .materialize_operation(
            "rusttable.shadhi",
            OperationId::new(2).expect("shadhi operation ID"),
        )
        .expect("default shadhi operation");
    let offset = Operation::new(
        OperationId::new(1).expect("offset operation ID"),
        OperationKey::new("rusttable.linear_offset").expect("offset key"),
        true,
        [(
            ParameterName::new("value").expect("value name"),
            ParameterValue::Scalar(FiniteF64::new(0.01).expect("finite value")),
        )],
    )
    .expect("offset operation");
    let edit = Edit::from_parts(
        EditId::new(1).expect("edit ID"),
        PhotoId::new(2).expect("photo ID"),
        Revision::ZERO,
        Revision::ZERO,
        [offset, shadhi],
    )
    .expect("edit");
    let pipeline = CompiledPipeline::compile(&edit).expect("pipeline");
    let graph = CompiledOperationGraph::from_pipeline(&pipeline);
    let alpha = vec![0.25; 64];
    let preview = evaluate_graph_at_frame_boundaries(
        &graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Preview),
        || false,
    )
    .expect("preview");
    let export = evaluate_graph_at_frame_boundaries(
        &graph,
        &input,
        &alpha,
        FrameBoundaryOptions::new(FrameBoundaryMode::Export),
        || false,
    )
    .expect("export");
    assert_eq!(preview.image().frame(), input.frame());
    assert_eq!(preview.alpha(), alpha.as_slice());
    assert_eq!(preview.image().pixel_slice(), export.image().pixel_slice());
    assert!(
        preview
            .image()
            .pixel_slice()
            .iter()
            .flat_map(|p| { [p.red().get(), p.green().get(), p.blue().get()] })
            .all(f32::is_finite)
    );
}
