#![expect(
    clippy::suboptimal_flops,
    reason = "Native Color Zones arithmetic order is preserved for IEEE-754 parity."
)]

//! Color Zones LUT commitment ported from `src/iop/colorzones.c`.
//!
//! Curve interpolation and quantization are delegated to the direct
//! `src/common/curve_tools.c` and `src/common/splines.cpp` ports.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "the native lookup contract explicitly truncates a scaled f32 to a bounded integer bin"
)]

use std::fmt;
use std::sync::Arc;

use crate::common::curve_tools::{
    Curve, CurveAnchor, CurveBounds, CurveError, CurveType, MAX_ANCHORS, sample_curve_v1,
};
use crate::common::splines::{SplineError, sample_curve_v2};

use super::{
    COLORZONES_CHANNELS, ColorZonesChannel, ColorZonesConfig, ColorZonesCurveType,
    ColorZonesSplinesVersion,
};

/// Native Color Zones lookup-table resolution.
pub const COLORZONES_LUT_RESOLUTION: usize = 0x1_0000;

const COLORZONES_LUT_RESOLUTION_U32: u32 = 0x1_0000;
const COLORZONES_LUT_SCALE: f32 = 1.0 / 65_536.0;
const CURVE_CHANNELS: [ColorZonesChannel; COLORZONES_CHANNELS] = [
    ColorZonesChannel::Lightness,
    ColorZonesChannel::Chroma,
    ColorZonesChannel::Hue,
];

pub(super) type ColorZonesLuts = Arc<[f32]>;

/// Failure to commit checked Color Zones parameters into native LUTs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorZonesCompileError {
    /// Spline v1 adds two compatibility anchors to the active persisted
    /// prefix, which cannot exceed the native 20-anchor sampler capacity.
    LegacyAnchorCapacityExceeded {
        curve: ColorZonesChannel,
        active_nodes: usize,
        required_anchors: usize,
        maximum_anchors: usize,
    },
    Curve {
        curve: ColorZonesChannel,
        source: CurveError,
    },
    Spline {
        curve: ColorZonesChannel,
        source: SplineError,
    },
    UnexpectedSampleCount {
        curve: ColorZonesChannel,
        expected: usize,
        actual: usize,
    },
    AllocationFailed {
        required_bytes: usize,
    },
}

impl fmt::Display for ColorZonesCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LegacyAnchorCapacityExceeded {
                curve,
                active_nodes,
                required_anchors,
                maximum_anchors,
            } => write!(
                formatter,
                "Color Zones {curve:?} spline-v1 curve has {active_nodes} active nodes and requires {required_anchors} anchors; native capacity is {maximum_anchors}"
            ),
            Self::Curve { curve, source } => {
                write!(
                    formatter,
                    "Color Zones {curve:?} curve is invalid: {source}"
                )
            }
            Self::Spline { curve, source } => {
                write!(
                    formatter,
                    "Color Zones {curve:?} spline sampling failed: {source}"
                )
            }
            Self::UnexpectedSampleCount {
                curve,
                expected,
                actual,
            } => write!(
                formatter,
                "Color Zones {curve:?} sampler returned {actual} values; expected {expected}"
            ),
            Self::AllocationFailed { required_bytes } => write!(
                formatter,
                "Color Zones LUT allocation of {required_bytes} bytes failed"
            ),
        }
    }
}

impl std::error::Error for ColorZonesCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Curve { source, .. } => Some(source),
            Self::Spline { source, .. } => Some(source),
            Self::LegacyAnchorCapacityExceeded { .. }
            | Self::UnexpectedSampleCount { .. }
            | Self::AllocationFailed { .. } => None,
        }
    }
}

pub(super) fn compile_luts(
    config: &ColorZonesConfig,
) -> Result<ColorZonesLuts, ColorZonesCompileError> {
    let total_values = COLORZONES_CHANNELS * COLORZONES_LUT_RESOLUTION;
    let required_bytes = total_values * size_of::<f32>();
    let mut committed = Vec::new();
    committed
        .try_reserve_exact(total_values)
        .map_err(|_| ColorZonesCompileError::AllocationFailed { required_bytes })?;

    for curve_channel in CURVE_CHANNELS {
        let samples = compile_curve(config, curve_channel)?;
        if samples.len() != COLORZONES_LUT_RESOLUTION {
            return Err(ColorZonesCompileError::UnexpectedSampleCount {
                curve: curve_channel,
                expected: COLORZONES_LUT_RESOLUTION,
                actual: samples.len(),
            });
        }
        committed.extend(
            samples
                .into_iter()
                .map(|sample| f32::from(sample) * COLORZONES_LUT_SCALE),
        );
    }

    debug_assert_eq!(committed.len(), total_values);
    Ok(Arc::from(committed))
}

