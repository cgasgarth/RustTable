//! Legacy `filmic` parameter ABI, mapped from `src/iop/filmic.c`.
//!
//! The native v1 and v2 migration writes the v3 struct directly.  The integer
//! fields are deliberately represented as signed 32-bit values: that is the
//! persisted native ABI, not Rust's platform-sized `isize`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::too_many_arguments,
    reason = "the native history ABI is explicitly a 32-bit little-endian layout"
)]

use std::fmt;

pub const V1_PARAMETER_BYTES: usize = 52;
pub const V2_PARAMETER_BYTES: usize = 56;
pub const V3_PARAMETER_BYTES: usize = 60;
pub const SCHEMA_VERSION: u16 = 3;

/// Native v1 declaration order from `dt_iop_filmic_params_v1_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParametersV1 {
    pub grey_point_source: f32,
    pub black_point_source: f32,
    pub white_point_source: f32,
    pub security_factor: f32,
    pub grey_point_target: f32,
    pub black_point_target: f32,
    pub white_point_target: f32,
    pub output_power: f32,
    pub latitude_stops: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub balance: f32,
    pub interpolator: i32,
}

/// Native v2 declaration order from `dt_iop_filmic_params_v2_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParametersV2 {
    pub grey_point_source: f32,
    pub black_point_source: f32,
    pub white_point_source: f32,
    pub security_factor: f32,
    pub grey_point_target: f32,
    pub black_point_target: f32,
    pub white_point_target: f32,
    pub output_power: f32,
    pub latitude_stops: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub balance: f32,
    pub interpolator: i32,
    pub preserve_color: i32,
}

/// Current native v3 declaration order from `dt_iop_filmic_params_t`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParametersV3 {
    pub grey_point_source: f32,
    pub black_point_source: f32,
    pub white_point_source: f32,
    pub security_factor: f32,
    pub grey_point_target: f32,
    pub black_point_target: f32,
    pub white_point_target: f32,
    pub output_power: f32,
    pub latitude_stops: f32,
    pub contrast: f32,
    pub saturation: f32,
    pub global_saturation: f32,
    pub balance: f32,
    pub interpolator: i32,
    pub preserve_color: i32,
}

