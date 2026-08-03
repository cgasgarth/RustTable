#![allow(
    clippy::cast_precision_loss,
    clippy::float_cmp,
    reason = "source-derived tests compare native f32 values and deterministic byte layouts"
)]
#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Transfer test vectors preserve source evaluation order and IEEE-754 parity."
)]

#[path = "../src/operations/colortransfer/mod.rs"]
mod colortransfer;

use colortransfer::{
    COLORTRANSFER_CAPTURE_STRIDE, COLORTRANSFER_CHANNELS, COLORTRANSFER_COMPATIBILITY_ID,
    COLORTRANSFER_CPU_SUPPORTED, COLORTRANSFER_DEFAULT_CLUSTERS, COLORTRANSFER_GPU_SUPPORTED,
    COLORTRANSFER_HISTOGRAM_BINS, COLORTRANSFER_KMEANS_ITERATIONS, COLORTRANSFER_MAX_CLUSTERS,
    COLORTRANSFER_NATIVE_PARAMETER_BYTES, COLORTRANSFER_REGISTERED, COLORTRANSFER_RUST_ID,
    COLORTRANSFER_SAMPLE_FRACTION, COLORTRANSFER_SCHEMA_VERSION, COLORTRANSFER_UI_SUPPORTED,
    ColorTransferFlag, ColorTransferParameters, ColorTransferPixel, PointsRng,
};
use rusttable_processing::RasterDimensions;
use rusttable_processing::operations::{OperationExecutionError, ReconstructionBudget};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("valid dimensions")
}

fn pixels(width: u32, height: u32, phase: f32) -> Vec<ColorTransferPixel> {
    (0..height)
        .flat_map(|row| {
            (0..width).map(move |column| {
                let position = row * width + column;
                let angle = position as f32 * 0.173 + phase;
                ColorTransferPixel::new(
                    50.0 + 45.0 * angle.sin(),
                    55.0 * (angle * 0.71).cos(),
                    48.0 * (angle * 1.19).sin(),
                    0.15 + 0.7 * ((position % 17) as f32 / 16.0),
                )
            })
        })
        .collect()
}

#[test]
fn source_contract_constants_keep_integrated_deprecated_capabilities_truthful() {
    assert_eq!(COLORTRANSFER_COMPATIBILITY_ID, "colortransfer");
    assert_eq!(COLORTRANSFER_RUST_ID, "rusttable.colortransfer");
    assert_eq!(COLORTRANSFER_SCHEMA_VERSION, 1);
    assert_eq!(COLORTRANSFER_CHANNELS, 4);
    assert_eq!(COLORTRANSFER_CAPTURE_STRIDE, 3);
    assert_eq!(COLORTRANSFER_DEFAULT_CLUSTERS, 3);
    assert_eq!(COLORTRANSFER_HISTOGRAM_BINS, 2048);
    assert_eq!(COLORTRANSFER_KMEANS_ITERATIONS, 10);
    assert_eq!(COLORTRANSFER_MAX_CLUSTERS, 5);
    assert_eq!(COLORTRANSFER_SAMPLE_FRACTION, 0.2);
    const {
        assert!(COLORTRANSFER_CPU_SUPPORTED);
        assert!(!COLORTRANSFER_GPU_SUPPORTED);
        assert!(!COLORTRANSFER_UI_SUPPORTED);
        assert!(COLORTRANSFER_REGISTERED);
    }
    let input = vec![ColorTransferPixel::from_channels([12.0, 1.0, 2.0, 0.5])];
    let plan = ColorTransferParameters::default()
        .plan(dimensions(1, 1))
        .expect("native non-APPLY states have a pass-through plan");
    assert_eq!(plan.execute(&input).expect("pass-through"), input);
}

