//! Native Color Balance parameter ABI and history migration.
//!
//! Direct source lineage: `src/iop/colorbalance.c`, `legacy_params`, and
//! `dt_iop_colorbalance_params_t`.  The payloads are encoded explicitly in
//! declaration order; Rust layout and host alignment are intentionally absent.

use std::fmt;

pub const COLORBALANCE_INTROSPECTION_VERSION: u16 = 3;
pub const COLORBALANCE_V1_PARAMETER_BYTES: usize = 48;
pub const COLORBALANCE_V2_PARAMETER_BYTES: usize = 64;
pub const COLORBALANCE_V3_PARAMETER_BYTES: usize = 68;

pub const CHANNEL_FACTOR: usize = 0;
pub const CHANNEL_RED: usize = 1;
pub const CHANNEL_GREEN: usize = 2;
pub const CHANNEL_BLUE: usize = 3;
pub const CHANNEL_SIZE: usize = 4;

/// Native `dt_iop_colorbalance_mode_t` values and signed 32-bit ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ColorBalanceMode {
    LiftGammaGain = 0,
    SlopeOffsetPower = 1,
    Legacy = 2,
}

impl TryFrom<i32> for ColorBalanceMode {
    type Error = ColorBalanceCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::LiftGammaGain),
            1 => Ok(Self::SlopeOffsetPower),
            2 => Ok(Self::Legacy),
            value => Err(Self::Error::UnknownMode(value)),
        }
    }
}

impl From<ColorBalanceMode> for i32 {
    fn from(value: ColorBalanceMode) -> Self {
        value as Self
    }
}

/// Native v1 declaration order: lift, gamma, gain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceParametersV1 {
    pub lift: [f32; CHANNEL_SIZE],
    pub gamma: [f32; CHANNEL_SIZE],
    pub gain: [f32; CHANNEL_SIZE],
}

impl ColorBalanceParametersV1 {
    #[must_use]
    pub const fn new(
        lift: [f32; CHANNEL_SIZE],
        gamma: [f32; CHANNEL_SIZE],
        gain: [f32; CHANNEL_SIZE],
    ) -> Self {
        Self { lift, gamma, gain }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCE_V1_PARAMETER_BYTES] {
        encode_f32s::<12, COLORBALANCE_V1_PARAMETER_BYTES>(
            self.lift.into_iter().chain(self.gamma).chain(self.gain),
        )
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceCodecError> {
        let values = decode_f32s::<12>(bytes, COLORBALANCE_V1_PARAMETER_BYTES)?;
        Ok(Self::new(
            values[0..4].try_into().expect("fixed v1 lift width"),
            values[4..8].try_into().expect("fixed v1 gamma width"),
            values[8..12].try_into().expect("fixed v1 gain width"),
        ))
    }
}

/// Native v2 declaration order: mode, lift, gamma, gain, saturation,
/// contrast, grey.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceParametersV2 {
    pub mode: ColorBalanceMode,
    pub lift: [f32; CHANNEL_SIZE],
    pub gamma: [f32; CHANNEL_SIZE],
    pub gain: [f32; CHANNEL_SIZE],
    pub saturation: f32,
    pub contrast: f32,
    pub grey: f32,
}

