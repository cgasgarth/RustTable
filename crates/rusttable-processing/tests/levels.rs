#![allow(clippy::float_cmp)]

use rusttable_processing::RasterDimensions;
use rusttable_processing::operations::levels::{
    LEVELS_AUTO_HISTOGRAM_BINS, LEVELS_LUT_ENTRIES, LEVELS_MAXIMUM_LUT_BYTES, LevelsConfig,
    LevelsHistogram, LevelsHistory, LevelsMode, LevelsParametersV1, LevelsParametersV2,
    LevelsPixel, LevelsPlan, compute_automatic_levels, compute_manual_levels, migrate_v1_to_v2,
};
use rusttable_processing::operations::{OperationExecutionError, ReconstructionBudget};

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("test dimensions")
}

#[test]
fn v1_fixture_migrates_to_native_v2_defaults() {
    let bytes = hex_bytes(include_str!("fixtures/levels-params-v1.hex"));
    let old = LevelsParametersV1::from_bytes(&bytes).expect("v1 fixture");
    assert_eq!(old.levels_preset, 7);
    assert_eq!(
        migrate_v1_to_v2(old),
        LevelsParametersV2::new(LevelsMode::Manual, 0.0, 50.0, 100.0, [0.125, 0.5, 0.875])
    );
    assert_eq!(
        LevelsParametersV2::new(LevelsMode::Manual, 0.0, 50.0, 100.0, [0.125, 0.5, 0.875])
            .to_bytes()
            .as_slice(),
        hex_bytes(include_str!("fixtures/levels-params-v2.hex"))
    );
}

#[test]
fn decode_v1_history_materializes_native_current_parameters() {
    let bytes = hex_bytes(include_str!("fixtures/levels-params-v1.hex"));
    let expected =
        LevelsParametersV2::new(LevelsMode::Manual, 0.0, 50.0, 100.0, [0.125, 0.5, 0.875]);
    let history = LevelsHistory::decode(1, &bytes).expect("decode v1 history");

    assert_eq!(
        history.current().expect("v1 migrates").to_bytes(),
        expected.to_bytes()
    );
    assert_eq!(history.version(), 2);
    assert_eq!(history.payload(), expected.to_bytes());
}

#[test]
fn current_history_roundtrips_exact_declaration_order() {
    let bytes = hex_bytes(include_str!("fixtures/levels-params-v2.hex"));
    let parameters = LevelsParametersV2::from_bytes(&bytes).expect("v2 fixture");
    assert_eq!(
        parameters,
        LevelsParametersV2::new(LevelsMode::Manual, 0.0, 50.0, 100.0, [0.125, 0.5, 0.875])
    );
    assert_eq!(parameters.to_bytes().as_slice(), bytes);
}

#[test]
fn manual_histogram_uses_interleaved_l_bins_and_native_denominator() {
    let mut histogram = vec![0_u32; 256 * 4];
    histogram[4 * 3] = 2;
    histogram[4 * 200] = 2;
    let mut levels = [0.0, 0.5, 1.0];
    compute_manual_levels(Some(&histogram), &mut levels).expect("manual histogram");
    assert_eq!(levels, [12.0 / 1024.0, 406.0 / 1024.0, 800.0 / 1024.0]);
}

#[test]
fn automatic_histogram_preserves_percentile_order_and_marker_semantics() {
    assert_eq!(LEVELS_AUTO_HISTOGRAM_BINS, 16_384);

    let mut bins = vec![0_u32; 4 * 4];
    bins[0] = 2;
    bins[4] = 3;
    bins[8] = 4;
    bins[12] = 1;
    let histogram = LevelsHistogram::new(&bins, 4, 10).expect("automatic histogram");
    assert_eq!(
        compute_automatic_levels(Some(histogram), [20.0, 50.0, 80.0]),
        [0.0, 1.0 / 3.0, 2.0 / 3.0]
    );
    assert_eq!(
        compute_automatic_levels(None, [20.0, 50.0, 80.0]),
        [-f32::MAX, -f32::MAX, -f32::MAX]
    );
}

#[test]
fn automatic_plan_rejects_unresolved_native_marker_before_lut_construction() {
    let parameters =
        LevelsParametersV2::new(LevelsMode::Automatic, 50.0, 0.0, 100.0, [0.0, 0.5, 1.0]);
    let bins = vec![0_u32; 2 * 4];
    let histogram = LevelsHistogram::new(&bins, 2, 1).expect("automatic histogram");
    let result = LevelsPlan::new_with_budget(
        LevelsConfig::new(parameters).expect("finite parameters"),
        dimensions(1, 1),
        Some(histogram),
        ReconstructionBudget::new(0),
    );
    assert!(matches!(
        result,
        Err(OperationExecutionError::UnsupportedCapability(_))
    ));
}

