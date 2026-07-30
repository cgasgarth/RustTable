use std::fmt;
use std::hash::{Hash, Hasher};

pub const ROTATEPIXELS_COMPATIBILITY_ID: &str = "rotatepixels";
pub const ROTATEPIXELS_RUST_ID: &str = "rusttable.rotatepixels";
pub const ROTATEPIXELS_SCHEMA_VERSION: u16 = 1;
pub const ROTATEPIXELS_PARAMETER_VERSION: u16 = 1;
pub const ROTATEPIXELS_IMPLEMENTATION_VERSION: u16 = 1;
pub const ROTATEPIXELS_PARAMETER_BYTES: usize = 12;
pub const ROTATEPIXELS_MAX_DIMENSION: u32 = 1 << 30;

/// Version-one semantic parameters from Darktable's `rotatepixels` module.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RotatePixelsParametersV1 {
    pub rx: u32,
    pub ry: u32,
    pub angle: f32,
}

impl RotatePixelsParametersV1 {
    #[must_use]
    pub const fn new(rx: u32, ry: u32, angle: f32) -> Self {
        Self { rx, ry, angle }
    }

    #[must_use]
    pub fn to_bytes(self) -> [u8; ROTATEPIXELS_PARAMETER_BYTES] {
        let mut bytes = [0; ROTATEPIXELS_PARAMETER_BYTES];
        bytes[0..4].copy_from_slice(&self.rx.to_le_bytes());
        bytes[4..8].copy_from_slice(&self.ry.to_le_bytes());
        bytes[8..12].copy_from_slice(&self.angle.to_le_bytes());
        bytes
    }

    /// # Errors
    ///
    /// Returns an error for a payload with the wrong size or a non-finite angle.
    ///
    /// # Panics
    ///
    /// Panics only if the checked fixed-size slices cannot be converted.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, RotatePixelsCodecError> {
        if bytes.len() != ROTATEPIXELS_PARAMETER_BYTES {
            return Err(RotatePixelsCodecError::InvalidLength {
                expected: ROTATEPIXELS_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let angle = f32::from_le_bytes(bytes[8..12].try_into().expect("checked slice"));
        if !angle.is_finite() {
            return Err(RotatePixelsCodecError::NonFiniteAngle);
        }
        Ok(Self {
            rx: u32::from_le_bytes(bytes[0..4].try_into().expect("checked slice")),
            ry: u32::from_le_bytes(bytes[4..8].try_into().expect("checked slice")),
            angle,
        })
    }
}

/// Persisted semantic parameters plus untouched source bytes for provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct RotatePixelsConfig {
    parameters: RotatePixelsParametersV1,
    opaque_source: Option<Vec<u8>>,
}

impl RotatePixelsConfig {
    /// # Errors
    ///
    /// Returns an error when the angle is not finite.
    pub fn new(parameters: RotatePixelsParametersV1) -> Result<Self, RotatePixelsParameterError> {
        if !parameters.angle.is_finite() {
            return Err(RotatePixelsParameterError::NonFiniteAngle);
        }
        Ok(Self {
            parameters,
            opaque_source: None,
        })
    }

    #[must_use]
    pub fn with_opaque_source(mut self, source: Vec<u8>) -> Self {
        self.opaque_source = Some(source);
        self
    }

    #[must_use]
    pub const fn parameters(&self) -> RotatePixelsParametersV1 {
        self.parameters
    }

    #[must_use]
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }
}

impl Default for RotatePixelsConfig {
    fn default() -> Self {
        Self::new(RotatePixelsParametersV1::new(0, 0, 0.0)).expect("finite defaults")
    }
}

impl Eq for RotatePixelsConfig {}

impl Hash for RotatePixelsConfig {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parameters.rx.hash(state);
        self.parameters.ry.hash(state);
        self.parameters.angle.to_bits().hash(state);
        self.opaque_source.hash(state);
    }
}

/// A future parameter version is retained without attempting an unsafe decode.
#[derive(Debug, Clone, PartialEq)]
pub enum RotatePixelsHistoryParameters {
    V1(RotatePixelsParametersV1),
    Opaque { version: u16, bytes: Vec<u8> },
}

/// # Errors
///
/// Returns an error when a version-one payload is malformed.
pub fn decode_history(
    version: u16,
    bytes: &[u8],
) -> Result<RotatePixelsHistoryParameters, RotatePixelsCodecError> {
    match version {
        ROTATEPIXELS_PARAMETER_VERSION => Ok(RotatePixelsHistoryParameters::V1(
            RotatePixelsParametersV1::from_bytes(bytes)?,
        )),
        _ => Ok(RotatePixelsHistoryParameters::Opaque {
            version,
            bytes: bytes.to_vec(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RotatePixelsCodecError {
    InvalidLength { expected: usize, actual: usize },
    NonFiniteAngle,
}

impl fmt::Display for RotatePixelsCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "rotatepixels payload has {actual} bytes; expected {expected}"
                )
            }
            Self::NonFiniteAngle => formatter.write_str("rotatepixels angle must be finite"),
        }
    }
}

impl std::error::Error for RotatePixelsCodecError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotatePixelsParameterError {
    NonFiniteAngle,
}

impl fmt::Display for RotatePixelsParameterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("rotatepixels angle must be finite")
    }
}

impl std::error::Error for RotatePixelsParameterError {}

/// The four interpolation kernels exposed by the pixelpipe render preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RotatePixelsInterpolation {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

impl RotatePixelsInterpolation {
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [Self::Nearest, Self::Bilinear, Self::Bicubic, Self::Lanczos]
    }

    #[must_use]
    pub const fn support_pixels(self) -> u32 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }

    #[must_use]
    pub const fn compatibility_width(self) -> u32 {
        match self {
            Self::Nearest => 1,
            Self::Bilinear => 2,
            Self::Bicubic => 4,
            Self::Lanczos => 8,
        }
    }

    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }
}
