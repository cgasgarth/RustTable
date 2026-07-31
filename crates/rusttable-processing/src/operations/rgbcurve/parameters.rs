//! Parameter contracts ported from Darktable `src/iop/rgbcurve.c`.
//!
//! The native declaration order is intentionally represented by the field order
//! in [`RgbCurveParametersV1`]. Inactive node slots are retained because the
//! native history payload serializes all 20 slots for every channel.

#![forbid(unsafe_code)]

use std::fmt;

pub const CHANNELS: usize = 3;
pub const MAX_NODES: usize = 20;
pub const PARAMETER_VERSION: u16 = 1;
pub const PARAMETER_BYTES: usize = 516;
pub const MIN_X_DISTANCE: f32 = 0.0025;
pub const LUT_RESOLUTION: u32 = 0x1_0000;

/// Native RGB channel IDs from `rgbcurve_channel_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u32)]
pub enum RgbCurveChannel {
    Red = 0,
    Green = 1,
    Blue = 2,
}

impl RgbCurveChannel {
    pub const ALL: [Self; CHANNELS] = [Self::Red, Self::Green, Self::Blue];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Native interpolation IDs from `curve_tools.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RgbCurveType {
    CubicSpline = 0,
    CatmullRom = 1,
    MonotoneHermite = 2,
}

impl TryFrom<i32> for RgbCurveType {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::CubicSpline),
            1 => Ok(Self::CatmullRom),
            2 => Ok(Self::MonotoneHermite),
            other => Err(other),
        }
    }
}

/// Native `curve_autoscale` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum RgbCurveAutoscale {
    AutomaticRgb = 0,
    ManualRgb = 1,
}

impl TryFrom<i32> for RgbCurveAutoscale {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::AutomaticRgb),
            1 => Ok(Self::ManualRgb),
            other => Err(other),
        }
    }
}

/// Native `dt_iop_rgb_norms_t` values used for automatic color preservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum PreserveColors {
    None = 0,
    Luminance = 1,
    Max = 2,
    Average = 3,
    Sum = 4,
    Norm = 5,
    Power = 6,
}

impl TryFrom<i32> for PreserveColors {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Luminance),
            2 => Ok(Self::Max),
            3 => Ok(Self::Average),
            4 => Ok(Self::Sum),
            5 => Ok(Self::Norm),
            6 => Ok(Self::Power),
            other => Err(other),
        }
    }
}

/// One native `dt_iop_rgbcurve_node_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RgbCurveNode {
    pub x: f32,
    pub y: f32,
}

impl RgbCurveNode {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Checked version-1 parameter state in native declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct RgbCurveParametersV1 {
    pub curve_nodes: [[RgbCurveNode; MAX_NODES]; CHANNELS],
    pub curve_num_nodes: [u32; CHANNELS],
    pub curve_type: [RgbCurveType; CHANNELS],
    pub curve_autoscale: RgbCurveAutoscale,
    pub compensate_middle_grey: bool,
    pub preserve_colors: PreserveColors,
}

impl Default for RgbCurveParametersV1 {
    fn default() -> Self {
        let mut curve_nodes = [[RgbCurveNode::ZERO; MAX_NODES]; CHANNELS];
        for channel in &mut curve_nodes {
            channel[1] = RgbCurveNode::new(1.0, 1.0);
        }
        Self {
            curve_nodes,
            curve_num_nodes: [2; CHANNELS],
            curve_type: [RgbCurveType::MonotoneHermite; CHANNELS],
            curve_autoscale: RgbCurveAutoscale::AutomaticRgb,
            compensate_middle_grey: false,
            preserve_colors: PreserveColors::Luminance,
        }
    }
}

impl RgbCurveParametersV1 {
    /// Returns the exact native defaults after `init()` completes.
    #[must_use]
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Validates all active values without rewriting inactive serialized slots.
    pub fn validate(&self) -> Result<(), ParameterError> {
        for channel in 0..CHANNELS {
            let count = usize::try_from(self.curve_num_nodes[channel]).map_err(|_| {
                ParameterError::InvalidNodeCount {
                    channel,
                    value: i64::from(self.curve_num_nodes[channel]),
                }
            })?;
            if !(2..=MAX_NODES).contains(&count) {
                return Err(ParameterError::InvalidNodeCount {
                    channel,
                    value: i64::from(self.curve_num_nodes[channel]),
                });
            }
            let mut previous_x = None;
            for node_index in 0..count {
                let node = self.curve_nodes[channel][node_index];
                if !node.x.is_finite() {
                    return Err(ParameterError::NonFinite {
                        channel,
                        node: node_index,
                        coordinate: "x",
                    });
                }
                if !node.y.is_finite() {
                    return Err(ParameterError::NonFinite {
                        channel,
                        node: node_index,
                        coordinate: "y",
                    });
                }
                if let Some(previous_x) = previous_x {
                    if node.x <= previous_x {
                        return Err(ParameterError::NonIncreasingNodes {
                            channel,
                            left: node_index - 1,
                            right: node_index,
                        });
                    }
                }
                previous_x = Some(node.x);
            }
        }
        Ok(())
    }

