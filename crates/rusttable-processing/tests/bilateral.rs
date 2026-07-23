//! Source-derived contract for `src/common/bilateral.c` and
//! `src/common/bilateral.h`.

#![allow(
    clippy::float_cmp,
    reason = "channel-preservation checks require bit-identical source values"
)]

use rusttable_processing::common::bilateral::{BilateralError, BilateralGrid};

fn assert_close(actual: f32, expected: f32) {
    let tolerance = 8.0 * f32::EPSILON * expected.abs().max(1.0);
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected}, got {actual} (tolerance {tolerance})"
    );
}

fn source_pixels() -> Vec<[f32; 4]> {
    vec![
        [10.0, -15.0, 22.0, 0.10],
        [24.0, 18.0, -9.0, 0.20],
        [41.0, 5.0, 13.0, 0.30],
        [57.0, -4.0, -21.0, 0.40],
        [73.0, 27.0, 2.0, 0.50],
        [91.0, -31.0, 7.0, 0.60],
    ]
}

#[test]
fn grid_geometry_and_memory_match_the_c_clamps() {
    // sigma_s is floored to 0.5.  The requested z resolution is clamped
    // to four cells, so the effective range sigma becomes 100 / 4.
    let floor = BilateralGrid::new(8, 6, 0.1, 100.0).expect("valid bilateral grid");
    assert_eq!(floor.grid_dimensions(), [17, 13, 5]);
    assert_close(floor.effective_sigma_s(), 0.5);
    assert_close(floor.effective_sigma_r(), 25.0);

    // The C CPU allocation is the grid plus three size_x*size_z scratch
    // planes per worker.  The safe scalar port has one worker:
    // ((17 * 13 * 5) + (3 * 17 * 5)) * sizeof(float) = 5_440.
    assert_eq!(floor.memory_bytes(), 5_440);

    // A zero or negative spatial sigma follows the same source floor; it is
    // not an invalid parameter in the retained implementation.
    let zero = BilateralGrid::new(8, 6, 0.0, 100.0).expect("zero sigma_s is clamped");
    let negative = BilateralGrid::new(8, 6, -4.0, 100.0).expect("negative sigma_s is clamped");
    assert_eq!(zero.grid_dimensions(), floor.grid_dimensions());
    assert_eq!(negative.grid_dimensions(), floor.grid_dimensions());

    // Spatial resolution is capped at 3_000 cells and range resolution at
    // 50 cells before the effective sigmas and final (+1) dimensions are
    // recomputed.
    let capped = BilateralGrid::new(10_000, 1, 0.1, 0.01).expect("capped grid");
    assert_eq!(capped.grid_dimensions(), [3_001, 2, 51]);
    assert_close(capped.effective_sigma_s(), 10_000.0 / 3_000.0);
    assert_close(capped.effective_sigma_r(), 2.0);

    // The 3_000 cap applies before recomputing the effective sigma. At this
    // near-cap width, source-order f32 rounding makes the final product just
    // greater than 3_000, so ceil adds one more grid point.
    let near_cap = BilateralGrid::new(3_007, 1, 0.1, 100.0).expect("near-cap grid");
    assert_eq!(near_cap.grid_dimensions(), [3_002, 2, 5]);
}

#[test]
fn zero_detail_is_identity_and_slicing_preserves_a_b_and_alpha() {
    let input = source_pixels();
    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    // norm = -detail * sigma_r * 0.04, so detail zero copies the source.
    let identity = grid.slice(&input, 0.0).expect("zero-detail slice");
    assert_eq!(identity, input);

    let filtered = grid.slice(&input, -1.0).expect("base-layer slice");
    let expected_lightness = [
        14.118_055, 27.137_499, 41.433_01, 55.754_16, 70.007_416, 87.549_86,
    ];
    assert!(
        filtered
            .iter()
            .zip(&input)
            .any(|(actual, source)| actual[0] != source[0]),
        "a non-uniform lightness field must produce a bilateral correction"
    );
    for ((actual, source), expected) in filtered.iter().zip(&input).zip(expected_lightness) {
        assert_close(actual[0], expected);
        assert_eq!(actual[1], source[1], "a is copied, never filtered");
        assert_eq!(actual[2], source[2], "b is copied, never filtered");
        assert_eq!(actual[3], source[3], "alpha/spare channel is copied");
        assert!(actual[0] >= 0.0, "slice clamps lightness at zero");
    }
}

