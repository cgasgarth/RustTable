//! Source-derived contract for Darktable's `src/common/splines.cpp` V2 path.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::float_cmp,
    reason = "native quantization oracles and one-point bit behavior require direct f32 checks"
)]

use rusttable_processing::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveType, MAX_RESOLUTION,
};
use rusttable_processing::common::splines::{
    SplineError, interpolate_value_v2, interpolate_value_v2_periodic, sample_curve_v2,
};

const fn anchor(x: f32, y: f32) -> CurveAnchor {
    CurveAnchor::new(x, y)
}

fn curve(curve_type: CurveType, anchors: &[CurveAnchor]) -> Curve {
    Curve::new(curve_type, CurveBounds::unit(), anchors).expect("finite test curve")
}

#[test]
fn nonperiodic_dispatch_matches_native_v2_tables() {
    let anchors = [
        anchor(0.0, 0.1),
        anchor(0.17, 0.9),
        anchor(0.52, 0.2),
        anchor(0.78, 0.75),
        anchor(1.0, 0.4),
    ];
    let cases = [
        (
            CurveType::CubicSpline,
            [
                26, 121, 198, 235, 227, 187, 134, 84, 54, 59, 94, 141, 180, 195, 180, 146, 102,
            ],
        ),
        (
            CurveType::CatmullRom,
            [
                26, 117, 204, 229, 209, 169, 122, 80, 54, 59, 96, 145, 183, 190, 168, 134, 102,
            ],
        ),
        (
            CurveType::MonotoneHermite,
            [
                26, 118, 205, 228, 206, 165, 118, 76, 53, 61, 101, 151, 186, 188, 165, 133, 102,
            ],
        ),
    ];

    for (curve_type, expected) in cases {
        assert_eq!(
            sample_curve_v2(&curve(curve_type, &anchors), 17, 256, false)
                .expect("native-valid V2 table"),
            expected
        );
    }
}

#[test]
fn periodic_sampling_uses_variant_while_direct_periodic_uses_ordinary_monotone() {
    let anchors = [
        anchor(0.03, 0.1),
        anchor(0.10, 0.2),
        anchor(0.62, 0.9),
        anchor(0.91, 0.95),
    ];
    let sampled_variant =
        sample_curve_v2(&curve(CurveType::MonotoneHermite, &anchors), 17, 256, true)
            .expect("periodic monotone table");
    assert_eq!(
        sampled_variant,
        [
            59, 34, 60, 85, 112, 139, 165, 188, 208, 222, 230, 234, 238, 240, 242, 213, 59
        ]
    );

    let direct_ordinary = (0..17)
        .map(|sample| {
            let x = sample as f32 / 16.0;
            let value = interpolate_value_v2_periodic(&anchors, x, CurveType::MonotoneHermite, 1.0)
                .expect("ordinary direct periodic interpolation");
            (value * 255.0).round() as u16
        })
        .collect::<Vec<_>>();
    assert_eq!(
        direct_ordinary,
        [
            59, 34, 60, 84, 110, 136, 161, 184, 204, 219, 230, 236, 240, 242, 242, 213, 59
        ]
    );
    assert_ne!(sampled_variant, direct_ordinary);
}

