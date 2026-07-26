//! Source-derived contract for Darktable's retained
//! `src/common/curve_tools.c`, `src/common/curve_tools.h`, and V1 sampling
//! wrappers in `src/gui/draw.h`.

#![allow(
    clippy::float_cmp,
    reason = "the source-derived vectors intentionally assert exact f32 and u16 results"
)]

use rusttable_processing::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, MAX_ANCHORS, MAX_RESOLUTION,
    interpolate_value_v1, sample_curve_v1,
};

fn anchor(x: u32, y: u32) -> CurveAnchor {
    CurveAnchor::new(f32::from_bits(x), f32::from_bits(y))
}

fn unit_curve(curve_type: CurveType, anchors: &[CurveAnchor]) -> Curve {
    Curve::new(curve_type, CurveBounds::unit(), anchors).expect("valid unit curve")
}

fn standard_anchors() -> [CurveAnchor; 5] {
    [
        anchor(0x0000_0000, 0x3dcc_cccd),
        anchor(0x3e4c_cccd, 0x3f59_999a),
        anchor(0x3f0c_cccd, 0x3ea3_d70a),
        anchor(0x3f51_eb85, 0x3f70_a3d7),
        anchor(0x3f80_0000, 0x3f19_999a),
    ]
}

#[test]
fn construction_is_bounded_and_finite_but_preserves_source_order_and_range() {
    assert_eq!(
        CurveBounds::new(0.0, f32::INFINITY, 0.0, 1.0),
        Err(CurveError::NonFiniteBound { bound: "max_x" })
    );

    let non_finite = [CurveAnchor::new(0.0, 0.0), CurveAnchor::new(1.0, f32::NAN)];
    assert_eq!(
        Curve::new(CurveType::CatmullRom, CurveBounds::unit(), &non_finite),
        Err(CurveError::NonFiniteAnchor {
            index: 1,
            coordinate: "y",
        })
    );

    let full = vec![CurveAnchor::new(0.0, 0.5); MAX_ANCHORS];
    assert_eq!(
        Curve::new(CurveType::CatmullRom, CurveBounds::unit(), &full)
            .expect("fixed-capacity curve")
            .anchors()
            .len(),
        MAX_ANCHORS
    );

    let excessive = vec![CurveAnchor::new(0.0, 0.5); MAX_ANCHORS + 1];
    assert_eq!(
        Curve::new(CurveType::CatmullRom, CurveBounds::unit(), &excessive),
        Err(CurveError::TooManyAnchors {
            count: MAX_ANCHORS + 1,
            maximum: MAX_ANCHORS,
        })
    );

    let retained = [CurveAnchor::new(1.25, -0.5), CurveAnchor::new(-0.25, 1.5)];
    let curve = unit_curve(CurveType::CatmullRom, &retained);
    assert_eq!(curve.anchors(), retained);
}

#[test]
fn v1_validation_is_deferred_until_interpolation_or_sampling() {
    assert_eq!(
        interpolate_value_v1(&[], 0.5, CurveType::CatmullRom),
        Err(CurveError::TooFewAnchors {
            count: 0,
            minimum: 2,
        })
    );
    assert_eq!(
        interpolate_value_v1(&[CurveAnchor::new(0.5, 0.0)], 0.5, CurveType::CatmullRom),
        Err(CurveError::TooFewAnchors {
            count: 1,
            minimum: 2,
        })
    );
    assert_eq!(
        interpolate_value_v1(
            &[CurveAnchor::new(0.75, 0.0), CurveAnchor::new(0.25, 1.0),],
            0.5,
            CurveType::CatmullRom,
        ),
        Err(CurveError::NonIncreasingAnchors { left: 0, right: 1 })
    );
    assert_eq!(
        interpolate_value_v1(
            &[CurveAnchor::new(0.0, 0.0), CurveAnchor::new(1.0, 1.0),],
            f32::NAN,
            CurveType::CubicSpline,
        ),
        Err(CurveError::NonFiniteEvaluationInput)
    );

    let descending = unit_curve(
        CurveType::MonotoneHermite,
        &[CurveAnchor::new(0.75, 0.0), CurveAnchor::new(0.25, 1.0)],
    );
    assert_eq!(
        sample_curve_v1(&descending, 9, 256),
        Err(CurveError::NonIncreasingAnchors { left: 0, right: 1 })
    );
}

#[test]
fn direct_interpolation_matches_source_bit_patterns_for_all_curve_types() {
    let anchors = standard_anchors();
    let x = f32::from_bits(0x3ee0_0000);
    for (curve_type, expected) in [
        (CurveType::CubicSpline, 0x3ee2_d418),
        (CurveType::CatmullRom, 0x3ee7_1996),
        (CurveType::MonotoneHermite, 0x3ee9_9923),
    ] {
        let actual =
            interpolate_value_v1(&anchors, x, curve_type).expect("source-valid interpolation");
        assert_eq!(actual.to_bits(), expected, "{curve_type:?}");
    }
}

#[test]
fn unsuffixed_c_literals_keep_cubic_double_grouping_until_final_narrowing() {
    let anchors = [
        anchor(0x0000_0000, 0x3eab_0315),
        anchor(0x3e05_aa8b, 0xbe4e_0cbb),
        anchor(0x3ef8_814d, 0x3dab_c37f),
        anchor(0x3f50_69e5, 0x3f98_eb8c),
        anchor(0x3f80_0000, 0x3ef3_4cd0),
    ];
    let actual = interpolate_value_v1(
        &anchors,
        f32::from_bits(0x3f88_dd8c),
        CurveType::CubicSpline,
    )
    .expect("finite cubic extrapolation");
    assert_eq!(actual.to_bits(), 0x3df4_63aa);
    assert_ne!(
        actual.to_bits(),
        0x3df4_63b0,
        "all-f32 regrouping changes the retained result"
    );
}

