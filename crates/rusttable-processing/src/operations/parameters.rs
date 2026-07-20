use super::{
    MAX_PIXEL_ASPECT_RATIO, MIN_PIXEL_ASPECT_RATIO, SCALEPIXELS_PARAMETER_BYTES,
    SCALEPIXELS_SCHEMA_VERSION,
};
use crate::FiniteF32;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScalePixelsPreferences {
    image: ScalePixelsKernel,
    warp: ScalePixelsKernel,
}

impl ScalePixelsPreferences {
    #[must_use]
    pub const fn new(image: ScalePixelsKernel, warp: ScalePixelsKernel) -> Self {
        Self { image, warp }
    }

    #[must_use]
    pub const fn image(self) -> ScalePixelsKernel {
        self.image
    }

    #[must_use]
    pub const fn warp(self) -> ScalePixelsKernel {
        self.warp
    }
}

impl Default for ScalePixelsPreferences {
    fn default() -> Self {
        Self::new(ScalePixelsKernel::Bilinear, ScalePixelsKernel::Bilinear)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalePixelsKernel {
    Nearest,
    Bilinear,
    Bicubic,
    Lanczos,
}

impl ScalePixelsKernel {
    #[must_use]
    pub const fn tag(self) -> u8 {
        match self {
            Self::Nearest => 0,
            Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }

    #[must_use]
    pub const fn support(self) -> u32 {
        match self {
            Self::Nearest | Self::Bilinear => 1,
            Self::Bicubic => 2,
            Self::Lanczos => 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ScalePixelsConfig {
    pixel_aspect_ratio: FiniteF32,
    opaque_source: Option<Vec<u8>>,
}

impl ScalePixelsConfig {
    pub fn new(pixel_aspect_ratio: f32) -> Result<Self, ScalePixelsConfigError> {
        let value = FiniteF32::new(pixel_aspect_ratio)
            .map_err(|_| ScalePixelsConfigError::NonFiniteAspectRatio)?;
        if !(MIN_PIXEL_ASPECT_RATIO..=MAX_PIXEL_ASPECT_RATIO).contains(&pixel_aspect_ratio) {
            return Err(ScalePixelsConfigError::AspectRatioOutOfRange {
                value: pixel_aspect_ratio,
            });
        }
        Ok(Self {
            pixel_aspect_ratio: value,
            opaque_source: None,
        })
    }

    #[must_use]
    pub fn defaults() -> Self {
        Self::new(1.0).expect("scalepixels default is in range")
    }

    pub fn with_opaque_source(
        pixel_aspect_ratio: f32,
        source: Vec<u8>,
    ) -> Result<Self, ScalePixelsConfigError> {
        let mut config = Self::new(pixel_aspect_ratio)?;
        config.opaque_source = Some(source);
        Ok(config)
    }

    #[must_use]
    pub const fn pixel_aspect_ratio(&self) -> f32 {
        self.pixel_aspect_ratio.get()
    }

    #[must_use]
    pub fn opaque_source(&self) -> Option<&[u8]> {
        self.opaque_source.as_deref()
    }

    #[must_use]
    #[allow(clippy::float_cmp)]
    pub const fn is_identity(&self) -> bool {
        self.pixel_aspect_ratio.get() == 1.0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScalePixelsConfigError {
    NonFiniteAspectRatio,
    AspectRatioOutOfRange { value: f32 },
}

impl fmt::Display for ScalePixelsConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteAspectRatio => formatter.write_str("pixel aspect ratio is non-finite"),
            Self::AspectRatioOutOfRange { value } => {
                write!(formatter, "pixel aspect ratio {value} is outside 0.5..=2.0")
            }
        }
    }
}

impl std::error::Error for ScalePixelsConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScalePixelsParametersV1 {
    config: ScalePixelsConfig,
}

impl ScalePixelsParametersV1 {
    #[must_use]
    pub fn new(config: ScalePixelsConfig) -> Self {
        Self { config }
    }

    #[must_use]
    pub const fn config(&self) -> &ScalePixelsConfig {
        &self.config
    }

    #[must_use]
    pub fn to_bytes(&self) -> [u8; SCALEPIXELS_PARAMETER_BYTES] {
        self.config.pixel_aspect_ratio().to_bits().to_le_bytes()
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ScalePixelsCodecError> {
        if bytes.len() != SCALEPIXELS_PARAMETER_BYTES {
            return Err(ScalePixelsCodecError::InvalidLength {
                expected: SCALEPIXELS_PARAMETER_BYTES,
                actual: bytes.len(),
            });
        }
        let value = f32::from_bits(u32::from_le_bytes(
            bytes.try_into().expect("checked payload length"),
        ));
        let config = ScalePixelsConfig::with_opaque_source(value, bytes.to_vec())
            .map_err(ScalePixelsCodecError::Config)?;
        Ok(Self { config })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScalePixelsCodecError {
    InvalidLength { expected: usize, actual: usize },
    Config(ScalePixelsConfigError),
    UnknownVersion { version: u16, payload: Vec<u8> },
}

impl fmt::Display for ScalePixelsCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength { expected, actual } => {
                write!(
                    formatter,
                    "scalepixels payload has {actual} bytes; expected {expected}"
                )
            }
            Self::Config(error) => write!(formatter, "invalid scalepixels payload: {error}"),
            Self::UnknownVersion { version, payload } => write!(
                formatter,
                "scalepixels parameter version {version} is opaque ({} bytes)",
                payload.len()
            ),
        }
    }
}

impl std::error::Error for ScalePixelsCodecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScalePixelsHistory {
    V1(ScalePixelsParametersV1),
    Opaque { version: u16, payload: Vec<u8> },
}

impl ScalePixelsHistory {
    pub fn decode(version: u16, payload: &[u8]) -> Result<Self, ScalePixelsCodecError> {
        if version == SCALEPIXELS_SCHEMA_VERSION {
            return ScalePixelsParametersV1::from_bytes(payload).map(Self::V1);
        }
        Ok(Self::Opaque {
            version,
            payload: payload.to_vec(),
        })
    }

    #[must_use]
    pub fn payload(&self) -> Vec<u8> {
        match self {
            Self::V1(parameters) => parameters.to_bytes().to_vec(),
            Self::Opaque { payload, .. } => payload.clone(),
        }
    }

    #[must_use]
    pub const fn version(&self) -> u16 {
        match self {
            Self::V1(_) => SCALEPIXELS_SCHEMA_VERSION,
            Self::Opaque { version, .. } => *version,
        }
    }
}
