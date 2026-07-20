use std::fmt::{Display, Formatter};

#[derive(Debug, PartialEq, Eq)]
pub enum ScanError {
    Io {
        path: String,
        message: String,
    },
    InvalidOverrides {
        message: String,
    },
    InvalidManifest {
        message: String,
    },
    MissingSurface {
        path: String,
    },
    OperationExtraction {
        operation: String,
        message: String,
    },
    OperationValidation {
        operation: String,
        message: String,
    },
    UnknownOpenclKernel {
        operation: String,
        reference: String,
    },
    UnknownOpenclProgram {
        operation: String,
        reference: String,
    },
    ReferenceIdentityMismatch {
        expected: String,
        actual: String,
    },
    Serialization {
        message: String,
    },
}

impl Display for ScanError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, message } => write!(formatter, "I/O error at {path}: {message}"),
            Self::InvalidOverrides { message } => write!(formatter, "invalid overrides: {message}"),
            Self::InvalidManifest { message } => write!(formatter, "invalid manifest: {message}"),
            Self::MissingSurface { path } => write!(formatter, "missing reference surface: {path}"),
            Self::OperationExtraction { operation, message } => {
                write!(
                    formatter,
                    "operation extraction failed for {operation}: {message}"
                )
            }
            Self::OperationValidation { operation, message } => {
                write!(
                    formatter,
                    "operation validation failed for {operation}: {message}"
                )
            }
            Self::UnknownOpenclKernel {
                operation,
                reference,
            } => write!(
                formatter,
                "unknown OpenCL kernel for {operation}: {reference}"
            ),
            Self::UnknownOpenclProgram {
                operation,
                reference,
            } => write!(
                formatter,
                "unknown OpenCL program for {operation}: {reference}"
            ),
            Self::ReferenceIdentityMismatch { expected, actual } => {
                write!(
                    formatter,
                    "reference identity mismatch: expected {expected}, got {actual}"
                )
            }
            Self::Serialization { message } => {
                write!(formatter, "manifest serialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for ScanError {}