#[test]
fn manual_plan_rejects_both_signed_maximum_markers_before_lut_construction() {
    for marker in [-f32::MAX, f32::MAX] {
        for index in 0..3 {
            let mut levels = [0.0, 0.5, 1.0];
            levels[index] = marker;
            let parameters = LevelsParametersV2::new(LevelsMode::Manual, 0.0, 50.0, 100.0, levels);
            let result = LevelsPlan::new_with_budget(
                LevelsConfig::new(parameters).expect("finite parameters"),
                dimensions(1, 1),
                None,
                ReconstructionBudget::new(0),
            );
            assert!(matches!(
                result,
                Err(OperationExecutionError::UnsupportedCapability(_))
            ));
        }
    }
}

#[test]
fn cpu_leaf_matches_lab_lightness_lut_and_preserves_alpha() {
    let config = LevelsConfig::defaults();
    let plan = LevelsPlan::new(config, dimensions(2, 1), None).expect("default plan");
    assert_eq!(plan.lut().len(), LEVELS_LUT_ENTRIES);
    assert_eq!(plan.levels(), [0.0, 0.5, 1.0]);
    assert_eq!(plan.in_inv_gamma(), 1.0);

    let input = [
        LevelsPixel::new(50.0, 20.0, -10.0, 0.75),
        LevelsPixel::new(100.0, 20.0, -10.0, f32::NAN),
    ];
    let output = plan.execute(&input).expect("CPU levels");
    assert_eq!(output[0].channels(), [50.0, 20.0, -10.0, 0.75]);
    assert_eq!(output[1].lightness(), 100.0);
    assert_eq!(output[1].a(), 20.0);
    assert_eq!(output[1].b(), -10.0);
    assert!(output[1].alpha().is_nan());
}

#[test]
fn cpu_leaf_applies_low_clip_and_contrast_preserving_ab_rescale() {
    let parameters = LevelsParametersV2::new(LevelsMode::Manual, 0.2, 0.5, 0.8, [0.2, 0.5, 0.8]);
    let plan = LevelsPlan::new(
        LevelsConfig::new(parameters).expect("finite parameters"),
        dimensions(2, 1),
        None,
    )
    .expect("manual plan");
    let output = plan
        .execute(&[
            LevelsPixel::new(10.0, 4.0, -3.0, 0.5),
            LevelsPixel::new(50.0, 4.0, -3.0, 0.5),
        ])
        .expect("CPU levels");
    assert_eq!(output[0].lightness(), 0.0);
    assert_eq!(output[0].a().to_bits(), 0.0_f32.to_bits());
    assert_eq!(output[0].b().to_bits(), (-0.0_f32).to_bits());
    assert!((output[1].lightness() - 50.0).abs() < f32::EPSILON);
    assert!((output[1].a() - 4.0).abs() < f32::EPSILON);
    assert!((output[1].b() + 3.0).abs() < f32::EPSILON);
}

#[test]
fn pointwise_tiles_need_no_overlap_and_share_committed_lut() {
    let plan =
        LevelsPlan::new(LevelsConfig::defaults(), dimensions(2, 2), None).expect("default plan");
    assert_eq!(plan.tiling().overlap_pixels, 0);
    assert_eq!(plan.tiling().temporary_multiplier_milli, 0);
    let input = [
        LevelsPixel::new(10.0, 1.0, 2.0, 0.0),
        LevelsPixel::new(20.0, 1.0, 2.0, 0.0),
    ];
    let output = plan
        .execute_with_input_dimensions(&input, dimensions(2, 1))
        .expect("pointwise tile");
    assert_eq!(output.len(), input.len());
}

#[test]
fn cancellation_happens_at_row_boundary_without_publishing_output() {
    let plan =
        LevelsPlan::new(LevelsConfig::defaults(), dimensions(2, 2), None).expect("default plan");
    let input = [LevelsPixel::new(50.0, 1.0, 2.0, 0.0); 4];
    let mut checks = 0;
    let result = plan.execute_with_cancel(&input, || {
        checks += 1;
        checks > 1
    });
    assert_eq!(result, Err(OperationExecutionError::Cancelled));
    assert!(checks >= 2);
}

#[test]
fn plan_admission_counts_lut_and_output_memory() {
    let result = LevelsPlan::new_with_budget(
        LevelsConfig::defaults(),
        dimensions(1, 1),
        None,
        ReconstructionBudget::new(LEVELS_MAXIMUM_LUT_BYTES),
    );
    assert_eq!(
        result,
        Err(OperationExecutionError::MemoryBudgetExceeded {
            required: LEVELS_MAXIMUM_LUT_BYTES + std::mem::size_of::<LevelsPixel>(),
            budget: LEVELS_MAXIMUM_LUT_BYTES,
        })
    );
}

fn hex_bytes(value: &str) -> Vec<u8> {
    let compact: String = value
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    (0..compact.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&compact[index..index + 2], 16).expect("hex fixture"))
        .collect()
}
