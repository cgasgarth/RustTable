//! Tone Curve parameter contracts ported from `src/iop/tonecurve.c`.
//!
//! Native declaration order is preserved because every active and inactive node
//! slot is part of the history ABI.

#![forbid(unsafe_code)]

use std::fmt;

pub const CHANNELS: usize = 3;
pub const MAX_NODES: usize = 20;
pub const PARAMETER_VERSION: u16 = 5;
pub const PARAMETER_BYTES: usize = 520;
pub const LEGACY_V1_BYTES: usize = 52;
pub const LEGACY_V3_BYTES: usize = 512;
pub const LEGACY_V4_BYTES: usize = 516;
pub const LUT_RESOLUTION: u32 = 0x1_0000;

/// Native channel IDs: L, a, b.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(usize)]
pub enum ToneCurveChannel {
    L = 0,
    A = 1,
    B = 2,
}

impl ToneCurveChannel {
    pub const ALL: [Self; CHANNELS] = [Self::L, Self::A, Self::B];

    #[must_use]
    pub const fn index(self) -> usize {
        self as usize
    }
}

/// Native interpolation IDs from `curve_tools.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ToneCurveType {
    CubicSpline = 0,
    CatmullRom = 1,
    MonotoneHermite = 2,
}

impl TryFrom<i32> for ToneCurveType {
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

/// Native `dt_iop_tonecurve_autoscale_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ToneCurveAutoscale {
    AutomaticLab = 1,
    ManualLab = 0,
    AutomaticXyz = 2,
    AutomaticRgb = 3,
}

impl TryFrom<i32> for ToneCurveAutoscale {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::ManualLab),
            1 => Ok(Self::AutomaticLab),
            2 => Ok(Self::AutomaticXyz),
            3 => Ok(Self::AutomaticRgb),
            other => Err(other),
        }
    }
}

/// Native `dt_iop_rgb_norms_t` values used for RGB automatic color scaling.
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

/// One native `dt_iop_tonecurve_node_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ToneCurveNode {
    pub x: f32,
    pub y: f32,
}

impl ToneCurveNode {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    #[must_use]
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Checked v5 parameters in native declaration order.
#[derive(Debug, Clone, PartialEq)]
pub struct ToneCurveParametersV5 {
    pub tonecurve: [[ToneCurveNode; MAX_NODES]; CHANNELS],
    pub tonecurve_nodes: [u32; CHANNELS],
    pub tonecurve_type: [ToneCurveType; CHANNELS],
    pub tonecurve_autoscale_ab: ToneCurveAutoscale,
    pub tonecurve_preset: i32,
    pub tonecurve_unbound_ab: bool,
    pub preserve_colors: PreserveColors,
}

impl Default for ToneCurveParametersV5 {
    fn default() -> Self {
        let mut tonecurve = [[ToneCurveNode::ZERO; MAX_NODES]; CHANNELS];
        tonecurve[0][1] = ToneCurveNode::new(1.0, 1.0);
        tonecurve[1][1] = ToneCurveNode::new(0.5, 0.5);
        tonecurve[1][2] = ToneCurveNode::new(1.0, 1.0);
        tonecurve[2][1] = ToneCurveNode::new(0.5, 0.5);
        tonecurve[2][2] = ToneCurveNode::new(1.0, 1.0);
        Self {
            tonecurve,
            tonecurve_nodes: [2, 3, 3],
            tonecurve_type: [ToneCurveType::MonotoneHermite; CHANNELS],
            tonecurve_autoscale_ab: ToneCurveAutoscale::AutomaticRgb,
            tonecurve_preset: 0,
            tonecurve_unbound_ab: true,
            preserve_colors: PreserveColors::Average,
        }
    }
}

impl ToneCurveParametersV5 {
    #[must_use]
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Validates active node state without touching inactive serialized tails.
    pub fn validate(&self) -> Result<(), ParameterError> {
        for channel in 0..CHANNELS {
            let count = usize::try_from(self.tonecurve_nodes[channel]).map_err(|_| {
                ParameterError::InvalidNodeCount {
                    channel,
                    value: i64::from(self.tonecurve_nodes[channel]),
                }
            })?;
            if !(2..=MAX_NODES).contains(&count) {
                return Err(ParameterError::InvalidNodeCount {
                    channel,
                    value: i64::from(self.tonecurve_nodes[channel]),
                });
            }
            let mut previous_x = None;
            for node in 0..count {
                let point = self.tonecurve[channel][node];
                if !point.x.is_finite() {
                    return Err(ParameterError::NonFinite {
                        channel,
                        node,
                        coordinate: "x",
                    });
                }
                if !point.y.is_finite() {
                    return Err(ParameterError::NonFinite {
                        channel,
                        node,
                        coordinate: "y",
                    });
                }
                if let Some(previous_x) = previous_x {
                    if point.x <= previous_x {
                        return Err(ParameterError::NonIncreasingNodes {
                            channel,
                            left: node - 1,
                            right: node,
                        });
                    }
                }
                previous_x = Some(point.x);
            }
        }
        Ok(())
    }

    pub(crate) fn from_raw(
        tonecurve: [[ToneCurveNode; MAX_NODES]; CHANNELS],
        tonecurve_nodes: [i32; CHANNELS],
        tonecurve_type: [i32; CHANNELS],
        tonecurve_autoscale_ab: i32,
        tonecurve_preset: i32,
        tonecurve_unbound_ab: i32,
        preserve_colors: i32,
    ) -> Result<Self, ParameterError> {
        for (channel, value) in tonecurve_nodes.iter().copied().enumerate() {
            if value < 0 {
                return Err(ParameterError::InvalidNodeCount {
                    channel,
                    value: i64::from(value),
                });
            }
        }
        let tonecurve_type = tonecurve_type
            .map(|value| {
                ToneCurveType::try_from(value)
                    .map_err(|value| ParameterError::InvalidCurveType { value })
            })
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .expect("three curve types");
        let parameters = Self {
            tonecurve,
            tonecurve_nodes: tonecurve_nodes.map(|value| value as u32),
            tonecurve_type,
            tonecurve_autoscale_ab: ToneCurveAutoscale::try_from(tonecurve_autoscale_ab)
                .map_err(|value| ParameterError::InvalidAutoscale { value })?,
            tonecurve_preset,
            tonecurve_unbound_ab: match tonecurve_unbound_ab {
                0 => false,
                1 => true,
                value => {
                    return Err(ParameterError::InvalidBoolean {
                        field: "tonecurve_unbound_ab",
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
                    "Tone Curve channel {channel} has invalid node count {value}"
                )
            }
            Self::InvalidCurveType { value } => {
                write!(formatter, "invalid Tone Curve type {value}")
            }
            Self::InvalidAutoscale { value } => {
                write!(formatter, "invalid Tone Curve autoscale {value}")
            }
            Self::InvalidBoolean { field, value } => {
                write!(formatter, "Tone Curve {field} has invalid boolean {value}")
            }
            Self::InvalidPreserveColors { value } => {
                write!(formatter, "invalid Tone Curve preserve-colors mode {value}")
            }
            Self::NonFinite {
                channel,
                node,
                coordinate,
            } => write!(
                formatter,
                "Tone Curve channel {channel} node {node} {coordinate} is non-finite"
            ),
            Self::NonIncreasingNodes {
                channel,
                left,
                right,
            } => write!(
                formatter,
                "Tone Curve channel {channel} nodes {left} and {right} are not increasing"
            ),
        }
    }
}

impl std::error::Error for ParameterError {}
