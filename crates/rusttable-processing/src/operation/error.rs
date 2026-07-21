use std::fmt;

use rusttable_core::{FiniteF64, Operation};

use crate::{FiniteF32, ScalarNarrowingError};

use super::OperationCompileError;

pub(crate) fn compile_opacity(operation: &Operation) -> Result<FiniteF32, OperationCompileError> {
    match FiniteF32::try_from(
        FiniteF64::new(operation.opacity().get()).expect("core opacity is finite"),
    ) {
        Ok(value) => Ok(value),
        Err(ScalarNarrowingError::Underflow) => {
            Err(OperationCompileError::OpacityNarrowingUnderflow {
                operation_id: operation.id(),
            })
        }
        Err(ScalarNarrowingError::Overflow) => unreachable!("checked opacity cannot overflow f32"),
    }
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
            Self::OpacityNarrowingUnderflow { operation_id } => write!(
                formatter,
                "operation {operation_id} has opacity that underflows f32"
            ),
            Self::NegativeParameter {
                operation_id,
                key,
                parameter,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has negative parameter {parameter}"
            ),
            Self::InvalidParameters {
                operation_id,
                key,
                reason,
            } => write!(
                formatter,
                "operation {operation_id} with key {key} has invalid parameters: {reason}"
            ),
        }
    }
}

impl std::error::Error for OperationCompileError {}
