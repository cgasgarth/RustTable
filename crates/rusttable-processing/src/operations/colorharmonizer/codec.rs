//! Parameter ABI ported from `src/iop/colorharmonizer.c`.
//!
//! The native module has introspection version 1 and a 60-byte payload.  This
//! module deliberately keeps the known payload separate from the opaque
//! unknown-version representation so future history remains byte-exact but
//! cannot become executable accidentally.

use std::fmt;

pub const COLORHARMONIZER_SCHEMA_VERSION: u16 = 1;
pub const COLORHARMONIZER_PARAMETER_BYTES: usize = 60;
pub const COLORHARMONIZER_MAX_NODES: usize = 4;
pub const COLORHARMONIZER_RYB_INVERSE_STEPS: usize = 720;

pub const COLORHARMONIZER_DEFAULT_ANCHOR_HUE: f32 = 0.1;
pub const COLORHARMONIZER_DEFAULT_PULL_STRENGTH: f32 = 0.0;
pub const COLORHARMONIZER_DEFAULT_NEUTRAL_PROTECTION: f32 = 0.5;
pub const COLORHARMONIZER_DEFAULT_PULL_WIDTH: f32 = 1.0;
pub const COLORHARMONIZER_DEFAULT_NUM_CUSTOM_NODES: i32 = 4;
pub const COLORHARMONIZER_DEFAULT_SMOOTHING: f32 = 0.0;

/// Native enum values from `dt_iop_colorharmonizer_rule_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorHarmonizerRule {
    Monochromatic = 0,
    Analogous = 1,
    AnalogousComplementary = 2,
    Complementary = 3,
    SplitComplementary = 4,
    Dyad = 5,
    Triad = 6,
    Tetrad = 7,
    Square = 8,
    Custom = 9,
}

impl TryFrom<i32> for ColorHarmonizerRule {
    type Error = i32;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Monochromatic),
            1 => Ok(Self::Analogous),
            2 => Ok(Self::AnalogousComplementary),
            3 => Ok(Self::Complementary),
            4 => Ok(Self::SplitComplementary),
            5 => Ok(Self::Dyad),
            6 => Ok(Self::Triad),
            7 => Ok(Self::Tetrad),
            8 => Ok(Self::Square),
            9 => Ok(Self::Custom),
            other => Err(other),
        }
    }
}

impl ColorHarmonizerRule {
    #[must_use]
    pub const fn native_value(self) -> i32 {
        self as i32
    }

    /// Native geometry offsets in RYB turns, in source table order.
    #[must_use]
    pub const fn geometry(self) -> &'static [f32] {
        match self {
            Self::Monochromatic => &[0.0],
            Self::Analogous => &[-1.0 / 12.0, 0.0, 1.0 / 12.0],
            Self::AnalogousComplementary => &[-1.0 / 12.0, 0.0, 1.0 / 12.0, 6.0 / 12.0],
            Self::Complementary => &[0.0, 6.0 / 12.0],
            Self::SplitComplementary => &[0.0, 5.0 / 12.0, 7.0 / 12.0],
            Self::Dyad => &[-1.0 / 12.0, 1.0 / 12.0],
            Self::Triad => &[0.0, 4.0 / 12.0, 8.0 / 12.0],
            Self::Tetrad => &[-1.0 / 12.0, 1.0 / 12.0, 5.0 / 12.0, 7.0 / 12.0],
            Self::Square => &[0.0, 3.0 / 12.0, 6.0 / 12.0, 9.0 / 12.0],
            Self::Custom => &[],
        }
    }
}

/// Exact native v1 parameter payload in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorHarmonizerParametersV1 {
    pub rule: ColorHarmonizerRule,
    pub anchor_hue: f32,
    pub pull_strength: f32,
    pub neutral_protection: f32,
    pub pull_width: f32,
    pub custom_hue: [f32; COLORHARMONIZER_MAX_NODES],
    pub num_custom_nodes: i32,
    pub node_saturation: [f32; COLORHARMONIZER_MAX_NODES],
    pub smoothing: f32,
}