#[test]
fn native_parameter_abi_round_trips_every_field_and_unknown_flags() {
    assert_eq!(COLORTRANSFER_NATIVE_PARAMETER_BYTES, 8280);
    let mut bytes = vec![0_u8; COLORTRANSFER_NATIVE_PARAMETER_BYTES];
    bytes[0..4].copy_from_slice(&(-17_i32).to_le_bytes());
    bytes[4..8].copy_from_slice(&12.5_f32.to_le_bytes());
    bytes[8196..8200].copy_from_slice(&(-3.25_f32).to_le_bytes());
    bytes[8236..8240].copy_from_slice(&7.75_f32.to_le_bytes());
    bytes[8276..8280].copy_from_slice(&5_i32.to_le_bytes());

    let parameters = ColorTransferParameters::from_bytes(&bytes).expect("native payload");
    assert_eq!(parameters.flag(), ColorTransferFlag::Unknown(-17));
    assert_eq!(parameters.histogram()[0].to_bits(), 12.5_f32.to_bits());
    assert_eq!(parameters.means()[0][0].to_bits(), (-3.25_f32).to_bits());
    assert_eq!(parameters.variances()[0][0].to_bits(), 7.75_f32.to_bits());
    assert_eq!(parameters.clusters(), 5);
    assert_eq!(parameters.to_bytes(), bytes);

    let defaults = ColorTransferParameters::default();
    assert_eq!(
        defaults.to_bytes().len(),
        COLORTRANSFER_NATIVE_PARAMETER_BYTES
    );
    assert_eq!(defaults.flag(), ColorTransferFlag::Neutral);
    assert_eq!(defaults.clusters(), COLORTRANSFER_DEFAULT_CLUSTERS);
}

#[test]
fn histogram_capture_uses_native_three_float_stride_and_inverse_scale() {
    let dimensions = dimensions(2, 1);
    let input = vec![
        // Native capture sees L=10 for pixel zero and the first alpha lane as
        // L=50 for pixel one because its helper advances by three floats.
        ColorTransferPixel::new(10.0, 0.0, 0.0, 50.0),
        ColorTransferPixel::new(90.0, 0.0, 0.0, 0.75),
    ];
    let parameters = ColorTransferParameters::acquire(&input, dimensions).expect("capture");

    // The maximum captured sample is the first pixel's alpha, not the second
    // pixel's logical L lane. One occupied maximum bin maps back exactly.
    assert_eq!(
        parameters.histogram()[COLORTRANSFER_HISTOGRAM_BINS - 1],
        50.0
    );
    assert_eq!(parameters.flag(), ColorTransferFlag::Acquire2);
}

#[test]
fn native_xorshift_stream_is_kept_in_source_order() {
    let mut rng = colortransfer::PointsRng {
        state0: 1,
        state1: 2,
    };
    let expected = [0x0000_0000, 0x0000_0000, 0x3680_0000, 0x3740_0000];
    for expected_bits in expected {
        assert_eq!(rng.next_f32().to_bits(), expected_bits);
    }
}

#[test]
fn stochastic_capture_consumes_persistent_points_state() {
    let dimensions = dimensions(64, 48);
    let input = pixels(dimensions.width(), dimensions.height(), 0.0);
    let mut rng = PointsRng::default();
    let initial = rng;
    let first = ColorTransferParameters::acquire_with_clusters_and_cancel_with_rng(
        &input,
        dimensions,
        COLORTRANSFER_DEFAULT_CLUSTERS,
        &mut rng,
        || false,
    )
    .expect("first capture")
    .to_bytes();
    let after_first = rng;
    let second = ColorTransferParameters::acquire_with_clusters_and_cancel_with_rng(
        &input,
        dimensions,
        COLORTRANSFER_DEFAULT_CLUSTERS,
        &mut rng,
        || false,
    )
    .expect("second capture")
    .to_bytes();
    assert_ne!(initial, after_first);
    assert_ne!(after_first, rng);
    assert_ne!(first, second);

    let five = ColorTransferParameters::acquire_with_clusters(&input, dimensions, 5)
        .expect("five-cluster capture");
    assert_eq!(five.clusters(), 5);
    assert!(ColorTransferParameters::acquire_with_clusters(&input, dimensions, 6).is_err());
}

