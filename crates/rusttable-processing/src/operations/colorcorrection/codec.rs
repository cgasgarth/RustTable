//! Native v1 Color Correction history ABI from `src/iop/colorcorrection.c`.

#![forbid(unsafe_code)]

use std::fmt;
use std::mem::size_of;

pub const COLORCORRECTION_SCHEMA_VERSION: u16 = 1;
pub const COLORCORRECTION_V1_PARAMETER_BYTES: usize = 20;
pub const COLORCORRECTION_MIGRATION_EDGES: &[(u16, u16)] = &[];

/// Native `dt_iop_colorcorrection_params_t` in declaration order.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct ColorCorrectionParametersV1 {
    pub hia: f32,
    pub hib: f32,
    pub loa: f32,
    pub lob: f32,
    pub saturation: f32,
}

const _: () =
    assert!(size_of::<ColorCorrectionParametersV1>() == COLORCORRECTION_V1_PARAMETER_BYTES);

impl ColorCorrectionParametersV1 {
    #[must_use]
    pub const fn new(hia: f32, hib: f32, loa: f32, lob: f32, saturation: f32) -> Self {
        Self {
            hia,
            hib,
            loa,
            lob,
            saturation,
        }
    }

    /// Introspection initializes the four unannotated endpoints to zero and
    /// uses the source `$DEFAULT: 1.0` annotation for saturation.
    #[must_use]
    pub const fn defaults() -> Self {
        Self::new(0.0, 0.0, 0.0, 0.0, 1.0)
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; COLORCORRECTION_V1_PARAMETER_BYTES] {
        let mut bytes = [0_u8; COLORCORRECTION_V1_PARAMETER_BYTES];
        write_f32(&mut bytes, 0, self.hia);
        write_f32(&mut bytes, 4, self.hib);
        write_f32(&mut bytes, 8, self.loa);
        write_f32(&mut bytes, 12, self.lob);
        write_f32(&mut bytes, 16, self.saturation);
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ColorCorrectionCodecError> {
        if bytes.len() != COLORCORRECTION_V1_PARAMETER_BYTES {
            return Err(ColorCorrectionCodecError::InvalidLength {
                version: COLORCORRECTION_SCHEMA_VERSION,
                expected: COLORCORRECTION_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        Ok(Self::new(
            read_f32(bytes, 0),
            read_f32(bytes, 4),
            read_f32(bytes, 8),
            read_f32(bytes, 12),
            read_f32(bytes, 16),
        ))
    }
}

impl Default for ColorCorrectionParametersV1 {
    fn default() -> Self {
        Self::defaults()
    }
}

/// Typed current history plus byte-exact retention for unknown versions.
#[derive(Debug, Clone, PartialEq)]
pub enum ColorCorrectionHistory {
    V1(ColorCorrectionParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

impl ColorCorrectionHistory {
    pub fn decode(version: u16, bytes: &[u8]) -> Result<Self, ColorCorrectionCodecError> {
        match version {
            COLORCORRECTION_SCHEMA_VERSION => {
                Ok(Self::V1(ColorCorrectionParametersV1::from_bytes(bytes)?))
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
            Self::V1(_) => COLORCORRECTION_SCHEMA_VERSION,
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

    /// Color Correction has no native `legacy_params` branch: v1 is already
    /// current and every unknown version remains non-executable and opaque.
    pub fn migrate_to_current(
        &self,
    ) -> Result<ColorCorrectionParametersV1, ColorCorrectionCodecError> {
        match self {
            Self::V1(parameters) => Ok(*parameters),
            Self::Opaque { version, .. } => Err(ColorCorrectionCodecError::OpaqueVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorCorrectionCodecError {
    InvalidLength {
        version: u16,
        expected: usize,
        actual: usize,
    },
    OpaqueVersion(u16),
}

impl fmt::Display for ColorCorrectionCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength {
                version,
                expected,
                actual,
            } => write!(
                formatter,
                "Color Correction v{version} payload has {actual} bytes; expected {expected}"
            ),
            Self::OpaqueVersion(version) => {
                write!(
                    formatter,
                    "Color Correction history version {version} is opaque"
                )
            }
        }
    }
}

impl std::error::Error for ColorCorrectionCodecError {}

fn write_f32(bytes: &mut [u8], offset: usize, value: f32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_f32(bytes: &[u8], offset: usize) -> f32 {
    f32::from_le_bytes(
        bytes[offset..offset + 4]
            .try_into()
            .expect("validated Color Correction v1 field range"),
    )
}
