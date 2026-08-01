//! Source-derived bounded Color Mapping CPU leaf coverage.
//!
//! This test includes the operation by path so the leaf remains isolated from
//! the shared registry, production history routing, pixelpipe, GPU, and GTK
//! seams that are explicitly deferred.

#![allow(
    clippy::assertions_on_constants,
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::float_cmp,
    clippy::needless_range_loop,
    clippy::too_many_lines,
    clippy::unreadable_literal,
    reason = "source-derived vectors assert native ABI order and f32 boundaries"
)]

#[path = "../src/operations/colormapping/mod.rs"]
mod colormapping;

use colormapping::source_map::{COLOR_MAPPING_SOURCE_MAP, ColorMappingPortStatus};
use colormapping::{
    COLOR_MAPPING_CHANNELS, COLOR_MAPPING_PARAMETER_BYTES, COLOR_MAPPING_SCHEMA_VERSION,
    ColorMappingAcquisition, ColorMappingAnalysisError, ColorMappingCapabilityError,
    ColorMappingChannel, ColorMappingCodecError, ColorMappingConfig, ColorMappingExecutionError,
    ColorMappingHistory, ColorMappingParametersV1, ColorMappingPixel, ColorMappingPlan,
    ColorMappingTargetAnalysis, DEFAULT_CLUSTERS, DEFAULT_COLORSPACE, DEFAULT_DOMINANCE,
    DEFAULT_EQUALIZATION, DEFAULT_GROUPS, FLAG_HAS_SOURCE_TARGET, GPU_KERNELS, GPU_PROGRAM, HISTN,
    MAXN, MIGRATION_EDGES, PointsState, capabilities, capture_histogram, invert_histogram,
};

use rusttable_processing::RasterDimensions;

const DEFAULT_FIXTURE: &str = include_str!("fixtures/colormapping/default-v1.txt");
const MULTICLUSTER_FIXTURE: &str = include_str!("fixtures/colormapping/multicluster-4x3.lab");

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("nonzero dimensions")
}

fn pixel(lightness: f32, a: f32, b: f32, alpha: f32) -> ColorMappingPixel {
    [lightness, a, b, alpha]
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual:?}, expected {expected:?}, tolerance {tolerance:?}"
    );
}

fn multicluster_fixture() -> Vec<[ColorMappingPixel; 3]> {
    MULTICLUSTER_FIXTURE
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| {
            let values: [f32; 12] = line
                .split_whitespace()
                .map(|value| value.parse().expect("fixture f32"))
                .collect::<Vec<_>>()
                .try_into()
                .expect("twelve-channel fixture row");
            [
                values[0..4].try_into().expect("input Lab-alpha row"),
                values[4..8].try_into().expect("Shepard output row"),
                values[8..12]
                    .try_into()
                    .expect("equalized bilateral output row"),
            ]
        })
        .collect()
}

fn mapped_parameters(equalization: f32) -> ColorMappingParametersV1 {
    let mut parameters = ColorMappingParametersV1::defaults();
    parameters.flag = FLAG_HAS_SOURCE_TARGET;
    parameters.n = 1;
    parameters.equalization = equalization;
    parameters.source_ihist[0] = 100.0;
    parameters.source_mean[0] = [10.0, 20.0];
    parameters.source_var[0] = [2.0, 4.0];
    parameters.source_weight[0] = 1.0;
    parameters.target_hist.fill(0);
    parameters.target_mean[0] = [1.0, 2.0];
    parameters.target_var[0] = [1.0, 2.0];
    parameters.target_weight[0] = 1.0;
    parameters
}