#[test]
fn unsuffixed_c_literals_narrow_each_catmull_basis_before_accumulation() {
    let anchors = [
        anchor(0x0000_0000, 0x3f24_7828),
        anchor(0x3db9_cf27, 0x3e1d_f581),
        anchor(0x3efe_a96e, 0x3f92_0ea4),
        anchor(0x3f3c_0576, 0x3e4c_025c),
        anchor(0x3f80_0000, 0x3f68_dded),
    ];
    let actual = interpolate_value_v1(&anchors, f32::from_bits(0x3ec9_a5ef), CurveType::CatmullRom)
        .expect("finite Catmull interpolation");
    assert_eq!(actual.to_bits(), 0x3f7f_55fd);
    assert_ne!(
        actual.to_bits(),
        0x3f7f_55fb,
        "all-f32 basis evaluation changes the retained result"
    );
}

#[test]
fn sampled_luts_match_source_quantization_for_all_curve_types() {
    let anchors = standard_anchors();
    for (curve_type, expected) in [
        (
            CurveType::CubicSpline,
            [
                0x001a, 0x00ac, 0x00da, 0x009a, 0x0055, 0x0074, 0x00d3, 0x00e9, 0x0099,
            ],
        ),
        (
            CurveType::CatmullRom,
            [
                0x001a, 0x00aa, 0x00d5, 0x0098, 0x0058, 0x006e, 0x00d0, 0x00e6, 0x0099,
            ],
        ),
        (
            CurveType::MonotoneHermite,
            [
                0x001a, 0x00a5, 0x00db, 0x009d, 0x0057, 0x0073, 0x00d5, 0x00e3, 0x0099,
            ],
        ),
    ] {
        let curve = unit_curve(curve_type, &anchors);
        assert_eq!(
            sample_curve_v1(&curve, 9, 256).expect("source-valid LUT"),
            expected,
            "{curve_type:?}"
        );
    }
}

#[test]
fn endpoint_extension_truncates_but_interior_quantization_adds_one_half() {
    let curve = unit_curve(
        CurveType::CatmullRom,
        &[
            anchor(0x3e4c_cccd, 0x3f00_0000),
            anchor(0x3f4c_cccd, 0x3f00_0000),
        ],
    );
    assert_eq!(
        sample_curve_v1(&curve, 11, 65_536).expect("constant sampled curve"),
        [
            0x7fff, 0x7fff, 0x8000, 0x8000, 0x8000, 0x8000, 0x8000, 0x8000, 0x8000, 0x7fff, 0x7fff,
        ]
    );
}

#[test]
fn zero_anchors_expand_to_the_bounds_diagonal() {
    let curve = unit_curve(CurveType::CubicSpline, &[]);
    assert_eq!(
        sample_curve_v1(&curve, 5, 256).expect("implicit diagonal"),
        [0x0000, 0x0040, 0x0080, 0x00bf, 0x00ff]
    );
}

#[test]
fn monotone_zero_slopes_flatten_both_sides_of_each_flat_segment() {
    let anchors = [
        anchor(0x0000_0000, 0x3e4c_cccd),
        anchor(0x3e80_0000, 0x3e4c_cccd),
        anchor(0x3f00_0000, 0x3f4c_cccd),
        anchor(0x3f40_0000, 0x3f4c_cccd),
        anchor(0x3f80_0000, 0x3f4c_cccd),
    ];
    let midpoint = interpolate_value_v1(
        &anchors,
        f32::from_bits(0x3ec0_0000),
        CurveType::MonotoneHermite,
    )
    .expect("monotone flat-segment interpolation");
    assert_eq!(midpoint.to_bits(), 0x3f00_0000);

    let curve = unit_curve(CurveType::MonotoneHermite, &anchors);
    assert_eq!(
        sample_curve_v1(&curve, 9, 256).expect("flat-segment LUT"),
        [
            0x0033, 0x0033, 0x0033, 0x0080, 0x00cc, 0x00cc, 0x00cc, 0x00cc, 0x00cc,
        ]
    );
}

#[test]
fn resolution_limits_return_typed_errors() {
    let curve = unit_curve(
        CurveType::CubicSpline,
        &[CurveAnchor::new(0.0, 0.0), CurveAnchor::new(1.0, 1.0)],
    );
    for resolution in [0, 1, MAX_RESOLUTION + 1] {
        assert_eq!(
            sample_curve_v1(&curve, resolution, 256),
            Err(CurveError::InvalidSamplingResolution { resolution })
        );
        assert_eq!(
            sample_curve_v1(&curve, 256, resolution),
            Err(CurveError::InvalidOutputResolution { resolution })
        );
    }
    assert_eq!(
        sample_curve_v1(&curve, 2, 2).expect("minimum resolutions"),
        [0, 1]
    );

    let maximum =
        sample_curve_v1(&curve, MAX_RESOLUTION, MAX_RESOLUTION).expect("maximum legacy LUT");
    assert_eq!(maximum.len(), 65_536);
    assert_eq!(maximum[0], 0);
    assert_eq!(maximum[32_768], 32_768);
    assert_eq!(maximum[65_535], 65_535);
}