impl ParametersV1 {
    #[must_use]
    pub const fn new(
        grey_point_source: f32,
        black_point_source: f32,
        white_point_source: f32,
        security_factor: f32,
        grey_point_target: f32,
        black_point_target: f32,
        white_point_target: f32,
        output_power: f32,
        latitude_stops: f32,
        contrast: f32,
        saturation: f32,
        balance: f32,
        interpolator: i32,
    ) -> Self {
        Self {
            grey_point_source,
            black_point_source,
            white_point_source,
            security_factor,
            grey_point_target,
            black_point_target,
            white_point_target,
            output_power,
            latitude_stops,
            contrast,
            saturation,
            balance,
            interpolator,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; V1_PARAMETER_BYTES] {
        let floats = [
            self.grey_point_source,
            self.black_point_source,
            self.white_point_source,
            self.security_factor,
            self.grey_point_target,
            self.black_point_target,
            self.white_point_target,
            self.output_power,
            self.latitude_stops,
            self.contrast,
            self.saturation,
            self.balance,
        ];
        let mut bytes = [0; V1_PARAMETER_BYTES];
        encode_floats(&mut bytes, &floats);
        bytes[48..52].copy_from_slice(&self.interpolator.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CodecError> {
        require_length(bytes, V1_PARAMETER_BYTES)?;
        let values = decode_floats::<12>(bytes);
        Ok(Self::new(
            values[0],
            values[1],
            values[2],
            values[3],
            values[4],
            values[5],
            values[6],
            values[7],
            values[8],
            values[9],
            values[10],
            values[11],
            i32::from_le_bytes(bytes[48..52].try_into().expect("checked v1 length")),
        ))
    }
}

impl ParametersV2 {
    #[must_use]
    pub const fn new(
        grey_point_source: f32,
        black_point_source: f32,
        white_point_source: f32,
        security_factor: f32,
        grey_point_target: f32,
        black_point_target: f32,
        white_point_target: f32,
        output_power: f32,
        latitude_stops: f32,
        contrast: f32,
        saturation: f32,
        balance: f32,
        interpolator: i32,
        preserve_color: i32,
    ) -> Self {
        Self {
            grey_point_source,
            black_point_source,
            white_point_source,
            security_factor,
            grey_point_target,
            black_point_target,
            white_point_target,
            output_power,
            latitude_stops,
            contrast,
            saturation,
            balance,
            interpolator,
            preserve_color,
        }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; V2_PARAMETER_BYTES] {
        let floats = [
            self.grey_point_source,
            self.black_point_source,
            self.white_point_source,
            self.security_factor,
            self.grey_point_target,
            self.black_point_target,
            self.white_point_target,
            self.output_power,
            self.latitude_stops,
            self.contrast,
            self.saturation,
            self.balance,
        ];
        let mut bytes = [0; V2_PARAMETER_BYTES];
        encode_floats(&mut bytes, &floats);
        bytes[48..52].copy_from_slice(&self.interpolator.to_le_bytes());
        bytes[52..56].copy_from_slice(&self.preserve_color.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CodecError> {
        require_length(bytes, V2_PARAMETER_BYTES)?;
        let values = decode_floats::<12>(bytes);
        Ok(Self::new(
            values[0],
            values[1],
            values[2],
            values[3],
            values[4],
            values[5],
            values[6],
            values[7],
            values[8],
            values[9],
            values[10],
            values[11],
            i32::from_le_bytes(bytes[48..52].try_into().expect("checked v2 length")),
            i32::from_le_bytes(bytes[52..56].try_into().expect("checked v2 length")),
        ))
    }
}

impl ParametersV3 {
    #[must_use]
    pub const fn new(
        grey_point_source: f32,
        black_point_source: f32,
        white_point_source: f32,
        security_factor: f32,
        grey_point_target: f32,
        black_point_target: f32,
        white_point_target: f32,
        output_power: f32,
        latitude_stops: f32,
        contrast: f32,
        saturation: f32,
        global_saturation: f32,
        balance: f32,
        interpolator: i32,
        preserve_color: i32,
    ) -> Self {
        Self {
            grey_point_source,
            black_point_source,
            white_point_source,
            security_factor,
            grey_point_target,
            black_point_target,
            white_point_target,
            output_power,
            latitude_stops,
            contrast,
            saturation,
            global_saturation,
            balance,
            interpolator,
            preserve_color,
        }
    }

    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(
            18.0, -8.65, 2.45, 0.0, 18.0, 0.0, 100.0, 2.2, 2.0, 1.5, 100.0, 100.0, 0.0, 0, 0,
        )
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; V3_PARAMETER_BYTES] {
        let floats = [
            self.grey_point_source,
            self.black_point_source,
            self.white_point_source,
            self.security_factor,
            self.grey_point_target,
            self.black_point_target,
            self.white_point_target,
            self.output_power,
            self.latitude_stops,
            self.contrast,
            self.saturation,
            self.global_saturation,
            self.balance,
        ];
        let mut bytes = [0; V3_PARAMETER_BYTES];
        encode_floats(&mut bytes, &floats);
        bytes[52..56].copy_from_slice(&self.interpolator.to_le_bytes());
        bytes[56..60].copy_from_slice(&self.preserve_color.to_le_bytes());
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, CodecError> {
        require_length(bytes, V3_PARAMETER_BYTES)?;
        let values = decode_floats::<13>(bytes);
        Ok(Self::new(
            values[0],
            values[1],
            values[2],
            values[3],
            values[4],
            values[5],
            values[6],
            values[7],
            values[8],
            values[9],
            values[10],
            values[11],
            values[12],
            i32::from_le_bytes(bytes[52..56].try_into().expect("checked v3 length")),
            i32::from_le_bytes(bytes[56..60].try_into().expect("checked v3 length")),
        ))
    }

    /// Checked boundary for materialization. Native editor ranges are not
    /// applied here: persisted finite values remain finite and unmodified.
    pub fn validate_finite(self) -> Result<Self, CodecError> {
        let values = [
            self.grey_point_source,
            self.black_point_source,
            self.white_point_source,
            self.security_factor,
            self.grey_point_target,
            self.black_point_target,
            self.white_point_target,
            self.output_power,
            self.latitude_stops,
            self.contrast,
            self.saturation,
            self.global_saturation,
            self.balance,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(CodecError::NonFiniteParameter);
        }
        Ok(self)
    }
}

/// Direct v1/v2 to v3 migration, matching `legacy_params` field assignments.
#[must_use]
pub const fn migrate_v1_to_v3(old: ParametersV1) -> ParametersV3 {
    ParametersV3::new(
        old.grey_point_source,
        old.black_point_source,
        old.white_point_source,
        old.security_factor,
        old.grey_point_target,
        old.black_point_target,
        old.white_point_target,
        old.output_power,
        old.latitude_stops,
        old.contrast,
        old.saturation,
        100.0,
        old.balance,
        old.interpolator,
        0,
    )
}

/// Direct v2 to v3 migration, matching `legacy_params` field assignments.
#[must_use]
pub const fn migrate_v2_to_v3(old: ParametersV2) -> ParametersV3 {
    ParametersV3::new(
        old.grey_point_source,
        old.black_point_source,
        old.white_point_source,
        old.security_factor,
        old.grey_point_target,
        old.black_point_target,
        old.white_point_target,
        old.output_power,
        old.latitude_stops,
        old.contrast,
        old.saturation,
        100.0,
        old.balance,
        old.interpolator,
        old.preserve_color,
    )
}

/// Known history remains typed; unknown versions retain their exact payload.
#[derive(Debug, Clone, PartialEq)]
pub enum History {
    V1(ParametersV1),
    V2(ParametersV2),
    V3(ParametersV3),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl History {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, CodecError> {
        match version {
            1 => Ok(Self::V1(ParametersV1::from_bytes(bytes)?)),
            2 => Ok(Self::V2(ParametersV2::from_bytes(bytes)?)),
            SCHEMA_VERSION => Ok(Self::V3(ParametersV3::from_bytes(bytes)?)),
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
            Self::V3(_) => SCHEMA_VERSION,
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

    pub fn current(&self) -> Result<ParametersV3, CodecError> {
        match self {
            Self::V1(parameters) => migrate_v1_to_v3(*parameters).validate_finite(),
            Self::V2(parameters) => migrate_v2_to_v3(*parameters).validate_finite(),
            Self::V3(parameters) => parameters.validate_finite(),
            Self::Opaque { version, .. } => Err(CodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
    NonFiniteParameter,
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "filmic payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "filmic version {version} is opaque and unsupported"
                )
            }
            Self::NonFiniteParameter => formatter.write_str("filmic parameter is non-finite"),
        }
    }
}

impl std::error::Error for CodecError {}

const fn require_length(bytes: &[u8], expected: usize) -> Result<(), CodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(CodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
    }
}

fn encode_floats<const N: usize>(bytes: &mut [u8], values: &[f32; N]) {
    for (index, value) in values.iter().copied().enumerate() {
        let start = index * 4;
        bytes[start..start + 4].copy_from_slice(&value.to_le_bytes());
    }
}

fn decode_floats<const N: usize>(bytes: &[u8]) -> [f32; N] {
    std::array::from_fn(|index| {
        let start = index * 4;
        f32::from_le_bytes(
            bytes[start..start + 4]
                .try_into()
                .expect("checked float length"),
        )
    })
}
