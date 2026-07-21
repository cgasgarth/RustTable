use std::fmt;

use crate::DispatchError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TilingError {
    ZeroAlignment,
    ZeroBudget,
    ReserveExceedsBudget,
    ZeroBytesPerPixel,
    RoiOutOfBounds,
    EmptyRoi,
    ZeroTileDimension,
    InvalidTileBounds,
    ZeroCandidateLimit,
    InvalidResourceAlignment,
    GenerationMismatch,
    ArithmeticOverflow,
    NoLegalCandidate { required: u64, available: u64 },
    Coverage(CoverageError),
    Dispatch(DispatchError),
    Residency(ResidencyError),
}

impl fmt::Display for TilingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroAlignment => formatter.write_str("tile alignment must be nonzero"),
            Self::ZeroBudget => formatter.write_str("GPU tile budgets must be nonzero"),
            Self::ReserveExceedsBudget => formatter.write_str("GPU tile reserve exceeds budget"),
            Self::ZeroBytesPerPixel => formatter.write_str("bytes per pixel must be nonzero"),
            Self::RoiOutOfBounds => formatter.write_str("GPU tile ROI is outside the image"),
            Self::EmptyRoi => formatter.write_str("GPU tile ROI must be nonempty"),
            Self::ZeroTileDimension => formatter.write_str("tile dimensions must be nonzero"),
            Self::InvalidTileBounds => formatter.write_str("tile bounds are inconsistent"),
            Self::ZeroCandidateLimit => formatter.write_str("candidate limit must be nonzero"),
            Self::InvalidResourceAlignment => {
                formatter.write_str("GPU resource alignment must be a nonzero power of two")
            }
            Self::GenerationMismatch => formatter.write_str("GPU tile generation mismatch"),
            Self::ArithmeticOverflow => formatter.write_str("GPU tile arithmetic overflowed"),
            Self::NoLegalCandidate {
                required,
                available,
            } => {
                write!(
                    formatter,
                    "no legal GPU tile candidate: required {required}, available {available}"
                )
            }
            Self::Coverage(error) => error.fmt(formatter),
            Self::Dispatch(error) => error.fmt(formatter),
            Self::Residency(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for TilingError {}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageError {
    OutsideRoi,
    Overlap,
    Gap,
    ArithmeticOverflow,
}

impl fmt::Display for CoverageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::OutsideRoi => "output tile is outside requested ROI",
            Self::Overlap => "output tile interiors overlap",
            Self::Gap => "output tile interiors do not cover the requested ROI",
            Self::ArithmeticOverflow => "coverage arithmetic overflowed",
        })
    }
}

impl std::error::Error for CoverageError {}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResidencyError {
    ZeroBytes,
    EmptyRoi,
    InvalidLifetime,
    InvalidHostBoundary,
    DuplicateHostBoundary,
    GenerationMismatch,
    ArithmeticOverflow,
    BudgetExceeded { required: u64, budget: u64 },
    Transfer(crate::transfer::TransferError),
}

impl fmt::Display for ResidencyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroBytes => formatter.write_str("resident intermediate bytes must be nonzero"),
            Self::EmptyRoi => formatter.write_str("resident intermediate ROI must be nonempty"),
            Self::InvalidLifetime => formatter.write_str("resident lifetime must be nonempty"),
            Self::InvalidHostBoundary => formatter.write_str("host boundary node range is invalid"),
            Self::DuplicateHostBoundary => formatter.write_str("host boundary is duplicated"),
            Self::GenerationMismatch => {
                formatter.write_str("resident resource generation mismatch")
            }
            Self::ArithmeticOverflow => formatter.write_str("residency arithmetic overflowed"),
            Self::BudgetExceeded { required, budget } => {
                write!(
                    formatter,
                    "resident bytes {required} exceed budget {budget}"
                )
            }
            Self::Transfer(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ResidencyError {}

impl From<crate::transfer::TransferError> for ResidencyError {
    fn from(error: crate::transfer::TransferError) -> Self {
        Self::Transfer(error)
    }
}