fn compile_curve(
    config: &ColorZonesConfig,
    curve_channel: ColorZonesChannel,
) -> Result<Vec<u16>, ColorZonesCompileError> {
    let curve = config.curve(curve_channel);
    let curve_type = native_curve_type(curve.curve_type());
    match config.splines_version() {
        ColorZonesSplinesVersion::V1 => {
            let active_nodes = curve.node_count();
            let required_anchors = active_nodes + 2;
            if required_anchors > MAX_ANCHORS {
                return Err(ColorZonesCompileError::LegacyAnchorCapacityExceeded {
                    curve: curve_channel,
                    active_nodes,
                    required_anchors,
                    maximum_anchors: MAX_ANCHORS,
                });
            }
            let anchors = legacy_v1_anchors(config, curve_channel);
            let committed_curve =
                Curve::new(curve_type, CurveBounds::unit(), &anchors).map_err(|source| {
                    ColorZonesCompileError::Curve {
                        curve: curve_channel,
                        source,
                    }
                })?;
            sample_curve_v1(
                &committed_curve,
                COLORZONES_LUT_RESOLUTION_U32,
                COLORZONES_LUT_RESOLUTION_U32,
            )
            .map_err(|source| ColorZonesCompileError::Curve {
                curve: curve_channel,
                source,
            })
        }
        ColorZonesSplinesVersion::V2 => {
            let anchors: Vec<_> = curve
                .points()
                .iter()
                .copied()
                .map(|point| {
                    CurveAnchor::new(point.x(), apply_strength(point.y(), config.strength()))
                })
                .collect();
            let committed_curve =
                Curve::new(curve_type, CurveBounds::unit(), &anchors).map_err(|source| {
                    ColorZonesCompileError::Curve {
                        curve: curve_channel,
                        source,
                    }
                })?;
            sample_curve_v2(
                &committed_curve,
                COLORZONES_LUT_RESOLUTION_U32,
                COLORZONES_LUT_RESOLUTION_U32,
                config.channel() == ColorZonesChannel::Hue,
            )
            .map_err(|source| ColorZonesCompileError::Spline {
                curve: curve_channel,
                source,
            })
        }
    }
}

fn legacy_v1_anchors(
    config: &ColorZonesConfig,
    curve_channel: ColorZonesChannel,
) -> Vec<CurveAnchor> {
    let points = config.curve(curve_channel).points();
    let strength = config.strength();
    let periodic = config.channel() == ColorZonesChannel::Hue;
    let penultimate = points[points.len() - 2];
    let first = points[0];
    let second = points[1];
    let last = points[points.len() - 1];

    let mut anchors = Vec::with_capacity(points.len() + 2);
    let leading_y = if periodic { penultimate.y() } else { first.y() };
    anchors.push(CurveAnchor::new(
        penultimate.x() - 1.0,
        apply_strength(leading_y, strength),
    ));
    anchors.extend(
        points
            .iter()
            .copied()
            .map(|point| CurveAnchor::new(point.x(), apply_strength(point.y(), strength))),
    );
    let trailing_y = if periodic { second.y() } else { last.y() };
    anchors.push(CurveAnchor::new(
        second.x() + 1.0,
        apply_strength(trailing_y, strength),
    ));
    anchors
}

const fn native_curve_type(curve_type: ColorZonesCurveType) -> CurveType {
    match curve_type {
        ColorZonesCurveType::Cubic => CurveType::CubicSpline,
        ColorZonesCurveType::Catmull => CurveType::CatmullRom,
        ColorZonesCurveType::Monotone => CurveType::MonotoneHermite,
    }
}

fn apply_strength(value: f32, strength: f32) -> f32 {
    value + (value - 0.5) * (strength / 100.0)
}

