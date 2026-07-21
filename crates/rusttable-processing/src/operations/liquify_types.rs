use crate::LinearRgb;
use crate::operations::common::OperationExecutionError;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquifyExecution {
    pub(super) pixels: Vec<LinearRgb>,
    pub(super) identity: [u8; 32],
}

impl LiquifyExecution {
    pub fn pixels(&self) -> &[LinearRgb] {
        &self.pixels
    }

    pub const fn identity(&self) -> [u8; 32] {
        self.identity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiquifyValidationError {
    NonFiniteCoordinate,
    CoordinateOutOfRange,
    UnknownWarpType(u8),
    UnknownNodeType(u8),
    UnknownPathType(u8),
    UnknownStatus(u8),
    InvalidFalloff,
    ZeroRadius,
    TooManyNodes(usize),
    PathMustStartWithMove,
    InvalidBezierTopology,
    InvalidLinks,
    UnknownFlags,
}

impl fmt::Display for LiquifyValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "liquify validation failed: {self:?}")
    }
}

impl std::error::Error for LiquifyValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiquifyCodecError {
    InvalidHeader,
    InvalidHex,
    InvalidLength { actual: usize },
    Oversized { actual: usize, maximum: usize },
    UnsupportedVersion(u16),
    Validation(LiquifyValidationError),
}

impl fmt::Display for LiquifyCodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "liquify codec failed: {self:?}")
    }
}

impl std::error::Error for LiquifyCodecError {}

impl From<LiquifyValidationError> for LiquifyCodecError {
    fn from(error: LiquifyValidationError) -> Self {
        Self::Validation(error)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiquifyExecutionError {
    Validation(LiquifyValidationError),
    Codec(LiquifyCodecError),
    InvalidShape { expected: usize, actual: usize },
    NonFiniteMask,
    FieldTooLarge(usize),
    NonFiniteField,
    UnsupportedOpaquePayload(u16),
    NonConvergentInverse { x: f32, y: f32 },
    ArithmeticOverflow,
    Cancelled,
    SampleNonFinite,
}

impl fmt::Display for LiquifyExecutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "liquify execution failed: {self:?}")
    }
}

impl std::error::Error for LiquifyExecutionError {}

impl From<LiquifyValidationError> for LiquifyExecutionError {
    fn from(error: LiquifyValidationError) -> Self {
        Self::Validation(error)
    }
}

impl From<LiquifyCodecError> for LiquifyExecutionError {
    fn from(error: LiquifyCodecError) -> Self {
        Self::Codec(error)
    }
}

impl From<OperationExecutionError> for LiquifyExecutionError {
    fn from(_: OperationExecutionError) -> Self {
        Self::SampleNonFinite
    }
}