    pub(crate) fn from_raw(
        curve_nodes: [[RgbCurveNode; MAX_NODES]; CHANNELS],
        curve_num_nodes: [i32; CHANNELS],
        curve_type: [i32; CHANNELS],
        curve_autoscale: i32,
        compensate_middle_grey: i32,
        preserve_colors: i32,
    ) -> Result<Self, ParameterError> {
        for (channel, value) in curve_num_nodes.iter().copied().enumerate() {
            if value < 0 {
                return Err(ParameterError::InvalidNodeCount {
                    channel,
                    value: i64::from(value),
                });
            }
        }
        let curve_num_nodes = curve_num_nodes.map(|value| value as u32);
        let curve_type = [
            RgbCurveType::try_from(curve_type[0])
                .map_err(|value| ParameterError::InvalidCurveType { value })?,
            RgbCurveType::try_from(curve_type[1])
                .map_err(|value| ParameterError::InvalidCurveType { value })?,
            RgbCurveType::try_from(curve_type[2])
                .map_err(|value| ParameterError::InvalidCurveType { value })?,
        ];
        let parameters = Self {
            curve_nodes,
            curve_num_nodes,
            curve_type,
            curve_autoscale: RgbCurveAutoscale::try_from(curve_autoscale)
                .map_err(|value| ParameterError::InvalidAutoscale { value })?,
            compensate_middle_grey: match compensate_middle_grey {
                0 => false,
                1 => true,
                value => {
                    return Err(ParameterError::InvalidBoolean {
                        field: "compensate_middle_grey",
                        value,
                    });
                }
            },
            preserve_colors: PreserveColors::try_from(preserve_colors)
                .map_err(|value| ParameterError::InvalidPreserveColors { value })?,
        };
        parameters.validate()?;
        Ok(parameters)
    }
}

/// Checked parameter failures at the version-1 boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParameterError {
    InvalidNodeCount {
        channel: usize,
        value: i64,
    },
    InvalidCurveType {
        value: i32,
    },
    InvalidAutoscale {
        value: i32,
    },
    InvalidBoolean {
        field: &'static str,
        value: i32,
    },
    InvalidPreserveColors {
        value: i32,
    },
    NonFinite {
        channel: usize,
        node: usize,
        coordinate: &'static str,
    },
    NonIncreasingNodes {
        channel: usize,
        left: usize,
        right: usize,
    },
}

impl fmt::Display for ParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNodeCount { channel, value } => {
                write!(
                    formatter,
                    "RGB Curve channel {channel} has invalid node count {value}"
                )
            }
            Self::InvalidCurveType { value } => write!(formatter, "invalid RGB Curve type {value}"),
            Self::InvalidAutoscale { value } => {
                write!(formatter, "invalid RGB Curve autoscale {value}")
            }
            Self::InvalidBoolean { field, value } => {
                write!(formatter, "RGB Curve {field} has invalid boolean {value}")
            }
            Self::InvalidPreserveColors { value } => {
                write!(formatter, "invalid RGB Curve preserve-colors mode {value}")
            }
            Self::NonFinite {
                channel,
                node,
                coordinate,
            } => {
                write!(
                    formatter,
                    "RGB Curve channel {channel} node {node} {coordinate} is non-finite"
                )
            }
            Self::NonIncreasingNodes {
                channel,
                left,
                right,
            } => {
                write!(
                    formatter,
                    "RGB Curve channel {channel} nodes {left} and {right} are not increasing"
                )
            }
        }
    }
}

impl std::error::Error for ParameterError {}
