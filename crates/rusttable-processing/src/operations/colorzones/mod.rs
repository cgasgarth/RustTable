//! Darktable-compatible Color Zones v1-v5 history and checked parameters.
//!
//! The byte layouts, direct legacy migrations, enum tags, defaults, and active
//! curve-node contract are derived from `src/iop/colorzones.c`. This slice does
//! not claim the native curve evaluator or pixel-processing implementation.

#![forbid(unsafe_code)]
#![allow(
    clippy::large_types_passed_by_value,
    clippy::missing_errors_doc,
    clippy::trivially_copy_pass_by_ref,
    reason = "the source-shaped raw codec keeps explicit by-value migrations and uniform borrowed encoders"
)]

mod codec;

use std::fmt;

use crate::FiniteF32;

pub use codec::{
    COLORZONES_CHANNELS, COLORZONES_COMPATIBILITY_ID, COLORZONES_LEGACY_BANDS,
    COLORZONES_MAX_NODES, COLORZONES_RUST_ID, COLORZONES_SCHEMA_VERSION, COLORZONES_V1_BANDS,
    COLORZONES_V1_PARAMETER_BYTES, COLORZONES_V2_PARAMETER_BYTES, COLORZONES_V3_PARAMETER_BYTES,
    COLORZONES_V4_PARAMETER_BYTES, COLORZONES_V5_PARAMETER_BYTES, ColorZonesCodecError,
    ColorZonesHistory, ColorZonesNode, ColorZonesParametersV1, ColorZonesParametersV2,
    ColorZonesParametersV3, ColorZonesParametersV4, ColorZonesParametersV5, migrate_v1_to_v5,
    migrate_v2_to_v5, migrate_v3_to_v5, migrate_v4_to_v5,
};

/// Native selection channel stored in Color Zones history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesChannel {
    Lightness = 0,
    Chroma = 1,
    Hue = 2,
}

impl ColorZonesChannel {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Lightness),
            1 => Some(Self::Chroma),
            2 => Some(Self::Hue),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::Lightness => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Lightness => 0,
            Self::Chroma => 1,
            Self::Hue => 2,
        }
    }
}

/// Native point-processing branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesMode {
    Smooth = 0,
    Strong = 1,
}

impl ColorZonesMode {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Smooth),
            1 => Some(Self::Strong),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::Smooth => 0,
            Self::Strong => 1,
        }
    }
}

/// Native curve interpolation tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesCurveType {
    Cubic = 0,
    Catmull = 1,
    Monotone = 2,
}

impl ColorZonesCurveType {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::Cubic),
            1 => Some(Self::Catmull),
            2 => Some(Self::Monotone),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::Cubic => 0,
            Self::Catmull => 1,
            Self::Monotone => 2,
        }
    }
}

/// Native curve-boundary implementation version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorZonesSplinesVersion {
    V1 = 0,
    V2 = 1,
}

impl ColorZonesSplinesVersion {
    #[must_use]
    pub const fn from_raw(value: i32) -> Option<Self> {
        match value {
            0 => Some(Self::V1),
            1 => Some(Self::V2),
            _ => None,
        }
    }

    #[must_use]
    pub const fn raw(self) -> i32 {
        match self {
            Self::V1 => 0,
            Self::V2 => 1,
        }
    }
}

/// One finite active curve point.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColorZonesPoint {
    x: FiniteF32,
    y: FiniteF32,
}

impl ColorZonesPoint {
    #[must_use]
    pub const fn x(self) -> f32 {
        self.x.get()
    }

    #[must_use]
    pub const fn y(self) -> f32 {
        self.y.get()
    }
}

/// One checked curve containing only the active native prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColorZonesCurve {
    curve_type: ColorZonesCurveType,
    points: Vec<ColorZonesPoint>,
}

impl ColorZonesCurve {
    #[must_use]
    pub const fn curve_type(&self) -> ColorZonesCurveType {
        self.curve_type
    }

    #[must_use]
    pub fn points(&self) -> &[ColorZonesPoint] {
        &self.points
    }

    #[must_use]
    pub const fn node_count(&self) -> usize {
        self.points.len()
    }
}

/// Checked Color Zones parameters ready for later curve compilation.
///
/// Native history is not clamped or sorted here. Only active nodes are
/// inspected; inactive v5 tail storage is deliberately ignored.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ColorZonesConfig {
    channel: ColorZonesChannel,
    curves: [ColorZonesCurve; COLORZONES_CHANNELS],
    strength: FiniteF32,
    mode: ColorZonesMode,
    splines_version: ColorZonesSplinesVersion,
}

impl ColorZonesConfig {
    /// Returns Darktable's checked native v5 defaults.
    ///
    /// # Panics
    ///
    /// Panics if the source-derived raw defaults stop satisfying this module's
    /// semantic validation contract.
    #[must_use]
    pub fn defaults() -> Self {
        Self::try_from(&ColorZonesParametersV5::defaults())
            .expect("native Color Zones defaults are valid")
    }

    #[must_use]
    pub const fn channel(&self) -> ColorZonesChannel {
        self.channel
    }

    #[must_use]
    pub const fn curves(&self) -> &[ColorZonesCurve; COLORZONES_CHANNELS] {
        &self.curves
    }

