//! Fixed-layout parameter/history codec ported from `src/iop/lut3d.c`.
//!
//! The native v3 payload is intentionally encoded field-by-field.  `RustTable`
//! never serializes this contract through a host `repr(C)` layout, and the
//! compressed CLUT bytes remain opaque until the GMIC seam is ported.

use std::fmt;

pub const LUT3D_SCHEMA_VERSION: u32 = 3;
pub const LUT3D_MAX_PATHNAME: usize = 512;
pub const LUT3D_MAX_LUTNAME: usize = 128;
pub const LUT3D_CLUT_LEVEL: usize = 48;
pub const LUT3D_MAX_KEYPOINTS: usize = 2048;
pub const LUT3D_COMPRESSED_CLUT_BYTES: usize = LUT3D_MAX_KEYPOINTS * 2 * 3;

pub const LUT3D_V1_PARAMETER_BYTES: usize = LUT3D_MAX_PATHNAME + 4 + 4;
pub const LUT3D_V3_PARAMETER_BYTES: usize =
    LUT3D_MAX_PATHNAME + 4 + 4 + 4 + LUT3D_COMPRESSED_CLUT_BYTES + LUT3D_MAX_LUTNAME;
pub const LUT3D_V2_PARAMETER_BYTES: usize = LUT3D_V3_PARAMETER_BYTES + 4;

const COLORSPACE_OFFSET: usize = LUT3D_MAX_PATHNAME;
const INTERPOLATION_OFFSET: usize = COLORSPACE_OFFSET + 4;
const KEYPOINTS_OFFSET: usize = INTERPOLATION_OFFSET + 4;
const CLUT_OFFSET: usize = KEYPOINTS_OFFSET + 4;
const LUTNAME_OFFSET: usize = CLUT_OFFSET + LUT3D_COMPRESSED_CLUT_BYTES;

/// Native `dt_iop_lut3d_colorspace_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Lut3dColorspace {
    Srgb = 0,
    AdobeRgb = 1,
    Rec709 = 2,
    LinearRec709 = 3,
    LinearRec2020 = 4,
    LinearProphoto = 5,
}

impl TryFrom<i32> for Lut3dColorspace {
    type Error = Lut3dCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Srgb),
            1 => Ok(Self::AdobeRgb),
            2 => Ok(Self::Rec709),
            3 => Ok(Self::LinearRec709),
            4 => Ok(Self::LinearRec2020),
            5 => Ok(Self::LinearProphoto),
            other => Err(Self::Error::UnknownColorspace(other)),
        }
    }
}

impl Lut3dColorspace {
    #[must_use]
    pub const fn native_value(self) -> i32 {
        self as i32
    }
}

/// Native `dt_iop_lut3d_interpolation_t` values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum Lut3dInterpolation {
    Tetrahedral = 0,
    Trilinear = 1,
    Pyramid = 2,
}

impl TryFrom<i32> for Lut3dInterpolation {
    type Error = Lut3dCodecError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Tetrahedral),
            1 => Ok(Self::Trilinear),
            2 => Ok(Self::Pyramid),
            other => Err(Self::Error::UnknownInterpolation(other)),
        }
    }
}

impl Lut3dInterpolation {
    #[must_use]
    pub const fn native_value(self) -> i32 {
        self as i32
    }
}

/// Exact native v3 fields in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lut3dParameters {
    pub filepath: [u8; LUT3D_MAX_PATHNAME],
    pub colorspace: Lut3dColorspace,
    pub interpolation: Lut3dInterpolation,
    pub nb_keypoints: i32,
    pub c_clut: [u8; LUT3D_COMPRESSED_CLUT_BYTES],
    pub lutname: [u8; LUT3D_MAX_LUTNAME],
}

impl Default for Lut3dParameters {
    fn default() -> Self {
        Self::defaults()
    }
}

impl Lut3dParameters {
    /// Native defaults: empty paths, sRGB, tetrahedral, and no compressed LUT.
    #[must_use]
    pub const fn defaults() -> Self {
        Self {
            filepath: [0; LUT3D_MAX_PATHNAME],
            colorspace: Lut3dColorspace::Srgb,
            interpolation: Lut3dInterpolation::Tetrahedral,
            nb_keypoints: 0,
            c_clut: [0; LUT3D_COMPRESSED_CLUT_BYTES],
            lutname: [0; LUT3D_MAX_LUTNAME],
        }
    }