#[test]
fn one_point_preserves_early_return_and_wrapper_quantization() {
    let anchors = [anchor(0.25, 0.5)];
    for curve_type in [
        CurveType::CubicSpline,
        CurveType::CatmullRom,
        CurveType::MonotoneHermite,
    ] {
        assert_eq!(
            interpolate_value_v2(&anchors, -100.0, curve_type).expect("one-point constant"),
            0.5
        );
        assert_eq!(
            interpolate_value_v2(&anchors, 100.0, curve_type).expect("one-point constant"),
            0.5
        );
    }

    let one = curve(CurveType::MonotoneHermite, &anchors);
    assert_eq!(
        sample_curve_v2(&one, 9, 256, false).expect("non-periodic one-point table"),
        [127, 127, 128, 127, 127, 127, 127, 127, 127]
    );
    assert_eq!(
        sample_curve_v2(&one, 9, 256, true).expect("periodic one-point table"),
        [128; 9]
    );

    let unclipped = curve(CurveType::MonotoneHermite, &[anchor(0.25, 2.0)]);
    assert_eq!(
        sample_curve_v2(&unclipped, 9, MAX_RESOLUTION, true),
        Err(SplineError::QuantizationOutOfRange { sample: 0 })
    );

    // Color Zones strength can move a y=1 endpoint to y=1.5. Native outer
    // samples convert 98_302.5 to int 98_302, then assign modulo 65_536.
    // The sole on-grid sample still passes through the wrapper's int clamp.
    let positive_wrap = curve(CurveType::MonotoneHermite, &[anchor(0.25, 1.5)]);
    assert_eq!(
        sample_curve_v2(&positive_wrap, 5, MAX_RESOLUTION, false)
            .expect("non-periodic positive int-to-u16 wrapping"),
        [32_766, 65_535, 32_766, 32_766, 32_766]
    );

    let negative_wrap = curve(CurveType::MonotoneHermite, &[anchor(0.25, -0.5)]);
    assert_eq!(
        sample_curve_v2(&negative_wrap, 5, MAX_RESOLUTION, false)
            .expect("non-periodic negative int-to-u16 wrapping"),
        [32_769, 0, 32_769, 32_769, 32_769]
    );
    assert_eq!(
        sample_curve_v2(&negative_wrap, 5, MAX_RESOLUTION, true),
        Err(SplineError::QuantizationOutOfRange { sample: 0 })
    );
}

#[test]
fn zero_anchor_expansion_is_identity_and_periodic_aliases_are_rejected() {
    let empty = curve(CurveType::CubicSpline, &[]);
    assert_eq!(
        sample_curve_v2(&empty, 5, MAX_RESOLUTION, false).expect("box diagonal identity"),
        [0, 16_384, 32_768, 49_151, 65_535]
    );
    assert_eq!(
        sample_curve_v2(&empty, 5, MAX_RESOLUTION, true),
        Err(SplineError::DuplicateAbscissa {
            first: 0,
            second: 1,
        })
    );
}

#[test]
fn nonperiodic_sampling_retains_source_order_gates_then_clips_and_sorts() {
    let reversed_gate = curve(
        CurveType::CatmullRom,
        &[
            anchor(0.75, 0.8),
            anchor(-1.0, 0.1),
            anchor(0.25, 0.2),
            anchor(0.50, 0.6),
        ],
    );
    assert_eq!(
        sample_curve_v2(&reversed_gate, 9, 256, false).expect("source-order gate table"),
        [204, 204, 204, 204, 204, 204, 153, 153, 153]
    );

    let raw = [
        anchor(0.2, 0.2),
        anchor(0.9, 0.1),
        anchor(0.7, 0.8),
        anchor(-0.2, 0.9),
        anchor(0.4, 0.4),
        anchor(0.8, 0.6),
    ];
    let retained_sorted = [
        anchor(0.2, 0.2),
        anchor(0.4, 0.4),
        anchor(0.7, 0.8),
        anchor(0.8, 0.6),
    ];
    let expected = [51, 51, 51, 75, 102, 139, 179, 204, 153, 153, 153];
    assert_eq!(
        sample_curve_v2(&curve(CurveType::CatmullRom, &raw), 11, 256, false)
            .expect("clipped and sorted table"),
        expected
    );
    assert_eq!(
        sample_curve_v2(
            &curve(CurveType::CatmullRom, &retained_sorted),
            11,
            256,
            false,
        )
        .expect("canonical retained table"),
        expected
    );
}

#[test]
fn spline_clipping_does_not_change_native_outer_constant_branches() {
    let anchors = [anchor(0.25, 2.0), anchor(0.75, 3.0)];
    for curve_type in [
        CurveType::CubicSpline,
        CurveType::CatmullRom,
        CurveType::MonotoneHermite,
    ] {
        assert_eq!(
            sample_curve_v2(&curve(curve_type, &anchors), 5, 256, false)
                .expect("representable outer constants"),
            [510, 255, 255, 255, 765]
        );
    }
}

#[test]
fn periodic_sampling_folds_then_sorts_without_a_minimum_x_offset() {
    let folded = [anchor(1.25, 0.2), anchor(-0.25, 0.8), anchor(0.50, 0.5)];
    let canonical = [anchor(0.25, 0.2), anchor(0.50, 0.5), anchor(0.75, 0.8)];
    let expected = [128, 80, 51, 77, 128, 179, 204, 175, 128];
    assert_eq!(
        sample_curve_v2(&curve(CurveType::CatmullRom, &folded), 9, 256, true)
            .expect("folded table"),
        expected
    );
    assert_eq!(
        sample_curve_v2(&curve(CurveType::CatmullRom, &canonical), 9, 256, true)
            .expect("canonical table"),
        expected
    );
}

