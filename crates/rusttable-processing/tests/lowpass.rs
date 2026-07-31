#![allow(
    clippy::float_cmp,
    clippy::unreadable_literal,
    reason = "source-derived f32 expectations intentionally retain native spellings"
)]

#[path = "../src/operations/lowpass/mod.rs"]
mod lowpass;

use lowpass::{
    GaussianOrder, LOWPASS_LEGACY_PARAMETER_BYTES, LOWPASS_MIGRATION_EDGES, LowpassAlgorithm,
    LowpassAllocationMode, LowpassCapabilities, LowpassCodecError, LowpassConfig, LowpassError,
    LowpassHistory, LowpassMigrationError, LowpassParametersV1, LowpassParametersV2,
    LowpassParametersV3, LowpassParametersV4, LowpassPlan,
};
use rusttable_processing::{RasterDimensions, operations::ReconstructionBudget};

const FIXTURE: &str = include_str!("fixtures/lowpass/impulse_3x2.lab");

fn fixture() -> Vec<[f32; 4]> {
    FIXTURE
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| {
            let values: Vec<f32> = line
                .split_whitespace()
                .map(|value| value.parse().expect("fixture f32"))
                .collect();
            values.try_into().expect("four-channel fixture row")
        })
        .collect()
}

fn dimensions(width: u32, height: u32) -> RasterDimensions {
    RasterDimensions::new(width, height).expect("valid dimensions")
}

fn config(
    order: GaussianOrder,
    algorithm: LowpassAlgorithm,
    radius: f32,
    contrast: f32,
    brightness: f32,
    saturation: f32,
    unbound: i32,
) -> LowpassConfig {
    LowpassConfig::new(
        order, radius, contrast, brightness, saturation, algorithm, unbound,
    )
    .expect("valid lowpass config")
}

fn plan(config: LowpassConfig, width: u32, height: u32) -> LowpassPlan {
    LowpassPlan::new(config, dimensions(width, height)).expect("valid lowpass plan")
}

fn assert_close(actual: f32, expected: f32, tolerance: f32) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "actual {actual:?}, expected {expected:?}, tolerance {tolerance:?}"
    );
}

#[test]
fn native_payloads_are_little_endian_and_keep_all_field_offsets() {
    assert_eq!(LOWPASS_LEGACY_PARAMETER_BYTES, [16, 20, 24]);
    assert_eq!(LOWPASS_MIGRATION_EDGES, &[(1, 4), (2, 4), (3, 4)]);
    let v1 = LowpassParametersV1::new(2, -3.5, -1.25, 0.75).to_bytes();
    assert_eq!(v1.len(), 16);
    assert_eq!(&v1[0..4], &2u32.to_le_bytes());
    assert_eq!(&v1[4..8], &(-3.5f32).to_le_bytes());
    assert_eq!(&v1[8..12], &(-1.25f32).to_le_bytes());
    assert_eq!(&v1[12..16], &0.75f32.to_le_bytes());

    let v2 = LowpassParametersV2::new(1, 2.0, 0.5, -0.25, 1.5).to_bytes();
    assert_eq!(v2.len(), 20);
    assert_eq!(&v2[0..4], &1u32.to_le_bytes());
    assert_eq!(&v2[12..16], &(-0.25f32).to_le_bytes());
    assert_eq!(&v2[16..20], &1.5f32.to_le_bytes());

    let v3 = LowpassParametersV3::new(0, 4.0, 0.5, 0.25, 1.5, -7).to_bytes();
    assert_eq!(v3.len(), 24);
    assert_eq!(&v3[16..20], &1.5f32.to_le_bytes());
    assert_eq!(&v3[20..24], &(-7i32).to_le_bytes());

    let v4 = LowpassParametersV4::new(2, 4.0, 0.5, 0.25, 1.5, 1, -7).to_bytes();
    assert_eq!(v4.len(), 28);
    assert_eq!(&v4[16..20], &1.5f32.to_le_bytes());
    assert_eq!(&v4[20..24], &1u32.to_le_bytes());
    assert_eq!(&v4[24..28], &(-7i32).to_le_bytes());
    assert_eq!(
        LowpassParametersV4::from_bytes(&v4).expect("v4 decode"),
        LowpassParametersV4::new(2, 4.0, 0.5, 0.25, 1.5, 1, -7)
    );
}