fn multicluster_parameters(equalization: f32) -> ColorMappingParametersV1 {
    let mut parameters = ColorMappingParametersV1::defaults();
    parameters.flag = FLAG_HAS_SOURCE_TARGET;
    parameters.n = 3;
    parameters.dominance = 35.0;
    parameters.equalization = equalization;
    for index in 0..HISTN {
        let normalized = index as f32 / (HISTN - 1) as f32;
        parameters.source_ihist[index] = 10.0 + 90.0 * normalized * normalized;
        parameters.target_hist[index] = i32::try_from(index).expect("HISTN fits i32");
    }
    parameters.source_mean[..3].copy_from_slice(&[[-35.0, 25.0], [5.0, -30.0], [40.0, 15.0]]);
    parameters.source_var[..3].copy_from_slice(&[[12.0, 18.0], [8.0, 14.0], [20.0, 10.0]]);
    parameters.source_weight[..3].copy_from_slice(&[0.3, 0.2, 0.5]);
    parameters.target_mean[..3].copy_from_slice(&[[-25.0, -20.0], [0.0, 25.0], [30.0, -5.0]]);
    parameters.target_var[..3].copy_from_slice(&[[10.0, 12.0], [16.0, 9.0], [14.0, 20.0]]);
    parameters.target_weight[..3].copy_from_slice(&[0.2, 0.5, 0.3]);
    parameters
}

#[test]
fn native_abi_defaults_and_fixture_keep_the_full_array_layout() {
    assert_eq!(COLOR_MAPPING_SCHEMA_VERSION, 1);
    assert_eq!(COLOR_MAPPING_PARAMETER_BYTES, 16_600);
    assert_eq!(HISTN, 2_048);
    assert_eq!(MAXN, 5);
    assert_eq!(COLOR_MAPPING_CHANNELS, 4);
    assert_eq!(DEFAULT_CLUSTERS, 3);
    assert_eq!(DEFAULT_DOMINANCE, 100.0);
    assert_eq!(DEFAULT_EQUALIZATION, 50.0);
    assert_eq!(DEFAULT_COLORSPACE, "Lab");
    assert_eq!(DEFAULT_GROUPS, ["effect", "effects"]);
    assert_eq!(GPU_PROGRAM, 8);
    assert_eq!(
        GPU_KERNELS,
        ["colormapping_histogram", "colormapping_mapping"]
    );
    assert_eq!(MIGRATION_EDGES, &[]);
    assert!(DEFAULT_FIXTURE.contains("payload_bytes=16600"));
    assert!(DEFAULT_FIXTURE.contains(
        "field_order=flag,n,dominance,equalization,source_ihist,source_mean,source_var,source_weight,target_hist,target_mean,target_var,target_weight"
    ));

    let defaults = ColorMappingParametersV1::defaults();
    let bytes = defaults.to_bytes();
    assert_eq!(bytes.len(), COLOR_MAPPING_PARAMETER_BYTES);
    assert_eq!(
        &bytes[..16],
        &[0, 0, 0, 0, 3, 0, 0, 0, 0, 0, 200, 66, 0, 0, 72, 66]
    );
    assert_eq!(ColorMappingParametersV1::from_bytes(&bytes), Ok(defaults));
    assert_eq!(
        ColorMappingConfig::defaults().parameters().n,
        DEFAULT_CLUSTERS
    );
}

#[test]
fn known_history_round_trips_and_future_payloads_stay_opaque() {
    let payload = ColorMappingParametersV1::defaults().to_bytes();
    let history = ColorMappingHistory::decode(1, &payload).expect("v1 payload");
    assert_eq!(history.version(), 1);
    assert_eq!(history.payload(), payload);
    assert_eq!(
        history.current().expect("known current"),
        ColorMappingParametersV1::defaults()
    );

    let zero_payload = vec![0; COLOR_MAPPING_PARAMETER_BYTES];
    assert!(matches!(
        ColorMappingHistory::decode(1, &zero_payload),
        Ok(ColorMappingHistory::V1(_))
    ));
    let short_payload = vec![0; COLOR_MAPPING_PARAMETER_BYTES - 1];
    assert_eq!(
        ColorMappingHistory::decode(1, &short_payload),
        Err(ColorMappingCodecError::InvalidLength {
            expected: COLOR_MAPPING_PARAMETER_BYTES,
            actual: COLOR_MAPPING_PARAMETER_BYTES - 1,
        })
    );

    let future_bytes = vec![0xde, 0xad, 0xbe, 0xef];
    let future = ColorMappingHistory::decode(7, &future_bytes).expect("opaque future value");
    assert_eq!(future.version(), 7);
    assert_eq!(future.payload(), future_bytes);
    assert_eq!(
        future.current(),
        Err(ColorMappingCodecError::UnsupportedVersion(7))
    );
}