    #[must_use]
    pub const fn curve(&self, channel: ColorZonesChannel) -> &ColorZonesCurve {
        &self.curves[channel.index()]
    }

    #[must_use]
    pub const fn strength(&self) -> f32 {
        self.strength.get()
    }

    #[must_use]
    pub const fn mode(&self) -> ColorZonesMode {
        self.mode
    }

    #[must_use]
    pub const fn splines_version(&self) -> ColorZonesSplinesVersion {
        self.splines_version
    }
}

impl TryFrom<&ColorZonesParametersV5> for ColorZonesConfig {
    type Error = ColorZonesParameterError;

    fn try_from(parameters: &ColorZonesParametersV5) -> Result<Self, Self::Error> {
        let channel = ColorZonesChannel::from_raw(parameters.channel).ok_or(
            ColorZonesParameterError::InvalidEnum {
                parameter: "channel",
                value: parameters.channel,
            },
        )?;
        let mode = ColorZonesMode::from_raw(parameters.mode).ok_or(
            ColorZonesParameterError::InvalidEnum {
                parameter: "mode",
                value: parameters.mode,
            },
        )?;
        let splines_version = ColorZonesSplinesVersion::from_raw(parameters.splines_version)
            .ok_or(ColorZonesParameterError::InvalidEnum {
                parameter: "splines_version",
                value: parameters.splines_version,
            })?;
        let strength = FiniteF32::new(parameters.strength)
            .map_err(|_| ColorZonesParameterError::NonFiniteStrength)?;

        let mut curves = Vec::with_capacity(COLORZONES_CHANNELS);
        for curve_channel in 0..COLORZONES_CHANNELS {
            let count = parameters.curve_num_nodes[curve_channel];
            let minimum_node_count = match splines_version {
                ColorZonesSplinesVersion::V1 => 2,
                ColorZonesSplinesVersion::V2 => 1,
            };
            if !(minimum_node_count
                ..=i32::try_from(COLORZONES_MAX_NODES).expect("node limit fits i32"))
                .contains(&count)
            {
                return Err(ColorZonesParameterError::InvalidNodeCount {
                    channel: curve_channel,
                    count,
                });
            }
            let curve_type = ColorZonesCurveType::from_raw(parameters.curve_type[curve_channel])
                .ok_or(ColorZonesParameterError::InvalidCurveType {
                    channel: curve_channel,
                    value: parameters.curve_type[curve_channel],
                })?;
            let active = usize::try_from(count).expect("validated node count is positive");
            let mut points = Vec::with_capacity(active);
            for node in 0..active {
                let raw = parameters.curves[curve_channel][node];
                let x = FiniteF32::new(raw.x).map_err(|_| {
                    ColorZonesParameterError::NonFiniteActiveNode {
                        channel: curve_channel,
                        node,
                        coordinate: "x",
                    }
                })?;
                let y = FiniteF32::new(raw.y).map_err(|_| {
                    ColorZonesParameterError::NonFiniteActiveNode {
                        channel: curve_channel,
                        node,
                        coordinate: "y",
                    }
                })?;
                points.push(ColorZonesPoint { x, y });
            }
            curves.push(ColorZonesCurve { curve_type, points });
        }
        let curves = <[ColorZonesCurve; COLORZONES_CHANNELS]>::try_from(curves)
            .expect("one checked curve is built for every native channel");
        Ok(Self {
            channel,
            curves,
            strength,
            mode,
            splines_version,
        })
    }
}

impl TryFrom<ColorZonesParametersV5> for ColorZonesConfig {
    type Error = ColorZonesParameterError;

    fn try_from(parameters: ColorZonesParametersV5) -> Result<Self, Self::Error> {
        Self::try_from(&parameters)
    }
}

/// Invalid semantic values in an otherwise losslessly decoded v5 payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorZonesParameterError {
    InvalidEnum {
        parameter: &'static str,
        value: i32,
    },
    InvalidNodeCount {
        channel: usize,
        count: i32,
    },
    InvalidCurveType {
        channel: usize,
        value: i32,
    },
    NonFiniteStrength,
    NonFiniteActiveNode {
        channel: usize,
        node: usize,
        coordinate: &'static str,
    },
}

impl fmt::Display for ColorZonesParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidEnum { parameter, value } => {
                write!(formatter, "Color Zones {parameter} tag {value} is invalid")
            }
            Self::InvalidNodeCount { channel, count } => write!(
                formatter,
                "Color Zones curve {channel} has {count} active nodes; expected 2..={COLORZONES_MAX_NODES} for spline v1 or 1..={COLORZONES_MAX_NODES} for spline v2"
            ),
            Self::InvalidCurveType { channel, value } => write!(
                formatter,
                "Color Zones curve {channel} interpolation tag {value} is invalid"
            ),
            Self::NonFiniteStrength => formatter.write_str("Color Zones strength is non-finite"),
            Self::NonFiniteActiveNode {
                channel,
                node,
                coordinate,
            } => write!(
                formatter,
                "Color Zones curve {channel} active node {node} {coordinate} coordinate is non-finite"
            ),
        }
    }
}

impl std::error::Error for ColorZonesParameterError {}