#[test]
fn direct_migrations_preserve_fields_and_use_old_radius_sign() {
    let v1 = LowpassHistory::V1(LowpassParametersV1::new(2, -4.0, 0.5, 1.5))
        .migrate_to_v4()
        .expect("v1 migration");
    assert_eq!(v1, LowpassParametersV4::new(2, 4.0, 0.5, 0.0, 1.5, 1, 0));

    let v2 = LowpassHistory::V2(LowpassParametersV2::new(1, -5.0, -0.5, 0.25, 1.25))
        .migrate_to_v4()
        .expect("v2 migration");
    assert_eq!(v2, LowpassParametersV4::new(1, 5.0, -0.5, 0.25, 1.25, 1, 0));

    let v3 = LowpassHistory::V3(LowpassParametersV3::new(0, 6.0, 0.75, -0.25, -1.0, 9))
        .migrate_to_v4()
        .expect("v3 migration");
    assert_eq!(
        v3,
        LowpassParametersV4::new(0, 6.0, 0.75, -0.25, -1.0, 0, 9)
    );

    let signed_zero = LowpassHistory::V1(LowpassParametersV1::new(0, -0.0, 0.0, 1.0))
        .migrate_to_v4()
        .expect("signed zero migration");
    assert_eq!(signed_zero.lowpass_algo, 0);
    assert_eq!(signed_zero.radius.to_bits(), 0);
}

#[test]
fn current_metadata_sized_legacy_payloads_remain_opaque_until_authoritative_fixtures_exist() {
    for (version, length) in [(1, 216), (2, 312), (3, 424)] {
        let mut bytes = vec![0xa5; length];
        bytes[..4].copy_from_slice(&0u32.to_le_bytes());
        let history = LowpassHistory::decode(version, &bytes).expect("opaque metadata payload");
        assert_eq!(history.version(), version);
        assert_eq!(history.payload(), bytes);
        assert_eq!(
            history.migrate_to_v4(),
            Err(LowpassMigrationError::OpaqueVersion(version))
        );
    }
}

#[test]
fn exact_history_lengths_and_unknown_versions_are_fail_closed() {
    assert!(matches!(
        LowpassHistory::decode(1, &[0; 15]),
        Err(LowpassCodecError::InvalidLength {
            expected: 16,
            actual: 15
        })
    ));
    assert!(matches!(
        LowpassHistory::decode(4, &[0; 27]),
        Err(LowpassCodecError::InvalidLength {
            expected: 28,
            actual: 27
        })
    ));
    let unknown = LowpassHistory::decode(99, &[1, 2, 3]).expect("opaque history");
    assert_eq!(unknown.version(), 99);
    assert_eq!(unknown.payload(), vec![1, 2, 3]);
    assert_eq!(
        unknown.migrate_to_v4(),
        Err(LowpassMigrationError::OpaqueVersion(99))
    );
    assert_eq!(
        LowpassPlan::from_history(&unknown, dimensions(1, 1)),
        Err(LowpassError::OpaqueHistory(99))
    );
}

#[test]
fn defaults_and_local_contrast_mask_payload_match_native() {
    assert_eq!(
        LowpassConfig::defaults().parameters(),
        LowpassParametersV4::new(0, 10.0, 1.0, 0.0, 1.0, 0, 1)
    );
    let preset = LowpassParametersV4::new(0, 50.0, -1.0, 0.0, 0.0, 0, 1);
    assert_eq!(
        LowpassHistory::decode(4, &preset.to_bytes())
            .expect("preset")
            .payload(),
        preset.to_bytes().to_vec()
    );
}

#[test]
fn finite_parameters_keep_native_ui_range_as_metadata_only() {
    let config = LowpassConfig::new(
        GaussianOrder::Zero,
        900.0,
        -20.0,
        11.0,
        -8.0,
        LowpassAlgorithm::Gaussian,
        0,
    )
    .expect("finite historical parameters");
    let plan = LowpassPlan::new(config, dimensions(1, 1)).expect("out-of-range values execute");
    assert_eq!(plan.sigma().to_bits(), 900.0f32.to_bits());
    let output = plan
        .execute(&[[50.0, 0.0, 0.0, 0.25]])
        .expect("out-of-range output");
    assert!(output[0].iter().all(|value| value.is_finite()));
}