#[test]
fn histogram_accumulation_normalization_and_inverse_keep_native_order() {
    let input = [
        pixel(0.0, 0.0, 0.0, 1.0),
        pixel(25.0, 1.0, 1.0, 1.0),
        pixel(50.0, 2.0, 2.0, 1.0),
        pixel(75.0, 3.0, 3.0, 1.0),
        pixel(100.0, 4.0, 4.0, 1.0),
    ];
    let histogram = capture_histogram(dimensions(5, 1), &input).expect("histogram");
    assert_eq!(histogram[0], 409);
    assert_eq!(histogram[31], 409);
    assert_eq!(histogram[512], 819);
    assert_eq!(histogram[1_024], 1_228);
    assert_eq!(histogram[1_536], 1_638);
    assert_eq!(histogram[HISTN - 1], 2_047);

    let inverse = invert_histogram(&histogram);
    assert_eq!(inverse[0].to_bits(), 0.0_f32.to_bits());
    assert_eq!(
        inverse[31].to_bits(),
        (100.0 * 31.0 / 2_048.0_f32).to_bits()
    );
    assert_eq!(inverse[32].to_bits(), inverse[31].to_bits());
    assert_eq!(
        inverse[HISTN - 1].to_bits(),
        (100.0 * 2_047.0 / 2_048.0_f32).to_bits()
    );
}

#[test]
fn source_and_target_acquisition_keep_kmeans_statistics_and_flags() {
    let input = [pixel(10.0, 1.0, 2.0, 1.0), pixel(20.0, 3.0, 4.0, 1.0)];
    let mut acquisition = ColorMappingAcquisition::new();
    let source = acquisition
        .source_analysis(dimensions(2, 1), &input, 1)
        .expect("source analysis");
    assert_eq!(source.mean[0], [2.0, 3.0]);
    assert_eq!(source.variance[0], [1.0, 1.0]);
    assert_eq!(source.weight[0].to_bits(), 1.0_f32.to_bits());

    let target = acquisition
        .target_analysis(dimensions(2, 1), &input, 1)
        .expect("target analysis");
    assert_eq!(target.mean[0], [2.0, 3.0]);
    assert_eq!(target.variance[0], [1.0, 1.0]);
    assert_eq!(target.histogram[HISTN - 1], 2_047);

    let parameters = ColorMappingParametersV1::defaults()
        .reset_analysis()
        .with_source_analysis(&source)
        .with_target_analysis(&target);
    assert_eq!(parameters.flag, FLAG_HAS_SOURCE_TARGET);
    assert!(parameters.has_source());
    assert!(parameters.has_target());
}

#[test]
fn operation_local_points_stream_matches_native_step_sequence_across_acquisitions() {
    let mut sequence = PointsState::new();
    let actual = [
        sequence.next_f32().to_bits(),
        sequence.next_f32().to_bits(),
        sequence.next_f32().to_bits(),
        sequence.next_f32().to_bits(),
        sequence.next_f32().to_bits(),
        sequence.next_f32().to_bits(),
    ];
    assert_eq!(
        actual,
        [
            0x0000_0000,
            0x0000_0000,
            0x3680_0000,
            0x3740_0000,
            0x3780_0000,
            0x37a4_0000,
        ]
    );

    let input = [
        pixel(10.0, -20.0, -30.0, 1.0),
        pixel(20.0, -10.0, -20.0, 1.0),
        pixel(30.0, 10.0, 20.0, 1.0),
        pixel(40.0, 20.0, 30.0, 1.0),
    ];
    let mut acquisition = ColorMappingAcquisition::new();
    acquisition
        .source_analysis(dimensions(4, 1), &input, 2)
        .expect("source analysis");
    acquisition
        .target_analysis(dimensions(4, 1), &input, 2)
        .expect("target analysis");

    let mut expected = PointsState::new();
    for _ in 0..8 {
        let _ = expected.next_f32();
    }
    assert_eq!(acquisition.points(), &expected);
}