impl ColorBalanceParametersV2 {
    #[must_use]
    pub const fn new(
        mode: ColorBalanceMode,
        lift: [f32; CHANNEL_SIZE],
        gamma: [f32; CHANNEL_SIZE],
        gain: [f32; CHANNEL_SIZE],
        saturation: f32,
        contrast: f32,
        grey: f32,
    ) -> Self {
        Self {
            mode,
            lift,
            gamma,
            gain,
            saturation,
            contrast,
            grey,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCE_V2_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLORBALANCE_V2_PARAMETER_BYTES];
        bytes[..4].copy_from_slice(&i32::from(self.mode).to_le_bytes());
        encode_f32s_into(
            &mut bytes,
            4,
            self.lift
                .into_iter()
                .chain(self.gamma)
                .chain(self.gain)
                .chain([self.saturation, self.contrast, self.grey]),
        );
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceCodecError> {
        if bytes.len() != COLORBALANCE_V2_PARAMETER_BYTES {
            return Err(ColorBalanceCodecError::InvalidLength {
                expected: COLORBALANCE_V2_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mode =
            i32::from_le_bytes(bytes[..4].try_into().expect("checked mode width")).try_into()?;
        let values = decode_f32s::<15>(&bytes[4..], COLORBALANCE_V2_PARAMETER_BYTES - 4)?;
        Ok(Self::new(
            mode,
            values[0..4].try_into().expect("fixed v2 lift width"),
            values[4..8].try_into().expect("fixed v2 gamma width"),
            values[8..12].try_into().expect("fixed v2 gain width"),
            values[12],
            values[13],
            values[14],
        ))
    }
}

/// Native current v3 declaration order: v2 plus `saturation_out`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorBalanceParametersV3 {
    pub mode: ColorBalanceMode,
    pub lift: [f32; CHANNEL_SIZE],
    pub gamma: [f32; CHANNEL_SIZE],
    pub gain: [f32; CHANNEL_SIZE],
    pub saturation: f32,
    pub contrast: f32,
    pub grey: f32,
    pub saturation_out: f32,
}

impl ColorBalanceParametersV3 {
    #[must_use]
    #[allow(
        clippy::too_many_arguments,
        reason = "the constructor preserves the native v3 declaration order"
    )]
    pub const fn new(
        mode: ColorBalanceMode,
        lift: [f32; CHANNEL_SIZE],
        gamma: [f32; CHANNEL_SIZE],
        gain: [f32; CHANNEL_SIZE],
        saturation: f32,
        contrast: f32,
        grey: f32,
        saturation_out: f32,
    ) -> Self {
        Self {
            mode,
            lift,
            gamma,
            gain,
            saturation,
            contrast,
            grey,
            saturation_out,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            ColorBalanceMode::SlopeOffsetPower,
            [1.0; CHANNEL_SIZE],
            [1.0; CHANNEL_SIZE],
            [1.0; CHANNEL_SIZE],
            1.0,
            1.0,
            18.0,
            1.0,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORBALANCE_V3_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLORBALANCE_V3_PARAMETER_BYTES];
        bytes[..4].copy_from_slice(&i32::from(self.mode).to_le_bytes());
        encode_f32s_into(
            &mut bytes,
            4,
            self.lift
                .into_iter()
                .chain(self.gamma)
                .chain(self.gain)
                .chain([
                    self.saturation,
                    self.contrast,
                    self.grey,
                    self.saturation_out,
                ]),
        );
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorBalanceCodecError> {
        if bytes.len() != COLORBALANCE_V3_PARAMETER_BYTES {
            return Err(ColorBalanceCodecError::InvalidLength {
                expected: COLORBALANCE_V3_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mode =
            i32::from_le_bytes(bytes[..4].try_into().expect("checked mode width")).try_into()?;
        let values = decode_f32s::<16>(&bytes[4..], COLORBALANCE_V3_PARAMETER_BYTES - 4)?;
        Ok(Self::new(
            mode,
            values[0..4].try_into().expect("fixed v3 lift width"),
            values[4..8].try_into().expect("fixed v3 gamma width"),
            values[8..12].try_into().expect("fixed v3 gain width"),
            values[12],
            values[13],
            values[14],
            values[15],
        ))
    }
}

fn encode_f32s<const COUNT: usize, const BYTES: usize>(
    values: impl IntoIterator<Item = f32>,
) -> [u8; BYTES] {
    let mut bytes = [0_u8; BYTES];
    encode_f32s_into(&mut bytes, 0, values);
    bytes
}

fn encode_f32s_into<const BYTES: usize>(
    bytes: &mut [u8; BYTES],
    start: usize,
    values: impl IntoIterator<Item = f32>,
) {
    for (index, value) in values.into_iter().enumerate() {
        let offset = start + index * 4;
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn decode_f32s<const COUNT: usize>(
    bytes: &[u8],
    expected: usize,
) -> Result<[f32; COUNT], ColorBalanceCodecError> {
    if bytes.len() != expected {
        return Err(ColorBalanceCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        });
    }
    Ok(std::array::from_fn(|index| {
        let offset = index * 4;
        f32::from_le_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("checked float width"),
        )
    }))
}

/// Typed known history plus byte-exact retention for future versions.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorBalanceHistory {
    V1(ColorBalanceParametersV1),
    V2(ColorBalanceParametersV2),
    V3(ColorBalanceParametersV3),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorBalanceHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorBalanceCodecError> {
        match version {
            1 => Ok(Self::V1(ColorBalanceParametersV1::from_bytes(bytes)?)),
            2 => Ok(Self::V2(ColorBalanceParametersV2::from_bytes(bytes)?)),
            COLORBALANCE_INTROSPECTION_VERSION => {
                Ok(Self::V3(ColorBalanceParametersV3::from_bytes(bytes)?))
            }
            _ => Ok(Self::Opaque {
                version,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
            Self::V3(_) => COLORBALANCE_INTROSPECTION_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::V2(parameters) => parameters.to_bytes().to_vec(),
            Self::V3(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub fn current(&self) -> Result<ColorBalanceParametersV3, ColorBalanceCodecError> {
        match self {
            Self::V1(parameters) => Ok(migrate_v1_to_v3(*parameters)),
            Self::V2(parameters) => Ok(migrate_v2_to_v3(*parameters)),
            Self::V3(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => {
                Err(ColorBalanceCodecError::UnsupportedVersion(*version))
            }
        }
    }
}

/// Native `legacy_params(old_version == 1)` with its exact defaults.
#[must_use]
pub const fn migrate_v1_to_v3(parameters: ColorBalanceParametersV1) -> ColorBalanceParametersV3 {
    ColorBalanceParametersV3::new(
        ColorBalanceMode::Legacy,
        parameters.lift,
        parameters.gamma,
        parameters.gain,
        1.0,
        1.0,
        18.0,
        1.0,
    )
}

/// Native `legacy_params(old_version == 2)` with `saturation_out` introduced
/// at its v3 default.
#[must_use]
pub const fn migrate_v2_to_v3(parameters: ColorBalanceParametersV2) -> ColorBalanceParametersV3 {
    ColorBalanceParametersV3::new(
        parameters.mode,
        parameters.lift,
        parameters.gamma,
        parameters.gain,
        parameters.saturation,
        parameters.contrast,
        parameters.grey,
        1.0,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorBalanceCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownMode(i32),
    UnsupportedVersion(u16),
}

impl fmt::Display for ColorBalanceCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => write!(
                formatter,
                "Color Balance payload has {actual} bytes; expected {expected}"
            ),
            Self::UnknownMode(value) => write!(formatter, "Color Balance mode {value} is unknown"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "Color Balance version {version} is opaque and unsupported"
            ),
        }
    }
}

impl std::error::Error for ColorBalanceCodecError {}
