use std::fmt;

use rusttable_core::{Operation, OperationId, OperationKey, ParameterName, ParameterValue};

use crate::{FiniteF32, ScalarNarrowingError};

const EXPOSURE_KEY: &str = "rusttable.exposure";
const EXPOSURE_PARAMETER: &str = "stops";
const LINEAR_OFFSET_KEY: &str = "rusttable.linear_offset";
const LINEAR_OFFSET_PARAMETER: &str = "value";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessingOperation {
    operation_id: OperationId,
    enabled: bool,
    kind: ProcessingOperationKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProcessingOperationKind {
    Exposure { stops: FiniteF32 },
    LinearOffset { value: FiniteF32 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationCompileError {
    UnsupportedOperationKey {
        operation_id: OperationId,
        key: OperationKey,
    },
    MissingParameter {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    UnexpectedParameter {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    WrongParameterType {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    ScalarNarrowingOverflow {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
    ScalarNarrowingUnderflow {
        operation_id: OperationId,
        key: OperationKey,
        parameter: ParameterName,
    },
}

impl ProcessingOperation {
    /// Compiles one validated core operation into closed processing data.
    ///
    /// # Errors
    ///
    /// Returns a typed [`OperationCompileError`] when the operation key or its
    /// exact schema is not supported by the processing boundary.
    pub fn compile(operation: &Operation) -> Result<Self, OperationCompileError> {
        match operation.key().as_str() {
            EXPOSURE_KEY => compile_scalar(operation, EXPOSURE_PARAMETER, |stops| {
                ProcessingOperationKind::Exposure { stops }
            }),
            LINEAR_OFFSET_KEY => compile_scalar(operation, LINEAR_OFFSET_PARAMETER, |value| {
                ProcessingOperationKind::LinearOffset { value }
            }),
            _ => Err(OperationCompileError::UnsupportedOperationKey {
                operation_id: operation.id(),
                key: operation.key().clone(),
            }),
        }
    }

    #[must_use]
    pub const fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn kind(&self) -> &ProcessingOperationKind {
        &self.kind
    }
}

fn compile_scalar<F>(
    operation: &Operation,
    required_name: &str,
    build: F,
) -> Result<ProcessingOperation, OperationCompileError>
where
    F: FnOnce(FiniteF32) -> ProcessingOperationKind,
{
    let required = ParameterName::new(required_name).expect("processing schema names are valid");
    if operation.parameter(&required).is_none() {
        return Err(OperationCompileError::MissingParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: required,
        });
    }
    if let Some((unexpected, _)) = operation
        .parameters()
        .find(|(name, _)| name.as_str() != required_name)
    {
        return Err(OperationCompileError::UnexpectedParameter {
            operation_id: operation.id(),
            key: operation.key().clone(),
            parameter: unexpected.clone(),
        });
    }
    let value = match operation.parameter(&required) {
        Some(ParameterValue::Scalar(value)) => *value,
        Some(_) => {
            return Err(OperationCompileError::WrongParameterType {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
        None => unreachable!("required parameter was checked above"),
    };
    let value = match FiniteF32::try_from(value) {
        Ok(value) => value,
        Err(ScalarNarrowingError::Overflow) => {
            return Err(OperationCompileError::ScalarNarrowingOverflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
        Err(ScalarNarrowingError::Underflow) => {
            return Err(OperationCompileError::ScalarNarrowingUnderflow {
                operation_id: operation.id(),
                key: operation.key().clone(),
                parameter: required,
            });
        }
    };
    Ok(ProcessingOperation {
        operation_id: operation.id(),
        enabled: operation.is_enabled(),
        kind: build(value),
    })
}

impl fmt::Display for OperationCompileError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedOperationKey { operation_id, key } => {
                write!(
                    formatter,
                    "operation {operation_id} has unsupported key {key}"
                )
            }
            Self::MissingParameter {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} is missing parameter {parameter}"
            ),
            Self::UnexpectedParameter {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has unexpected parameter {parameter}"
            ),
            Self::WrongParameterType {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has wrong type for parameter {parameter}"
            ),
            Self::ScalarNarrowingOverflow {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} overflows f32 parameter {parameter}"
            ),
            Self::ScalarNarrowingUnderflow {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} underflows f32 parameter {parameter}"
            ),
        }
    }
}

impl std::error::Error for OperationCompileError {}