#[test]
fn gaussian_orders_match_source_equations_on_the_fixture() {
    let input = fixture();
    let expected = [
        [
            [44.61212, -25.178225, 24.037994, 0.1],
            [52.48413, -9.641966, 7.97297, 0.2],
            [58.346558, -5.630613, 3.687216, 0.3],
            [53.49121, -52.662697, 52.126827, 0.4],
            [58.718872, -33.61485, 33.16656, 0.5],
            [64.41345, -26.68975, 26.957048, 0.6],
        ],
        [
            [0.0, 77.38696, -67.18585, 0.1],
            [13.993835, 56.48266, -33.21637, 0.2],
            [18.417358, 33.715973, -17.44947, 0.3],
            [0.0, 52.11011, -45.240982, 0.4],
            [9.422302, 38.033768, -22.366936, 0.5],
            [12.402344, 22.703362, -11.749962, 0.6],
        ],
        [
            [0.0, 0.3294288, -0.24241301, 0.1],
            [1.0864258, -0.26247698, 0.42652568, 0.2],
            [0.16784668, -0.34007642, 0.2597159, 0.3],
            [0.12359619, -0.3294289, 0.24241304, 0.4],
            [0.0, 0.26247835, -0.4265271, 0.5],
            [0.0, 0.34007674, -0.2597162, 0.6],
        ],
    ];
    for (order_index, order) in [GaussianOrder::Zero, GaussianOrder::One, GaussianOrder::Two]
        .into_iter()
        .enumerate()
    {
        let output = plan(
            config(order, LowpassAlgorithm::Gaussian, 2.0, 1.0, 0.0, 1.0, 1),
            3,
            2,
        )
        .execute(&input)
        .expect("gaussian output");
        for (pixel, (actual, wanted)) in output.iter().zip(expected[order_index]).enumerate() {
            for channel in 0..4 {
                assert_close(actual[channel], wanted[channel], 2.0e-5);
            }
            assert_eq!(actual[3].to_bits(), input[pixel][3].to_bits());
        }
    }
}

#[test]
fn gaussian_recursive_boundaries_match_nonconstant_one_dimensional_inputs() {
    let row = [
        [0.0, 0.0, 0.0, 0.1],
        [20.0, 5.0, -5.0, 0.2],
        [80.0, 10.0, -10.0, 0.3],
        [40.0, 15.0, -15.0, 0.4],
    ];
    let column = vec![row[0], row[1], row[2], row[3]];
    let row_output = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            2.0,
            1.0,
            0.0,
            1.0,
            1,
        ),
        4,
        1,
    )
    .execute(&row)
    .expect("row output");
    let column_output = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            2.0,
            1.0,
            0.0,
            1.0,
            1,
        ),
        1,
        4,
    )
    .execute(&column)
    .expect("column output");
    let row_expected = [16.603088, 26.70288, 36.054993, 39.738464];
    let column_expected = [16.603088, 26.70288, 36.054993, 39.738464];
    for (actual, wanted) in row_output.iter().zip(row_expected) {
        assert_close(actual[0], wanted, 2.0e-5);
    }
    for (actual, wanted) in column_output.iter().zip(column_expected) {
        assert_close(actual[0], wanted, 2.0e-5);
    }
}

#[test]
fn bounded_and_unbound_paths_preserve_source_channel_contracts() {
    let input = vec![[80.0, 200.0, -200.0, 0.7]];
    let bounded = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            0.1,
            1.0,
            0.0,
            2.0,
            0,
        ),
        1,
        1,
    )
    .execute(&input)
    .expect("bounded output");
    assert_eq!(bounded[0][1], 128.0);
    assert_eq!(bounded[0][2], -128.0);
    assert_eq!(bounded[0][3].to_bits(), input[0][3].to_bits());
    let unbound = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            0.1,
            1.0,
            0.0,
            2.0,
            1,
        ),
        1,
        1,
    )
    .execute(&input)
    .expect("unbound output");
    assert_eq!(unbound[0][1].to_bits(), 400.0f32.to_bits());
    assert_eq!(unbound[0][2].to_bits(), (-400.0f32).to_bits());
}

