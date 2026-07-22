use std::fmt;

use super::RawContainerKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawCapabilityKind {
    Container,
    Camera,
    Compression,
    Layout,
    SampleType,
    ImageIndex,
    ManifestDrift,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawCapabilityError {
    pub missing: RawCapabilityKind,
    pub container: Option<RawContainerKind>,
    pub maker: String,
    pub model: String,
    pub mode: String,
    pub detail: String,
    pub evidence: Box<super::RawCapabilityEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawSourceError {
    Empty,
    TooLarge { actual: u64, limit: u64 },
    LengthConversion,
    AllocationFailure,
    Read { offset: u64, requested: usize },
    Changed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawFrameValidationError {
    ZeroDimensions,
    EmptyArea,
    AreaOutOfBounds,
    ArithmeticOverflow,
    DimensionLimit { width: u32, height: u32 },
    PixelLimit { actual: u64, limit: u64 },
    SampleLimit { actual: u64, limit: u64 },
    DecodedByteLimit { actual: u64, limit: u64 },
    PlaneCount,
    PlaneLayout,
    PlaneSize { expected: u64, actual: u64 },
    InvalidCfa,
    InvalidCfaPhase,
    InvalidLevels,
    InvalidMatrix,
    InvalidPreviewRange,
    BitDepth,
    MetadataLimit,
    MetadataByteLimit,
    NonFiniteMetadata,
    UnsafeCameraIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawDecodeError {
    Cancelled,
    Source(RawSourceError),
    UnsupportedSignature {
        signature: Vec<u8>,
    },
    Malformed {
        container: Option<RawContainerKind>,
        message: String,
    },
    Capability(RawCapabilityError),
    InvalidFrame(RawFrameValidationError),
    Backend {
        container: RawContainerKind,
        message: String,
    },
}

impl fmt::Display for RawDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("RAW decode cancelled"),
            Self::Source(error) => write!(formatter, "RAW source failed: {error:?}"),
            Self::UnsupportedSignature { .. } => {
                formatter.write_str("RAW signature is unsupported")
            }
            Self::Malformed { container, message } => {
                write!(formatter, "malformed RAW {container:?}: {message}")
            }
            Self::Capability(error) => write!(
                formatter,
                "RAW capability {:?} is unavailable for {} {}: {}",
                error.missing, error.maker, error.model, error.detail
            ),
            Self::InvalidFrame(error) => write!(formatter, "invalid RAW frame: {error:?}"),
            Self::Backend { container, message } => {
                write!(formatter, "{container:?} backend failed: {message}")
            }
        }
    }
}

impl std::error::Error for RawDecodeError {}