#[test]
fn direct_interpolation_sorts_and_accepts_a_negative_period_as_native_does() {
    let unsorted = [anchor(0.8, 0.3), anchor(0.1, 0.7), anchor(0.5, 0.2)];
    let sorted = [anchor(0.1, 0.7), anchor(0.5, 0.2), anchor(0.8, 0.3)];
    for x in [0.0, 0.2, 0.6, 1.0] {
        assert_eq!(
            interpolate_value_v2(&unsorted, x, CurveType::CatmullRom)
                .expect("unsorted direct curve"),
            interpolate_value_v2(&sorted, x, CurveType::CatmullRom).expect("sorted direct curve")
        );
        assert_eq!(
            interpolate_value_v2_periodic(&unsorted, x, CurveType::CatmullRom, -1.0)
                .expect("negative native period"),
            interpolate_value_v2_periodic(&unsorted, x, CurveType::CatmullRom, 1.0)
                .expect("positive native period")
        );
    }
}

#[test]
fn native_undefined_duplicates_and_nonfinite_direct_inputs_are_typed_errors() {
    let duplicate = [anchor(0.25, 0.2), anchor(0.25, 0.8)];
    assert_eq!(
        interpolate_value_v2(&duplicate, 0.5, CurveType::CubicSpline),
        Err(SplineError::DuplicateAbscissa {
            first: 0,
            second: 1,
        })
    );

    let folded_duplicate = [anchor(0.0, 0.2), anchor(1.0, 0.8)];
    assert_eq!(
        interpolate_value_v2_periodic(&folded_duplicate, 0.5, CurveType::MonotoneHermite, 1.0,),
        Err(SplineError::DuplicateAbscissa {
            first: 0,
            second: 1,
        })
    );

    for anchors in [
        [anchor(f32::NAN, 0.5)],
        [anchor(0.5, f32::NAN)],
        [anchor(f32::INFINITY, 0.5)],
        [anchor(0.5, f32::NEG_INFINITY)],
    ] {
        assert_eq!(
            interpolate_value_v2(&anchors, 0.5, CurveType::CatmullRom),
            Err(SplineError::NonFiniteEvaluationInput)
        );
    }
    assert_eq!(
        interpolate_value_v2(&[anchor(0.5, 0.5)], f32::NAN, CurveType::CatmullRom),
        Err(SplineError::NonFiniteEvaluationInput)
    );
}

#[test]
fn invalid_resolutions_periods_and_nonfinite_transforms_are_rejected() {
    let ordinary = curve(CurveType::CatmullRom, &[anchor(0.0, 0.0), anchor(1.0, 1.0)]);
    for resolution in [0, 1, MAX_RESOLUTION + 1] {
        assert_eq!(
            sample_curve_v2(&ordinary, resolution, 256, false),
            Err(SplineError::InvalidSamplingResolution(resolution))
        );
        assert_eq!(
            sample_curve_v2(&ordinary, 256, resolution, false),
            Err(SplineError::InvalidOutputResolution(resolution))
        );
    }
    assert_eq!(
        interpolate_value_v2_periodic(&[anchor(0.25, 0.5)], 0.0, CurveType::CatmullRom, 0.0,),
        Err(SplineError::InvalidPeriod)
    );

    let zero_period_bounds = CurveBounds::new(1.0, 1.0, 0.0, 1.0).expect("finite equal x bounds");
    let zero_period = Curve::new(
        CurveType::CatmullRom,
        zero_period_bounds,
        &[anchor(0.25, 0.5)],
    )
    .expect("finite zero-period curve");
    assert_eq!(
        sample_curve_v2(&zero_period, 9, 256, true),
        Err(SplineError::InvalidPeriod)
    );

    let huge_bounds = CurveBounds::new(0.0, f32::MAX, 0.0, 1.0).expect("finite huge bounds");
    let overflow = Curve::new(CurveType::CatmullRom, huge_bounds, &[anchor(2.0, 0.5)])
        .expect("finite raw overflow curve");
    assert_eq!(
        sample_curve_v2(&overflow, 9, 256, false),
        Err(SplineError::NonFiniteResult)
    );
}