#[test]
fn zero_detail_clamps_only_the_lower_lightness_bound() {
    let input = vec![
        [-12.0, 1.0, 2.0, 0.10],
        [125.0, 3.0, 4.0, 0.20],
        [30.0, 5.0, 6.0, 0.30],
        [60.0, 7.0, 8.0, 0.40],
    ];
    let mut grid = BilateralGrid::new(2, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    let sliced = grid.slice(&input, 0.0).expect("zero-detail slice");
    assert_eq!(sliced[0], [0.0, 1.0, 2.0, 0.10]);
    assert_eq!(sliced[1], input[1], "lightness above 100 remains unclamped");
    for (actual, source) in sliced.iter().zip(&input) {
        assert_eq!(actual[1..], source[1..], "slice preserves a, b, and alpha");
    }

    let mut output = vec![
        [-9.0, 11.0, 12.0, 0.51],
        [140.0, 13.0, 14.0, 0.52],
        [40.0, 15.0, 16.0, 0.53],
        [70.0, 17.0, 18.0, 0.54],
    ];
    let original_output = output.clone();
    grid.slice_to_output(&input, &mut output, 0.0)
        .expect("zero-detail slice into output");
    assert_eq!(output[0][0], 0.0);
    assert_eq!(output[1][0], 140.0);
    for (actual, before) in output.iter().zip(original_output) {
        assert_eq!(
            actual[1..],
            before[1..],
            "slice_to_output preserves non-lightness channels"
        );
    }
}

#[test]
fn corner_impulse_matches_zero_extended_blur_boundaries() {
    let input = vec![
        [50.0, 1.0, 2.0, 0.10],
        [0.0, 3.0, 4.0, 0.20],
        [0.0, 5.0, 6.0, 0.30],
        [0.0, 7.0, 8.0, 0.40],
    ];
    let mut grid = BilateralGrid::new(2, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    // With effective sigma_s=0.5, each pixel splats density 400 at grid
    // coordinates two cells apart. Applying the source's zero-extended
    // [1,4,6,4,1]/16 spatial kernels and signed z derivative gives these
    // exact binary-fraction lightness values.
    let expected_lightness = [47.460_938, 1.171_875, 1.171_875, 0.195_312_5];
    let output = grid.slice(&input, -1.0).expect("corner impulse slice");
    for ((actual, source), expected) in output.iter().zip(&input).zip(expected_lightness) {
        assert_close(actual[0], expected);
        assert_eq!(actual[1..], source[1..]);
    }
}

#[test]
fn slice_to_output_adds_only_the_source_derived_lightness_delta() {
    let input = source_pixels();
    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    let detail = -1.0;
    let sliced = grid.slice(&input, detail).expect("ordinary slice");
    let mut output = vec![
        [120.0, 101.0, 102.0, 0.91],
        [121.0, 103.0, 104.0, 0.92],
        [122.0, 105.0, 106.0, 0.93],
        [123.0, 107.0, 108.0, 0.94],
        [124.0, 109.0, 110.0, 0.95],
        [125.0, 111.0, 112.0, 0.96],
    ];
    let original_output = output.clone();
    let expected_lightness = [
        124.118_06,
        124.137_5,
        122.433_01,
        121.754_16,
        121.007_416,
        121.549_86,
    ];

    grid.slice_to_output(&input, &mut output, detail)
        .expect("slice into existing output");

    for ((((actual, before), source), ordinary), expected) in output
        .iter()
        .zip(&original_output)
        .zip(&input)
        .zip(&sliced)
        .zip(expected_lightness)
    {
        let correction = ordinary[0] - source[0];
        assert_close(actual[0], expected);
        assert_close(actual[0], (before[0] + correction).max(0.0));
        assert_eq!(
            actual[1..],
            before[1..],
            "slice_to_output must not replace a, b, or alpha"
        );
    }
}

#[test]
fn retained_alias_entry_points_match_out_of_place_slicing() {
    let input = source_pixels();
    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    let expected = grid.slice(&input, -1.0).expect("ordinary slice");

    let mut slice_alias = input.clone();
    grid.slice_in_place(&mut slice_alias, -1.0)
        .expect("in-place slice");
    assert_eq!(slice_alias, expected);

    let mut output_alias = input.clone();
    grid.slice_to_output_in_place(&mut output_alias, -1.0)
        .expect("in-place additive slice");
    assert_eq!(output_alias, expected);
    for (actual, source) in output_alias.iter().zip(input) {
        assert_eq!(
            actual[1..],
            source[1..],
            "in-place additive slicing preserves non-lightness channels"
        );
    }
}

#[test]
fn already_cancelled_slice_reports_cancellation() {
    let input = source_pixels();
    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    let mut cancelled = || true;
    assert_eq!(
        grid.slice_with_cancel(&input, -1.0, &mut cancelled),
        Err(BilateralError::Cancelled)
    );
}

#[test]
fn mid_operation_cancellation_is_explicit_for_mutating_entry_points() {
    let input = source_pixels();

    let mut splat_grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    let mut splat_polls = 0;
    let splat = splat_grid.splat_with_cancel(&input, &mut || {
        splat_polls += 1;
        splat_polls >= 3
    });
    assert_eq!(splat, Err(BilateralError::Cancelled));

    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    let mut blur_polls = 0;
    let blur = grid.blur_with_cancel(&mut || {
        blur_polls += 1;
        blur_polls >= 3
    });
    assert_eq!(blur, Err(BilateralError::Cancelled));

    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");
    let mut output = input.clone();
    let mut slice_polls = 0;
    let slice = grid.slice_in_place_with_cancel(&mut output, -1.0, &mut || {
        slice_polls += 1;
        slice_polls >= 5
    });
    assert_eq!(slice, Err(BilateralError::Cancelled));
    assert_ne!(
        output[..3],
        input[..3],
        "the completed row is intentionally retained"
    );
    assert_eq!(
        output[3..],
        input[3..],
        "the caller must discard the documented partial output"
    );
}

#[test]
fn finite_detail_overflow_is_rejected_before_in_place_mutation() {
    let input = source_pixels();
    let mut grid = BilateralGrid::new(3, 2, 1.0, 100.0).expect("valid bilateral grid");
    grid.splat(&input).expect("matching input shape");
    grid.blur().expect("grid blur");

    assert_eq!(
        grid.slice(&input, f32::MAX),
        Err(BilateralError::InvalidParameter("detail"))
    );

    let mut in_place = input.clone();
    assert_eq!(
        grid.slice_in_place(&mut in_place, f32::MAX),
        Err(BilateralError::InvalidParameter("detail"))
    );
    assert_eq!(in_place, input, "validation precedes in-place mutation");

    let mut output = vec![[f32::MAX, 1.0, 2.0, 0.5]; input.len()];
    let original_output = output.clone();
    assert!(matches!(
        grid.slice_to_output(&input, &mut output, -1.0e37),
        Err(BilateralError::NonFiniteOutput { .. })
    ));
    assert_eq!(
        output, original_output,
        "validation precedes output mutation"
    );
}

#[test]
fn invalid_dimensions_parameters_and_buffer_shapes_are_rejected() {
    assert!(BilateralGrid::new(0, 2, 1.0, 100.0).is_err());
    assert!(BilateralGrid::new(2, 0, 1.0, 100.0).is_err());
    assert!(BilateralGrid::new(2, 2, f32::NAN, 100.0).is_err());
    assert!(BilateralGrid::new(2, 2, f32::INFINITY, 100.0).is_err());
    assert!(BilateralGrid::new(2, 2, 1.0, 0.0).is_err());
    assert!(BilateralGrid::new(2, 2, 1.0, -1.0).is_err());
    assert!(BilateralGrid::new(2, 2, 1.0, f32::NAN).is_err());
    assert!(BilateralGrid::new(2, 2, 1.0, f32::INFINITY).is_err());

    let mut grid = BilateralGrid::new(2, 2, 1.0, 100.0).expect("valid bilateral grid");
    let short = vec![[25.0, 0.0, 0.0, 1.0]; 3];
    let exact = vec![[25.0, 0.0, 0.0, 1.0]; 4];
    let mut non_finite = exact.clone();
    non_finite[2][0] = f32::NAN;
    let mut short_output = vec![[0.0; 4]; 3];
    assert!(grid.splat(&short).is_err());
    assert_eq!(
        grid.splat(&non_finite),
        Err(BilateralError::NonFiniteLightness { pixel: 2 })
    );
    grid.splat(&exact).expect("matching input shape");
    grid.blur().expect("grid blur");
    assert!(grid.slice(&short, -1.0).is_err());
    assert!(grid.slice(&exact, f32::NAN).is_err());
    assert!(
        grid.slice_to_output(&exact, &mut short_output, -1.0)
            .is_err()
    );
}

#[test]
fn highly_asymmetric_rasters_keep_short_grid_axes_safe() {
    // Sharing one effective spatial sigma can reduce the shorter final grid
    // axis below the initial four-cell clamp. Darktable's boundary formulas
    // imply zero extension there; the safe port must not index beyond it.
    let input = (0..10_000)
        .map(|index| {
            let lightness =
                f32::from(u16::try_from(index % 101).expect("modulo result fits in u16"));
            [lightness, 7.0, -9.0, 0.75]
        })
        .collect::<Vec<_>>();
    let mut grid = BilateralGrid::new(10_000, 1, 0.1, 100.0).expect("skinny grid");
    assert_eq!(grid.grid_dimensions(), [3_001, 2, 5]);
    grid.splat(&input).expect("skinny splat");
    grid.blur().expect("short-axis blur");
    let output = grid.slice(&input, -1.0).expect("skinny slice");
    assert_eq!(output.len(), input.len());
    for (actual, source) in output.iter().zip(input) {
        assert!(actual[0].is_finite());
        assert_eq!(actual[1..], source[1..]);
    }
}