impl ColorHarmonizerParametersV1 {
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "The constructor preserves the native v1 field order."
    )]
    pub const fn new(
        rule: ColorHarmonizerRule,
        anchor_hue: f32,
        pull_strength: f32,
        neutral_protection: f32,
        pull_width: f32,
        custom_hue: [f32; COLORHARMONIZER_MAX_NODES],
        num_custom_nodes: i32,
        node_saturation: [f32; COLORHARMONIZER_MAX_NODES],
        smoothing: f32,
    ) -> Self {
        Self {
            rule,
            anchor_hue,
            pull_strength,
            neutral_protection,
            pull_width,
            custom_hue,
            num_custom_nodes,
            node_saturation,
            smoothing,
        }
    }

    /// Native introspection defaults followed by `init()`'s custom-node values.
    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            ColorHarmonizerRule::Complementary,
            COLORHARMONIZER_DEFAULT_ANCHOR_HUE,
            COLORHARMONIZER_DEFAULT_PULL_STRENGTH,
            COLORHARMONIZER_DEFAULT_NEUTRAL_PROTECTION,
            COLORHARMONIZER_DEFAULT_PULL_WIDTH,
            [0.0, 0.25, 0.5, 0.75],
            COLORHARMONIZER_DEFAULT_NUM_CUSTOM_NODES,
            [1.0; COLORHARMONIZER_MAX_NODES],
            COLORHARMONIZER_DEFAULT_SMOOTHING,
        )
    }

    /// Serializes the pinned-target native layout explicitly as little endian.
    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORHARMONIZER_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLORHARMONIZER_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.rule.native_value().to_le_bytes());
        bytes[4..8].copy_from_slice(&self.anchor_hue.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.pull_strength.to_le_bytes());
        bytes[12..16].copy_from_slice(&self.neutral_protection.to_le_bytes());
        bytes[16..20].copy_from_slice(&self.pull_width.to_le_bytes());
        for (index, value) in self.custom_hue.into_iter().enumerate() {
            let start = 20 + index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[36..40].copy_from_slice(&self.num_custom_nodes.to_le_bytes());
        for (index, value) in self.node_saturation.into_iter().enumerate() {
            let start = 40 + index * 4;
            bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
        }
        bytes[56..60].copy_from_slice(&self.smoothing.to_le_bytes());
        bytes
    }

    /// Decodes only the exact 60-byte known-version payload.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorHarmonizerCodecError> {
        if bytes.len() != COLORHARMONIZER_PARAMETER_BYTES {
            return Err(ColorHarmonizerCodecError::InvalidLength {
                expected: COLORHARMONIZER_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let rule_value = i32::from_le_bytes(read_array(bytes, 0));
        let rule = ColorHarmonizerRule::try_from(rule_value)
            .map_err(ColorHarmonizerCodecError::UnknownRule)?;
        let custom_hue =
            std::array::from_fn(|index| f32::from_le_bytes(read_array(bytes, 20 + index * 4)));
        let node_saturation =
            std::array::from_fn(|index| f32::from_le_bytes(read_array(bytes, 40 + index * 4)));
        Ok(Self::new(
            rule,
            f32::from_le_bytes(read_array(bytes, 4)),
            f32::from_le_bytes(read_array(bytes, 8)),
            f32::from_le_bytes(read_array(bytes, 12)),
            f32::from_le_bytes(read_array(bytes, 16)),
            custom_hue,
            i32::from_le_bytes(read_array(bytes, 36)),
            node_saturation,
            f32::from_le_bytes(read_array(bytes, 56)),
        ))
    }
}

const fn read_array(bytes: &[u8], start: usize) -> [u8; 4] {
    [
        bytes[start],
        bytes[start + 1],
        bytes[start + 2],
        bytes[start + 3],
    ]
}

/// Known v1 history or byte-exact opaque future history.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorHarmonizerHistory {
    V1(ColorHarmonizerParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorHarmonizerHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorHarmonizerCodecError> {
        if version == COLORHARMONIZER_SCHEMA_VERSION {
            Ok(Self::V1(ColorHarmonizerParametersV1::from_bytes(bytes)?))
        } else {
            Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            })
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => COLORHARMONIZER_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub const fn current(&self) -> Result<ColorHarmonizerParametersV1, ColorHarmonizerCodecError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => {
                Err(ColorHarmonizerCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ColorHarmonizerCodecError {
    InvalidLength {
        expected: usize,
        actual: usize,
    },
    UnknownRule(i32),
    UnsupportedVersion(u16),
    NonFinite(&'static str),
    HueOutOfRange(&'static str),
    ParameterOutOfRange {
        name: &'static str,
        minimum: f32,
        maximum: f32,
    },
    NodeCountOutOfRange {
        value: i32,
        minimum: i32,
        maximum: i32,
    },
    NonPositivePullWidth,
}

impl fmt::Display for ColorHarmonizerCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Color Harmonizer payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnknownRule(value) => {
                write!(formatter, "Color Harmonizer rule {value} is unknown")
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "Color Harmonizer version {version} is opaque and unsupported"
                )
            }
            Self::NonFinite(name) => write!(formatter, "Color Harmonizer {name} is non-finite"),
            Self::HueOutOfRange(name) => {
                write!(
                    formatter,
                    "Color Harmonizer {name} must be in the inclusive [0, 1] domain"
                )
            }
            Self::ParameterOutOfRange {
                name,
                minimum,
                maximum,
            } => write!(
                formatter,
                "Color Harmonizer {name} must be in the inclusive [{minimum}, {maximum}] range"
            ),
            Self::NodeCountOutOfRange {
                value,
                minimum,
                maximum,
            } => write!(
                formatter,
                "Color Harmonizer num_custom_nodes {value} is outside [{minimum}, {maximum}]"
            ),
            Self::NonPositivePullWidth => {
                formatter.write_str("Color Harmonizer pull_width must be positive")
            }
        }
    }
}

impl std::error::Error for ColorHarmonizerCodecError {}