/// Exact scalar interpolation and comparison order from native `lookup`.
pub(super) fn lookup(lut: &[f32], input: f32) -> f32 {
    debug_assert_eq!(lut.len(), COLORZONES_LUT_RESOLUTION);
    let scaled = 65_536.0 * input;
    // Native C leaves non-finite and out-of-i32 conversion undefined. Rust's
    // saturating float cast plus saturating increment makes those inputs
    // deterministic without changing any native-defined lookup.
    let native_bin = scaled as i32;
    let bin0 = native_bin.clamp(0, 0xffff);
    let bin1 = native_bin.saturating_add(1).clamp(0, 0xffff);
    let fraction = scaled - bin0 as f32;
    lut[bin1 as usize] * fraction + lut[bin0 as usize] * (1.0 - fraction)
}

#[cfg(test)]
mod tests {
    use super::{COLORZONES_LUT_RESOLUTION, legacy_v1_anchors, lookup};
    use crate::operations::colorzones::{
        ColorZonesChannel, ColorZonesConfig, ColorZonesNode, ColorZonesParametersV5,
        ColorZonesSplinesVersion,
    };

    #[test]
    fn lookup_retains_native_interpolation_and_edge_rules() {
        let mut lut = vec![0.0; COLORZONES_LUT_RESOLUTION];
        lut[1] = 1.0;
        lut[COLORZONES_LUT_RESOLUTION - 1] = 0.75;

        assert_eq!(lookup(&lut, 0.5 / 65_536.0).to_bits(), 0.5_f32.to_bits());
        assert_eq!(
            lookup(&lut, -0.5 / 65_536.0).to_bits(),
            (-0.5_f32).to_bits()
        );
        assert_eq!(lookup(&lut, 1.0).to_bits(), 0.75_f32.to_bits());
        assert_eq!(lookup(&lut, 2.0).to_bits(), 0.75_f32.to_bits());
    }

    #[test]
    fn legacy_commit_extends_periodic_and_nonperiodic_curves_exactly() {
        let mut parameters = ColorZonesParametersV5::defaults();
        parameters.splines_version = ColorZonesSplinesVersion::V1.raw();
        parameters.curve_num_nodes = [3, 3, 3];
        parameters.strength = 100.0;
        for curve in 0..3 {
            parameters.curves[curve][0] = ColorZonesNode::new(0.0, 0.1);
            parameters.curves[curve][1] = ColorZonesNode::new(0.5, 0.7);
            parameters.curves[curve][2] = ColorZonesNode::new(1.0, 0.2);
        }
        let adjusted = |value: f32| value + (value - 0.5) * (100.0 / 100.0);

        parameters.channel = ColorZonesChannel::Lightness.raw();
        let nonperiodic =
            ColorZonesConfig::try_from(parameters).expect("checked nonperiodic config");
        let nonperiodic = legacy_v1_anchors(&nonperiodic, ColorZonesChannel::Lightness);
        let expected_x = [-0.5_f32, 0.0, 0.5, 1.0, 1.5];
        let expected_y = [
            adjusted(0.1),
            adjusted(0.1),
            adjusted(0.7),
            adjusted(0.2),
            adjusted(0.2),
        ];
        assert_eq!(nonperiodic.len(), expected_x.len());
        for ((anchor, expected_x), expected_y) in nonperiodic.iter().zip(expected_x).zip(expected_y)
        {
            assert_eq!(anchor.x().to_bits(), expected_x.to_bits());
            assert_eq!(anchor.y().to_bits(), expected_y.to_bits());
        }

        parameters.channel = ColorZonesChannel::Hue.raw();
        let periodic = ColorZonesConfig::try_from(parameters).expect("checked periodic config");
        let periodic = legacy_v1_anchors(&periodic, ColorZonesChannel::Lightness);
        let expected_y = [
            adjusted(0.7),
            adjusted(0.1),
            adjusted(0.7),
            adjusted(0.2),
            adjusted(0.7),
        ];
        assert_eq!(periodic.len(), expected_x.len());
        for ((anchor, expected_x), expected_y) in periodic.iter().zip(expected_x).zip(expected_y) {
            assert_eq!(anchor.x().to_bits(), expected_x.to_bits());
            assert_eq!(anchor.y().to_bits(), expected_y.to_bits());
        }
    }
}