#[test]
fn apply_preserves_alpha_and_returns_native_four_channel_results() {
    let dimensions = dimensions(64, 48);
    let target = pixels(dimensions.width(), dimensions.height(), 0.0);
    let source = pixels(dimensions.width(), dimensions.height(), 0.37);
    let parameters = ColorTransferParameters::acquire(&target, dimensions).expect("target");
    let plan = parameters.for_apply().plan(dimensions).expect("plan");
    assert_eq!(plan.dimensions(), dimensions);
    assert_eq!(
        source[0].channels()[3].to_bits(),
        source[0].alpha().to_bits()
    );
    let output = plan.execute(&source).expect("apply");

    assert_eq!(output.len(), source.len());
    for (input, result) in source.iter().zip(output) {
        assert_eq!(result.alpha().to_bits(), input.alpha().to_bits());
    }
}

#[test]
fn apply_flag_transition_changes_only_the_native_state_word() {
    let dimensions = dimensions(8, 8);
    let input = pixels(dimensions.width(), dimensions.height(), 0.0);
    let captured = ColorTransferParameters::acquire(&input, dimensions).expect("capture");
    let applied = captured.for_apply();
    assert_eq!(captured.flag(), ColorTransferFlag::Acquire2);
    let pass_through = captured
        .plan(dimensions)
        .expect("ACQUIRE2 remains a native pass-through state");
    assert_eq!(pass_through.execute(&input).expect("pass-through"), input);
    assert_eq!(applied.flag(), ColorTransferFlag::Apply);
    let captured_bytes = captured.to_bytes();
    let applied_bytes = applied.to_bytes();
    assert_eq!(&captured_bytes[4..], &applied_bytes[4..]);
    assert_ne!(&captured_bytes[..4], &applied_bytes[..4]);
}

#[test]
fn native_preview_acquisition_and_non_apply_states_are_pass_through() {
    let dimensions = dimensions(8, 8);
    let input = pixels(dimensions.width(), dimensions.height(), 0.0);
    let mut bytes = ColorTransferParameters::default().to_bytes();
    bytes[0..4].copy_from_slice(&0_i32.to_le_bytes());
    let mut parameters = ColorTransferParameters::from_bytes(&bytes).expect("ACQUIRE state");

    let acquired = parameters
        .process(&input, dimensions, true)
        .expect("preview acquisition");
    assert_eq!(acquired.output(), input.as_slice());
    assert_eq!(parameters.flag(), ColorTransferFlag::Acquire2);
    assert_eq!(acquired.parameter_flag(), ColorTransferFlag::Acquire2);
    assert_eq!(acquired.pipe_flag(), ColorTransferFlag::Acquired);
    assert!(
        acquired
            .pipe_parameters()
            .histogram()
            .iter()
            .any(|value| *value != 0.0)
    );

    let mut neutral = ColorTransferParameters::default();
    let neutral_result = neutral
        .process(&input, dimensions, true)
        .expect("neutral pass-through");
    assert_eq!(neutral_result.output(), input.as_slice());
    assert_eq!(neutral_result.pipe_flag(), ColorTransferFlag::Neutral);
    let mut budgeted = ColorTransferParameters::default();
    assert!(matches!(
        budgeted.process_with_budget_and_cancel(
            &input,
            dimensions,
            false,
            ReconstructionBudget::new(1),
            || false,
        ),
        Err(OperationExecutionError::MemoryBudgetExceeded { .. })
    ));

    let mut acquired_state = acquired.pipe_parameters().clone();
    let acquired_result = acquired_state
        .process(&input, dimensions, false)
        .expect("ACQUIRED pass-through");
    assert_eq!(acquired_result.output(), input.as_slice());
    assert_eq!(acquired_result.pipe_flag(), ColorTransferFlag::Acquired);
}