#[test]
fn invalid_parameters_and_analysis_inputs_fail_closed() {
    let mut parameters = ColorMappingParametersV1::defaults();
    parameters.n = 0;
    assert!(matches!(
        ColorMappingConfig::new(parameters),
        Err(colormapping::ColorMappingParameterError::InvalidClusterCount(0))
    ));

    let mut parameters = ColorMappingParametersV1::defaults();
    parameters.dominance = f32::NAN;
    assert!(matches!(
        ColorMappingConfig::new(parameters),
        Err(colormapping::ColorMappingParameterError::NonFinite(
            "dominance"
        ))
    ));

    let input = [pixel(1.0, 2.0, 3.0, f32::INFINITY)];
    assert!(capture_histogram(dimensions(1, 1), &input).is_ok());

    let invalid_lightness = [pixel(f32::NAN, 2.0, 3.0, 1.0)];
    assert_eq!(
        capture_histogram(dimensions(1, 1), &invalid_lightness),
        Err(ColorMappingAnalysisError::NonFiniteInput {
            pixel: 0,
            channel: ColorMappingChannel::Lightness,
        })
    );
}

#[test]
fn cpu_mapping_preserves_numeric_order_and_alpha_without_equalization() {
    let config = ColorMappingConfig::new(mapped_parameters(0.0)).expect("mapped parameters");
    let plan = ColorMappingPlan::new(config, dimensions(2, 1)).expect("plan");
    let input = [pixel(30.0, 12.0, 24.0, 0.25), pixel(60.0, 4.0, 8.0, 0.75)];
    let output = plan.execute(&input).expect("CPU output");
    assert_eq!(output[0][0].to_bits(), 30.0_f32.to_bits());
    assert_eq!(output[0][1].to_bits(), 32.0_f32.to_bits());
    assert_eq!(output[0][2].to_bits(), 64.0_f32.to_bits());
    assert_eq!(output[0][3].to_bits(), 0.25_f32.to_bits());
    assert_eq!(output[1][3].to_bits(), 0.75_f32.to_bits());
}

#[test]
fn lab_analysis_and_cpu_mapping_copy_through_nonfinite_alpha() {
    let input = [pixel(30.0, 12.0, 24.0, f32::NAN)];
    let analysis = ColorMappingTargetAnalysis::from_pixels(dimensions(1, 1), &input, 1)
        .expect("alpha is not part of Lab analysis");
    assert_eq!(analysis.histogram[HISTN - 1], 2_047);

    let config = ColorMappingConfig::new(mapped_parameters(0.0)).expect("mapped parameters");
    let plan = ColorMappingPlan::new(config, dimensions(1, 1)).expect("plan");
    let output = plan.execute(&input).expect("CPU output");
    assert!(output[0][3].is_nan());
}

