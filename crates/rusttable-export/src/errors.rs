use std::fmt;

use rusttable_core::RenderSizeError;
use rusttable_core::template::EvaluationError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportValidationError {
    MissingPhotoId,
    MissingEditRevision,
    MissingDependencySnapshot,
    MissingAssetDependency,
    InvalidSize(RenderSizeError),
    UnspecifiedOutputProfile,
    MissingOutputProfileReference,
    EncodingProfileMismatch,
    AlphaNotRepresentable,
    InvalidOpaqueIdentifier { field: &'static str },
    EmptyEncoderParameter,
    EmptyDestinationParameter,
}

impl fmt::Display for ExportValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::MissingPhotoId => "export request is missing a photo ID",
            Self::MissingEditRevision => "export request is missing an exact edit revision",
            Self::MissingDependencySnapshot => "export request is missing the dependency snapshot",
            Self::MissingAssetDependency => {
                "export request is missing its primary asset dependency"
            }
            Self::InvalidSize(error) => return error.fmt(formatter),
            Self::UnspecifiedOutputProfile => "export output profile is unspecified",
            Self::MissingOutputProfileReference => {
                "external output profile has no opaque reference"
            }
            Self::EncodingProfileMismatch => "pixel encoding and output profile disagree",
            Self::AlphaNotRepresentable => {
                "required alpha is not representable by the pixel encoding"
            }
            Self::InvalidOpaqueIdentifier { field } => {
                return write!(formatter, "{field} must be a non-empty opaque identifier");
            }
            Self::EmptyEncoderParameter => "encoder parameter names must not be empty",
            Self::EmptyDestinationParameter => "destination parameter names must not be empty",
        })
    }
}

impl std::error::Error for ExportValidationError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExportContractError {
    Evaluation(EvaluationError),
    Validation(ExportValidationError),
}

impl fmt::Display for ExportContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Evaluation(error) => write!(formatter, "export request template failed: {error}"),
            Self::Validation(error) => write!(formatter, "export request is invalid: {error}"),
        }
    }
}

impl std::error::Error for ExportContractError {}