#[test]
fn bilateral_fixture_matches_sliced_lightness_and_preserves_a_b_alpha() {
    let input = fixture();
    let output = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Bilateral,
            10.0,
            1.0,
            0.0,
            1.0,
            1,
        ),
        3,
        2,
    )
    .execute(&input)
    .expect("bilateral output");
    let expected_l = [1.7349243, 96.33026, 51.8631, 75.234985, 26.785278, 88.04779];
    for (index, (source, actual)) in input.iter().zip(output).enumerate() {
        assert_close(actual[0], expected_l[index], 2.0e-5);
        for channel in 1..4 {
            assert_eq!(actual[channel].to_bits(), source[channel].to_bits());
        }
    }
}

#[test]
fn curves_cover_linear_sigmoid_lut_edges_gamma_and_sequential_order() {
    for (lightness, expected) in [
        (-20.0, 0.0),
        (0.0, 0.0),
        (50.0, 50.0),
        (99.999, 99.998474),
        (100.0, 99.998474),
        (120.0, 119.99472),
    ] {
        let input = [[lightness, 0.0, 0.0, 0.4]];
        let output = plan(
            config(
                GaussianOrder::Zero,
                LowpassAlgorithm::Gaussian,
                0.1,
                1.0,
                0.0,
                1.0,
                1,
            ),
            1,
            1,
        )
        .execute(&input)
        .expect("edge output");
        assert_close(output[0][0], expected, 2.0e-4);
    }
    let sigmoid = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            0.1,
            2.0,
            0.0,
            1.0,
            1,
        ),
        1,
        1,
    )
    .execute(&[[25.0, 0.0, 0.0, 0.4]])
    .expect("sigmoid output");
    assert_close(sigmoid[0][0], 9.17511, 2.0e-5);
    let bright_up = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            0.1,
            1.0,
            1.0,
            1.0,
            1,
        ),
        1,
        1,
    )
    .execute(&[[25.0, 0.0, 0.0, 0.4]])
    .expect("positive gamma");
    let bright_down = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            0.1,
            1.0,
            -1.0,
            1.0,
            1,
        ),
        1,
        1,
    )
    .execute(&[[25.0, 0.0, 0.0, 0.4]])
    .expect("negative gamma");
    assert_close(bright_up[0][0], 50.0, 2.0e-5);
    assert_close(bright_down[0][0], 6.25, 2.0e-5);
    let sequential = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            0.1,
            2.0,
            1.0,
            1.0,
            1,
        ),
        1,
        1,
    )
    .execute(&[[25.0, 0.0, 0.0, 0.4]])
    .expect("sequential curves");
    assert_close(sequential[0][0], 30.290443, 2.0e-5);
}

#[test]
fn bilateral_geometry_and_plan_memory_use_are_source_shaped() {
    let plan = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Bilateral,
            10.0,
            1.0,
            0.0,
            1.0,
            1,
        ),
        3,
        2,
    );
    assert_eq!(plan.bilateral_geometry().expect("geometry").0, [5, 4, 5]);
    assert_eq!(
        plan.bilateral_geometry().expect("geometry").1.to_bits(),
        0.75f32.to_bits()
    );
    assert_eq!(
        plan.bilateral_geometry().expect("geometry").2.to_bits(),
        25.0f32.to_bits()
    );
    assert!(plan.required_memory_bytes() > 0);
    assert_eq!(plan.overlap_pixels(), 40);
}

#[test]
fn filter_initialization_failure_copies_caller_owned_destination_through() {
    let input = [[37.0, -9.0, 12.0, 0.25]];
    for (algorithm, failure_mode) in [
        (
            LowpassAlgorithm::Gaussian,
            LowpassAllocationMode::FailGaussianInitialization,
        ),
        (
            LowpassAlgorithm::Bilateral,
            LowpassAllocationMode::FailBilateralInitialization,
        ),
    ] {
        let filter_plan = plan(
            config(GaussianOrder::Zero, algorithm, 2.0, 1.0, 0.0, 1.0, 0),
            1,
            1,
        );
        let mut output = [[-100.0, 80.0, -60.0, 0.0]];
        filter_plan
            .execute_into_with_cancel_and_allocation_mode(&input, &mut output, failure_mode, || {
                false
            })
            .expect("native initialization failure copies through");
        assert_eq!(output[0][0].to_bits(), input[0][0].to_bits());
        assert_eq!(output[0][1].to_bits(), input[0][1].to_bits());
        assert_eq!(output[0][2].to_bits(), input[0][2].to_bits());
        assert_eq!(output[0][3].to_bits(), input[0][3].to_bits());
    }
}