#[test]
fn multicluster_fixture_covers_shepard_equalization_and_bilateral_slice() {
    let fixture = multicluster_fixture();
    let input: Vec<ColorMappingPixel> = fixture.iter().map(|row| row[0]).collect();
    let shepard = ColorMappingPlan::new_with_scale(
        ColorMappingConfig::new(multicluster_parameters(0.0)).expect("multi-cluster parameters"),
        dimensions(4, 3),
        25.0,
    )
    .expect("multi-cluster plan")
    .execute(&input)
    .expect("multi-cluster Shepard output");
    let equalized = ColorMappingPlan::new_with_scale(
        ColorMappingConfig::new(multicluster_parameters(65.0)).expect("multi-cluster parameters"),
        dimensions(4, 3),
        25.0,
    )
    .expect("multi-cluster plan")
    .execute(&input)
    .expect("multi-cluster equalized bilateral output");

    for (index, row) in fixture.iter().enumerate() {
        for channel in 0..3 {
            assert_close(shepard[index][channel], row[1][channel], 3.0e-5);
            assert_close(equalized[index][channel], row[2][channel], 3.0e-5);
        }
        assert_eq!(shepard[index][3].to_bits(), input[index][3].to_bits());
        assert_eq!(equalized[index][3].to_bits(), input[index][3].to_bits());
        assert_close(equalized[index][1], shepard[index][1], 3.0e-5);
        assert_close(equalized[index][2], shepard[index][2], 3.0e-5);
    }
    assert!(
        equalized
            .iter()
            .zip(&shepard)
            .any(|(with_bilateral, without)| (with_bilateral[0] - without[0]).abs() > 1.0)
    );
}

#[test]
fn bilateral_equalization_and_tiling_remain_finite_and_source_shaped() {
    let config = ColorMappingConfig::new(mapped_parameters(50.0)).expect("mapped parameters");
    let plan = ColorMappingPlan::new(config, dimensions(2, 2)).expect("plan");
    let input = [
        pixel(10.0, 12.0, 24.0, 0.1),
        pixel(20.0, 12.0, 24.0, 0.2),
        pixel(30.0, 12.0, 24.0, 0.3),
        pixel(40.0, 12.0, 24.0, 0.4),
    ];
    let output = plan.execute(&input).expect("bilateral CPU output");
    assert_eq!(output.len(), input.len());
    assert!(output.iter().flatten().all(|value| value.is_finite()));
    for (source, result) in input.into_iter().zip(output) {
        assert_eq!(source[3].to_bits(), result[3].to_bits());
    }
    let single_thread = plan.tiling(1).expect("single-thread tiling");
    let multi_thread = plan.tiling(4).expect("multi-thread tiling");
    assert_eq!(single_thread.alignment, 1);
    assert_eq!(single_thread.overlap, 200);
    assert!(single_thread.factor.is_finite());
    assert!(single_thread.max_buffer >= 1.0);
    assert!(multi_thread.factor > single_thread.factor);
    assert!(multi_thread.max_buffer > single_thread.max_buffer);
    assert_eq!(
        plan.tiling(0),
        Err(ColorMappingExecutionError::InvalidThreadCount)
    );
}