    /// Serializes the exact v3 offsets with little-endian integer fields.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; LUT3D_V3_PARAMETER_BYTES] {
        let mut bytes = [0; LUT3D_V3_PARAMETER_BYTES];
        bytes[..LUT3D_MAX_PATHNAME].copy_from_slice(&self.filepath);
        bytes[COLORSPACE_OFFSET..INTERPOLATION_OFFSET]
            .copy_from_slice(&self.colorspace.native_value().to_le_bytes());
        bytes[INTERPOLATION_OFFSET..KEYPOINTS_OFFSET]
            .copy_from_slice(&self.interpolation.native_value().to_le_bytes());
        bytes[KEYPOINTS_OFFSET..CLUT_OFFSET].copy_from_slice(&self.nb_keypoints.to_le_bytes());
        bytes[CLUT_OFFSET..LUTNAME_OFFSET].copy_from_slice(&self.c_clut);
        bytes[LUTNAME_OFFSET..].copy_from_slice(&self.lutname);
        bytes
    }

    /// Decodes only an exact v3 payload.
    pub fn from_v3_bytes(bytes: &[u8]) -> Result<Self, Lut3dCodecError> {
        if bytes.len() != LUT3D_V3_PARAMETER_BYTES {
            return Err(Lut3dCodecError::InvalidLength {
                expected: LUT3D_V3_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut filepath = [0; LUT3D_MAX_PATHNAME];
        filepath.copy_from_slice(&bytes[..LUT3D_MAX_PATHNAME]);
        let mut c_clut = [0; LUT3D_COMPRESSED_CLUT_BYTES];
        c_clut.copy_from_slice(&bytes[CLUT_OFFSET..LUTNAME_OFFSET]);
        let mut lutname = [0; LUT3D_MAX_LUTNAME];
        lutname.copy_from_slice(&bytes[LUTNAME_OFFSET..]);
        Ok(Self {
            filepath,
            colorspace: Lut3dColorspace::try_from(read_i32(bytes, COLORSPACE_OFFSET))?,
            interpolation: Lut3dInterpolation::try_from(read_i32(bytes, INTERPOLATION_OFFSET))?,
            nb_keypoints: read_i32(bytes, KEYPOINTS_OFFSET),
            c_clut,
            lutname,
        })
    }

    fn from_v1_bytes(bytes: &[u8]) -> Result<Self, Lut3dCodecError> {
        if bytes.len() != LUT3D_V1_PARAMETER_BYTES {
            return Err(Lut3dCodecError::InvalidLength {
                expected: LUT3D_V1_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let mut params = Self::defaults();
        // Native g_strlcpy() copies at most destination_size - 1 bytes and
        // always leaves a terminator, even when the old fixed array is full.
        let source = &bytes[..LUT3D_MAX_PATHNAME];
        let copy_len = source
            .iter()
            .position(|byte| *byte == 0)
            .unwrap_or(LUT3D_MAX_PATHNAME - 1)
            .min(LUT3D_MAX_PATHNAME - 1);
        params.filepath[..copy_len].copy_from_slice(&source[..copy_len]);
        params.colorspace = Lut3dColorspace::try_from(read_i32(bytes, COLORSPACE_OFFSET))?;
        params.interpolation = Lut3dInterpolation::try_from(read_i32(bytes, INTERPOLATION_OFFSET))?;
        Ok(params)
    }
}

fn read_i32(bytes: &[u8], offset: usize) -> i32 {
    i32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Known history is migrated to v3; unknown versions remain byte-exact and
/// cannot accidentally enter execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lut3dHistory {
    V3(Box<Lut3dParameters>),
    Opaque { version: u32, bytes: Vec<u8> },
}

impl Lut3dHistory {
    pub fn decode(version: u32, bytes: &[u8]) -> Result<Self, Lut3dCodecError> {
        match version {
            1 => Ok(Self::V3(Box::new(Lut3dParameters::from_v1_bytes(bytes)?))),
            2 => {
                if bytes.len() != LUT3D_V2_PARAMETER_BYTES {
                    return Err(Lut3dCodecError::InvalidLength {
                        expected: LUT3D_V2_PARAMETER_BYTES,
                        actual: bytes.len(),
                    });
                }
                Ok(Self::V3(Box::new(Lut3dParameters::from_v3_bytes(
                    &bytes[..LUT3D_V3_PARAMETER_BYTES],
                )?)))
            }
            LUT3D_SCHEMA_VERSION => Ok(Self::V3(Box::new(Lut3dParameters::from_v3_bytes(bytes)?))),
            other => Ok(Self::Opaque {
                version: other,
                bytes: bytes.to_vec(),
            }),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        match self {
            Self::V3(_) => LUT3D_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V3(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { bytes, .. } => bytes.clone(),
        }
    }

    pub fn current(&self) -> Result<&Lut3dParameters, Lut3dCodecError> {
        match self {
            Self::V3(parameters) => Ok(parameters.as_ref()),
            Self::Opaque { version, .. } => Err(Lut3dCodecError::UnsupportedVersion(*version)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lut3dCodecError {
    InvalidLength { expected: usize, actual: usize },
    UnknownColorspace(i32),
    UnknownInterpolation(i32),
    UnsupportedVersion(u32),
}

impl fmt::Display for Lut3dCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "LUT3D payload has {actual} bytes; expected {expected}"
                )
            }
            Self::UnknownColorspace(value) => {
                write!(formatter, "LUT3D colorspace value {value} is unknown")
            }
            Self::UnknownInterpolation(value) => {
                write!(formatter, "LUT3D interpolation value {value} is unknown")
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "LUT3D history version {version} is opaque and unsupported"
                )
            }
        }
    }
}

impl std::error::Error for Lut3dCodecError {}