fn assert_cancelled_without_destination_publication(
    filter_plan: &LowpassPlan,
    input: &[[f32; 4]],
    cancel_after: usize,
) -> usize {
    let sentinel = [91.0, -82.0, 73.0, -64.0];
    let mut output = vec![sentinel; input.len()];
    let mut polls = 0;
    assert_eq!(
        filter_plan
            .execute_into_with_cancel(input, &mut output, || {
                polls += 1;
                polls > cancel_after
            })
            .expect_err("filter cancellation"),
        LowpassError::Cancelled
    );
    assert!(
        output.iter().all(|sample| sample == &sentinel),
        "cancellation must not publish a partial destination"
    );
    polls
}

#[test]
fn cancellation_enters_gaussian_recursion_before_destination_publication() {
    let input = vec![[50.0, 0.0, 0.0, 1.0]; 8 * 8];
    let filter_plan = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            2.0,
            1.0,
            0.0,
            1.0,
            0,
        ),
        8,
        8,
    );
    let polls = assert_cancelled_without_destination_publication(&filter_plan, &input, 10);
    assert!(
        polls >= 11,
        "validation must finish before Gaussian recursion"
    );
}

#[test]
fn cancellation_enters_each_bilateral_stage_before_destination_publication() {
    let input = vec![[50.0, 0.0, 0.0, 1.0]; 4 * 4];
    let filter_plan = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Bilateral,
            10.0,
            1.0,
            0.0,
            1.0,
            0,
        ),
        4,
        4,
    );
    let splat_polls = assert_cancelled_without_destination_publication(&filter_plan, &input, 6);
    assert!(
        splat_polls >= 7,
        "cancellation must enter bilateral splatting"
    );
    let blur_polls = assert_cancelled_without_destination_publication(&filter_plan, &input, 10);
    assert!(blur_polls >= 11, "cancellation must enter bilateral blur");
    let slice_polls = assert_cancelled_without_destination_publication(&filter_plan, &input, 86);
    assert!(
        slice_polls >= 87,
        "cancellation must enter bilateral slicing"
    );
    let curve_polls = assert_cancelled_without_destination_publication(&filter_plan, &input, 90);
    assert!(
        curve_polls >= 91,
        "cancellation must enter lowpass curve mixing"
    );
}

#[test]
fn dimensions_nonfinite_and_shared_budget_fail_closed() {
    let input = fixture();
    let plan = plan(
        config(
            GaussianOrder::Zero,
            LowpassAlgorithm::Gaussian,
            2.0,
            1.0,
            0.0,
            1.0,
            0,
        ),
        3,
        2,
    );
    assert!(matches!(
        plan.execute(&input[..5]),
        Err(LowpassError::DimensionsMismatch {
            expected: 6,
            actual: 5
        })
    ));
    let mut nonfinite = input;
    nonfinite[2][1] = f32::NAN;
    assert!(matches!(
        plan.execute(&nonfinite),
        Err(LowpassError::NonFiniteInput {
            pixel: 2,
            channel: 1
        })
    ));
    assert!(matches!(
        LowpassPlan::new_with_scale_and_budget(
            config(
                GaussianOrder::Zero,
                LowpassAlgorithm::Gaussian,
                2.0,
                1.0,
                0.0,
                1.0,
                0
            ),
            dimensions(3, 2),
            1.0,
            1.0,
            ReconstructionBudget::new(1)
        ),
        Err(LowpassError::MemoryBudgetExceeded { .. })
    ));
}

#[test]
fn capabilities_keep_unported_surfaces_unavailable() {
    assert_eq!(
        LowpassCapabilities::cpu_only(),
        LowpassCapabilities {
            cpu: true,
            gpu: false,
            lab_d50: true,
            tiling: false,
            masks: false,
            analysis: false,
            ui: false,
            presets: false
        }
    );
}
