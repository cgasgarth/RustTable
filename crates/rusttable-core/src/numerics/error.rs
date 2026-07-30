use std::fmt;

/// Rejection while defining a numerical contract or implementation registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NumericalContractError {
    InvalidIdentifier,
    InvalidImplementationHash,
    InvalidReductionLeafSize,
    ExactToleranceWithBackendDefinedBehavior,
}

impl fmt::Display for NumericalContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidIdentifier => "numerical implementation identifiers are invalid",
            Self::InvalidImplementationHash => {
                "numerical implementation hash must be 64 lowercase hexadecimal characters"
            }
            Self::InvalidReductionLeafSize => {
                "fixed-tree reduction leaf size must be between 1 and 65535"
            }
            Self::ExactToleranceWithBackendDefinedBehavior => {
                "exact tolerance cannot use backend-defined floating-point behavior"
            }
        })
    }
}

impl std::error::Error for NumericalContractError {}

/// Rejection from a shared numerical helper.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NumericalError {
    NonFinite,
    DivisionByZero,
    OutOfRange,
    InvertedBounds,
    InvalidPeriod,
    InvalidReductionPlan,
    DuplicateReductionLeaf,
    MissingReductionLeaf,
    ReductionOverflow,
}

impl fmt::Display for NumericalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NonFinite => "non-finite value is forbidden by the selected policy",
            Self::DivisionByZero => "division by zero is forbidden",
            Self::OutOfRange => "value is outside the conversion range",
            Self::InvertedBounds => "numerical bounds are inverted",
            Self::InvalidPeriod => "circular period must be finite and positive",
            Self::InvalidReductionPlan => "reduction plan does not match its inputs",
            Self::DuplicateReductionLeaf => "reduction contains a duplicate leaf",
            Self::MissingReductionLeaf => "reduction is missing a required leaf",
            Self::ReductionOverflow => "reduction overflowed its declared domain",
        })
    }
}

impl std::error::Error for NumericalError {}
