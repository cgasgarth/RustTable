//! Little-endian history codec for the native v1/v2 Tone Equalizer ABI.

#![forbid(unsafe_code)]

use std::fmt;

use super::parameters::{
    DetailsFilter, LEGACY_V1_BYTES, LuminanceMethod, PARAMETER_BYTES, PARAMETER_VERSION,
    ParameterError, ToneEqualizerParametersV2,
};

const DETAILS_OFFSET: usize = 60;
const METHOD_OFFSET: usize = 64;
const ITERATIONS_OFFSET: usize = 68;

#[derive(Debug, Clone, PartialEq)]
pub enum ToneEqualizerHistory {
    V2(ToneEqualizerParametersV2),
}

impl ToneEqualizerHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ToneEqualizerCodecError> {
        let parameters = match version {
            1 => migrate_v1(bytes)?,
            PARAMETER_VERSION => decode_v2(bytes)?,
            other => return Err(ToneEqualizerCodecError::UnsupportedVersion(other)),
        };
        Ok(Self::V2(parameters))
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        PARAMETER_VERSION
    }

    #[must_use]
    pub const fn current(&self) -> &ToneEqualizerParametersV2 {
        match self {
            Self::V2(parameters) => parameters,
        }
    }

    #[must_use]
    pub fn payload(&self) -> [u8; PARAMETER_BYTES] {
        self.current().to_bytes()
    }

    #[must_use]
    pub const fn migration_edges() -> &'static [(u16, u16)] {
        &[(1, PARAMETER_VERSION)]
    }
}

impl ToneEqualizerParametersV2 {
    /// Serializes the 72-byte native v2 field sequence. C enum fields are
    /// represented as four-byte signed integers and are never Rust enum ABI.
    #[must_use]
    pub fn to_bytes(self) -> [u8; PARAMETER_BYTES] {
        let mut bytes = [0_u8; PARAMETER_BYTES];
        let values = [
            self.noise,
            self.ultra_deep_blacks,
            self.deep_blacks,
            self.blacks,
            self.shadows,
            self.midtones,
            self.highlights,
            self.whites,
            self.speculars,
            self.blending,
            self.smoothing,
            self.feathering,
            self.quantization,
            self.contrast_boost,
            self.exposure_boost,
        ];
        for (index, value) in values.into_iter().enumerate() {
            put_f32(&mut bytes, index * 4, value);
        }
        put_i32(&mut bytes, DETAILS_OFFSET, self.details.raw());
        put_i32(&mut bytes, METHOD_OFFSET, self.method.raw());
        put_i32(&mut bytes, ITERATIONS_OFFSET, self.iterations);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ToneEqualizerCodecError> {
        ensure_length(bytes, PARAMETER_BYTES)?;
        decode_v2(bytes)
    }
}

fn decode_v2(bytes: &[u8]) -> Result<ToneEqualizerParametersV2, ToneEqualizerCodecError> {
    let values = [
        read_f32(bytes, 0),
        read_f32(bytes, 4),
        read_f32(bytes, 8),
        read_f32(bytes, 12),
        read_f32(bytes, 16),
        read_f32(bytes, 20),
        read_f32(bytes, 24),
        read_f32(bytes, 28),
        read_f32(bytes, 32),
    ];
    let parameters = ToneEqualizerParametersV2::from_values(
        values,
        read_f32(bytes, 36),
        read_f32(bytes, 40),
        read_f32(bytes, 44),
        read_f32(bytes, 48),
        read_f32(bytes, 52),
        read_f32(bytes, 56),
        DetailsFilter::from_raw(read_i32(bytes, DETAILS_OFFSET))?,
        LuminanceMethod::from_raw(read_i32(bytes, METHOD_OFFSET))?,
        read_i32(bytes, ITERATIONS_OFFSET),
    );
    parameters
        .validate()
        .map_err(ToneEqualizerCodecError::InvalidParameters)?;
    Ok(parameters)
}

fn migrate_v1(bytes: &[u8]) -> Result<ToneEqualizerParametersV2, ToneEqualizerCodecError> {
    ensure_length(bytes, LEGACY_V1_BYTES)?;
    let parameters = ToneEqualizerParametersV2::from_values(
        [
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
            read_f32(bytes, 20),
            read_f32(bytes, 24),
            read_f32(bytes, 28),
            read_f32(bytes, 32),
        ],
        read_f32(bytes, 36),
        std::f32::consts::SQRT_2,
        read_f32(bytes, 40),
        0.0,
        read_f32(bytes, 44),
        read_f32(bytes, 48),
        DetailsFilter::from_raw(read_i32(bytes, 52))?,
        LuminanceMethod::from_raw(read_i32(bytes, 60))?,
        read_i32(bytes, 56),
    );
    parameters
        .validate()
        .map_err(ToneEqualizerCodecError::InvalidParameters)?;
    Ok(parameters)
}

const fn ensure_length(bytes: &[u8], expected: usize) -> Result<(), ToneEqualizerCodecError> {
    if bytes.len() == expected {
        Ok(())
    } else {
        Err(ToneEqualizerCodecError::InvalidLength {
            expected,
            actual: bytes.len(),
        })
    }
}

fn put_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_i32(bytes: &mut [u8], offset: usize, value: i32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("codec length was checked"),
    )
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("codec length was checked"),
    )
}

#[derive(Debug, Clone, PartialEq)]
pub enum ToneEqualizerCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnsupportedVersion(u16),
    InvalidParameters(ParameterError),
}

impl fmt::Display for ToneEqualizerCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "Tone Equalizer payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnsupportedVersion(version) => {
                write!(formatter, "Tone Equalizer version {version} is unsupported")
            }
            Self::InvalidParameters(error) => error.fmt(formatter),
        }
    }
}

impl From<ParameterError> for ToneEqualizerCodecError {
    fn from(error: ParameterError) -> Self {
        Self::InvalidParameters(error)
    }
}

impl std::error::Error for ToneEqualizerCodecError {}