#[test]
fn cancellation_polls_validation_copy_and_publication_boundaries() {
    const SECOND_COPY_CHUNK_POLL: usize = 9;
    const COPY_PUBLICATION_POLL: usize = 12;
    const COMPLETE_PUBLICATION_POLL: usize = 20;

    let config = ColorMappingConfig::new(mapped_parameters(0.0)).expect("mapped parameters");
    let already_cancelled = ColorMappingPlan::new(config.clone(), dimensions(2, 1)).expect("plan");
    let malformed = [pixel(f32::NAN, 2.0, 3.0, 1.0)];
    assert_eq!(
        already_cancelled.execute_with_cancel(&malformed, || true),
        Err(ColorMappingExecutionError::Cancelled)
    );
    assert_eq!(
        ColorMappingTargetAnalysis::from_pixels_with_cancel(
            dimensions(2, 1),
            &malformed,
            1,
            || true,
        ),
        Err(ColorMappingAnalysisError::Cancelled)
    );

    let mut malformed_late = vec![pixel(30.0, 12.0, 24.0, 0.5); 3_073];
    malformed_late[3_072][0] = f32::NAN;
    let validation_plan =
        ColorMappingPlan::new(config.clone(), dimensions(3_073, 1)).expect("plan");
    let mut validation_polls = 0;
    assert_eq!(
        validation_plan.execute_with_cancel(&malformed_late, || {
            validation_polls += 1;
            validation_polls == 3
        }),
        Err(ColorMappingExecutionError::Cancelled)
    );
    assert_eq!(validation_polls, 3);
    let mut analysis_validation_polls = 0;
    assert_eq!(
        ColorMappingTargetAnalysis::from_pixels_with_cancel(
            dimensions(3_073, 1),
            &malformed_late,
            1,
            || {
                analysis_validation_polls += 1;
                analysis_validation_polls == 3
            },
        ),
        Err(ColorMappingAnalysisError::Cancelled)
    );
    assert_eq!(analysis_validation_polls, 3);

    let copy_input = vec![pixel(30.0, 12.0, 24.0, 0.5); 4_096];
    let copy_plan = ColorMappingPlan::new(ColorMappingConfig::defaults(), dimensions(4_096, 1))
        .expect("copy-through plan");
    for cancel_at in [SECOND_COPY_CHUNK_POLL, COPY_PUBLICATION_POLL] {
        let mut copy_polls = 0;
        assert_eq!(
            copy_plan.execute_with_cancel(&copy_input, || {
                copy_polls += 1;
                copy_polls == cancel_at
            }),
            Err(ColorMappingExecutionError::Cancelled)
        );
        assert_eq!(copy_polls, cancel_at);
    }

    let complete_plan = ColorMappingPlan::new(config.clone(), dimensions(4_096, 1)).expect("plan");
    let mut publication_polls = 0;
    assert_eq!(
        complete_plan.execute_with_cancel(&copy_input, || {
            publication_polls += 1;
            publication_polls == COMPLETE_PUBLICATION_POLL
        }),
        Err(ColorMappingExecutionError::Cancelled)
    );
    assert_eq!(publication_polls, COMPLETE_PUBLICATION_POLL);

    let tiny = ColorMappingPlan::new(config, dimensions(1, 1))
        .expect("plan")
        .with_memory_budget(1);
    assert_eq!(
        tiny.execute(&[pixel(1.0, 2.0, 3.0, 1.0)]),
        Err(ColorMappingExecutionError::AllocationFailed { required_bytes: 16 })
    );
}

#[test]
fn incomplete_parameters_copy_through_and_capabilities_remain_honest() {
    let config = ColorMappingConfig::defaults();
    let plan = ColorMappingPlan::new(config, dimensions(1, 1)).expect("plan");
    let input = [pixel(7.0, -2.0, 3.0, 0.125)];
    assert_eq!(plan.execute(&input).expect("copy-through"), input);

    let caps = capabilities();
    assert!(caps.cpu_supported);
    assert!(!caps.gpu_supported);
    assert!(!caps.gtk_supported);
    assert!(caps.supports_blending);
    assert!(!caps.masks_consumed);
    assert!(caps.outer_blending_deferred);
    assert!(!caps.production_routing_deferred);
    assert_eq!(
        caps.require_gpu(),
        Err(ColorMappingCapabilityError::GpuUnavailable)
    );
    assert_eq!(
        caps.require_gtk(),
        Err(ColorMappingCapabilityError::GtkUnavailable)
    );
    assert_eq!(
        caps.require_outer_blending(),
        Err(ColorMappingCapabilityError::OuterBlendingDeferred)
    );
    assert_eq!(caps.require_production_routing(), Ok(()));

    assert!(COLOR_MAPPING_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("process") && entry.status == ColorMappingPortStatus::Ported
    }));
    assert!(COLOR_MAPPING_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("colormapping_histogram")
            && entry.status == ColorMappingPortStatus::ExplicitlyDeferred
    }));
    assert!(COLOR_MAPPING_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("process-global ownership")
            && entry.rust_symbol.contains("shared per-worker points owner")
            && entry.status == ColorMappingPortStatus::ExplicitlyDeferred
    }));
    assert!(COLOR_MAPPING_SOURCE_MAP.iter().any(|entry| {
        entry.native_symbol.contains("dt_points_get_for")
            && entry.rust_symbol.contains("caller-injected PointsState")
            && entry.status == ColorMappingPortStatus::Ported
    }));
}