#[test]
fn preview_acquisition_publishes_acquire2_only_after_passthrough_succeeds() {
    let dimensions = dimensions(5, 1);
    let input = pixels(dimensions.width(), dimensions.height(), 0.0);
    let mut bytes = ColorTransferParameters::default().to_bytes();
    bytes[0..4].copy_from_slice(&0_i32.to_le_bytes());
    let mut parameters = ColorTransferParameters::from_bytes(&bytes).expect("ACQUIRE state");
    let before = parameters.to_bytes();
    let mut rng = PointsRng::default();

    // Before pass-through there are two preflight polls, one capture poll per
    // row, and one poll per stochastic sample in each native iteration.
    let samples = usize::try_from(dimensions.pixel_count() / 5).expect("sample count fits usize");
    let polls_before_passthrough = 2
        + usize::try_from(dimensions.height()).expect("height fits usize")
        + COLORTRANSFER_KMEANS_ITERATIONS * samples;
    let mut polls = 0;
    let result = parameters.process_with_budget_and_cancel_with_rng(
        &input,
        dimensions,
        true,
        ReconstructionBudget::default(),
        &mut rng,
        || {
            polls += 1;
            polls > polls_before_passthrough
        },
    );

    assert_eq!(result, Err(OperationExecutionError::Cancelled));
    assert_eq!(polls, polls_before_passthrough + 1);
    assert_eq!(parameters.flag(), ColorTransferFlag::Acquire);
    assert_eq!(parameters.to_bytes(), before);
}

#[test]
fn budgeted_acquisition_uses_the_caller_supplied_budget() {
    let dimensions = dimensions(5, 1);
    let input = pixels(dimensions.width(), dimensions.height(), 0.0);
    let mut rng = PointsRng::default();
    let initial_rng = rng;

    let result = ColorTransferParameters::acquire_with_clusters_and_budget_and_cancel_with_rng(
        &input,
        dimensions,
        COLORTRANSFER_DEFAULT_CLUSTERS,
        ReconstructionBudget::new(1),
        &mut rng,
        || false,
    );

    assert!(matches!(
        result,
        Err(OperationExecutionError::MemoryBudgetExceeded { budget: 1, .. })
    ));
    assert_eq!(rng, initial_rng);
}

#[test]
fn cancellation_is_polled_during_capture_and_apply_never_publishes() {
    let dimensions = dimensions(64, 48);
    let input = pixels(dimensions.width(), dimensions.height(), 0.0);
    let mut polls = 0;
    let cancelled = ColorTransferParameters::acquire_with_cancel(&input, dimensions, || {
        polls += 1;
        polls > 2
    });
    assert_eq!(cancelled, Err(OperationExecutionError::Cancelled));

    let parameters = ColorTransferParameters::acquire(&input, dimensions).expect("target");
    let plan = parameters.for_apply().plan(dimensions).expect("plan");
    assert_eq!(
        plan.execute_with_cancel(&input, || true),
        Err(OperationExecutionError::Cancelled)
    );
}

#[test]
fn dimensions_and_budget_contract_fail_closed() {
    let small_dimensions = dimensions(4, 4);
    let input = pixels(small_dimensions.width(), small_dimensions.height(), 0.0);
    let parameters = ColorTransferParameters::acquire(&input, small_dimensions).expect("target");
    let applied = parameters.for_apply();
    let plan = applied.plan(small_dimensions).expect("plan");
    assert_eq!(
        plan.execute(&[]),
        Err(OperationExecutionError::DimensionsMismatch {
            expected: 16,
            actual: 0,
        })
    );

    assert!(matches!(
        applied.plan_with_budget(small_dimensions, ReconstructionBudget::new(1)),
        Err(OperationExecutionError::MemoryBudgetExceeded { .. })
    ));
    let oversized = dimensions(20_000, 20_000);
    assert!(matches!(
        applied.plan(oversized),
        Err(OperationExecutionError::MemoryBudgetExceeded { .. })
    ));

    let mut invalid = applied.to_bytes();
    invalid[8276..8280].copy_from_slice(&0_i32.to_le_bytes());
    let invalid = ColorTransferParameters::from_bytes(&invalid).expect("decode invalid n");
    assert!(matches!(
        invalid.plan(dimensions(1, 1)),
        Err(OperationExecutionError::UnsupportedCapability(_))
    ));
}
